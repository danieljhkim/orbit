//! Bounded spoke-to-hub SSH MCP link pool [ORB-10269].

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use orbit_common::types::{
    HubKnowledgeAllocationRequestV1, HubKnowledgeAllocationV1, McpCapability, OrbitError,
    SpokeRegistrationRequestV1, SpokeRegistrationResultV1, ToolSessionContext,
};
use orbit_common::utility::redaction::redact_sensitive_env_text;
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use super::hub_client::{HubClientExpectation, OrbitMcpClient, validate_remote_call_context};

const STDERR_LIMIT: u64 = 8 * 1024;

// This connector seam is crate-internal so sibling tests can supply deterministic peers.
pub(super) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy)]
pub(super) struct HubLinkLimits {
    pub(super) queue_capacity: usize,
    pub(super) initialize: Duration,
    pub(super) request: Duration,
    pub(super) idle: Duration,
    pub(super) idle_poll: Duration,
    pub(super) close: Duration,
}

impl Default for HubLinkLimits {
    fn default() -> Self {
        Self {
            queue_capacity: 8,
            initialize: Duration::from_secs(10),
            request: Duration::from_secs(30),
            idle: Duration::from_secs(60),
            idle_poll: Duration::from_secs(5),
            close: Duration::from_secs(2),
        }
    }
}

pub(super) trait HubClock: Send + Sync + 'static {
    fn now(&self) -> Duration;
}

pub(super) struct MonotonicClock(Instant);

impl Default for MonotonicClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl HubClock for MonotonicClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HubSpawnSpec {
    pub(super) ssh_alias: String,
    pub(super) hub_machine_id: String,
    pub(super) capability: McpCapability,
    pub(super) schema_digest: String,
}

impl HubSpawnSpec {
    pub(super) fn argv(&self) -> Vec<String> {
        vec![
            "ssh".to_string(),
            self.ssh_alias.clone(),
            "orbit".to_string(),
            "mcp".to_string(),
            "serve".to_string(),
            "--hub".to_string(),
            "--capabilities".to_string(),
            self.capability.to_string(),
        ]
    }

    pub(super) fn expectation(&self) -> HubClientExpectation {
        HubClientExpectation {
            hub_machine_id: self.hub_machine_id.clone(),
            effective_capability: self.capability,
            hub_schema_digest: self.schema_digest.clone(),
        }
    }
}

pub(super) trait HubPeer: Send {
    fn is_closed(&self) -> bool;
    fn call<'a>(
        &'a mut self,
        name: &'a str,
        input: Value,
        context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<Value, OrbitError>>;
    fn register_spoke<'a>(
        &'a mut self,
        request: &'a SpokeRegistrationRequestV1,
        context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<SpokeRegistrationResultV1, OrbitError>>;
    fn allocate_knowledge_id<'a>(
        &'a mut self,
        _request: &'a HubKnowledgeAllocationRequestV1,
        _context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<HubKnowledgeAllocationV1, OrbitError>> {
        Box::pin(async {
            Err(OrbitError::HubNegotiation(
                "hub peer does not implement private knowledge allocation".to_string(),
            ))
        })
    }
    fn close<'a>(&'a mut self) -> BoxFuture<'a, ()>;
}

pub(super) trait HubPeerFactory: Send + Sync + 'static {
    fn connect<'a>(
        &'a self,
        spec: &'a HubSpawnSpec,
        limits: HubLinkLimits,
    ) -> BoxFuture<'a, Result<Box<dyn HubPeer>, OrbitError>>;
}

#[derive(Default)]
struct SshHubPeerFactory;

impl HubPeerFactory for SshHubPeerFactory {
    fn connect<'a>(
        &'a self,
        spec: &'a HubSpawnSpec,
        limits: HubLinkLimits,
    ) -> BoxFuture<'a, Result<Box<dyn HubPeer>, OrbitError>> {
        Box::pin(async move {
            let argv = spec.argv();
            let mut command = Command::new(&argv[0]);
            command.args(&argv[1..]);
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            let mut child = command.spawn().map_err(|error| {
                OrbitError::HubUnavailable(format!(
                    "failed to start fixed SSH hub command for alias '{}': {error}",
                    spec.ssh_alias
                ))
            })?;
            let write = child.stdin.take().ok_or_else(|| {
                OrbitError::HubUnavailable("SSH hub process has no stdin".to_string())
            })?;
            let read = child.stdout.take().ok_or_else(|| {
                OrbitError::HubUnavailable("SSH hub process has no stdout".to_string())
            })?;
            let stderr = child.stderr.take().ok_or_else(|| {
                OrbitError::HubUnavailable("SSH hub process has no stderr".to_string())
            })?;
            let stderr_task = tokio::spawn(async move {
                let mut bytes = Vec::new();
                let _ = stderr.take(STDERR_LIMIT + 1).read_to_end(&mut bytes).await;
                redact_sensitive_env_text(&String::from_utf8_lossy(&bytes))
            });
            let client =
                match OrbitMcpClient::connect(read, write, &spec.expectation(), limits.initialize)
                    .await
                {
                    Ok(client) => client,
                    Err(error) => {
                        let _ = child.start_kill();
                        let _ = tokio::time::timeout(limits.close, child.wait()).await;
                        stderr_task.abort();
                        return Err(error);
                    }
                };
            Ok(Box::new(SshHubPeer {
                client,
                child,
                stderr_task,
                request_timeout: limits.request,
                close_timeout: limits.close,
            }) as Box<dyn HubPeer>)
        })
    }
}

struct SshHubPeer {
    client: OrbitMcpClient,
    child: Child,
    stderr_task: tokio::task::JoinHandle<String>,
    request_timeout: Duration,
    close_timeout: Duration,
}

impl HubPeer for SshHubPeer {
    fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    fn call<'a>(
        &'a mut self,
        name: &'a str,
        input: Value,
        context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<Value, OrbitError>> {
        Box::pin(async move {
            if self
                .child
                .try_wait()
                .map_err(|error| {
                    OrbitError::HubUnavailable(format!("inspect SSH hub process: {error}"))
                })?
                .is_some()
            {
                return Err(OrbitError::HubUnavailable(
                    "SSH hub process exited before request handoff".to_string(),
                ));
            }
            self.client
                .call_tool(name, input, context, self.request_timeout)
                .await
        })
    }

    fn register_spoke<'a>(
        &'a mut self,
        request: &'a SpokeRegistrationRequestV1,
        context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<SpokeRegistrationResultV1, OrbitError>> {
        Box::pin(async move {
            if self
                .child
                .try_wait()
                .map_err(|error| {
                    OrbitError::HubUnavailable(format!("inspect SSH hub process: {error}"))
                })?
                .is_some()
            {
                return Err(OrbitError::HubUnavailable(
                    "SSH hub process exited before registration handoff".to_string(),
                ));
            }
            self.client
                .register_spoke(request, context, self.request_timeout)
                .await
        })
    }

    fn allocate_knowledge_id<'a>(
        &'a mut self,
        request: &'a HubKnowledgeAllocationRequestV1,
        context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<HubKnowledgeAllocationV1, OrbitError>> {
        Box::pin(async move {
            if self
                .child
                .try_wait()
                .map_err(|error| {
                    OrbitError::HubUnavailable(format!("inspect SSH hub process: {error}"))
                })?
                .is_some()
            {
                return Err(OrbitError::HubUnavailable(
                    "SSH hub process exited before allocation handoff".to_string(),
                ));
            }
            self.client
                .allocate_knowledge_id(request, context, self.request_timeout)
                .await
        })
    }

    fn close<'a>(&'a mut self) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let _ = self.client.close(self.close_timeout).await;
            if tokio::time::timeout(self.close_timeout, self.child.wait())
                .await
                .is_err()
            {
                let _ = self.child.start_kill();
                let _ = tokio::time::timeout(self.close_timeout, self.child.wait()).await;
            }
            self.stderr_task.abort();
        })
    }
}

pub(super) struct CallRequest {
    pub(super) capability: McpCapability,
    pub(super) name: String,
    pub(super) input: Value,
    pub(super) context: ToolSessionContext,
    pub(super) response: mpsc::SyncSender<Result<Value, OrbitError>>,
}

pub(super) struct RegistrationRequest {
    capability: McpCapability,
    registration: SpokeRegistrationRequestV1,
    context: ToolSessionContext,
    response: mpsc::SyncSender<Result<SpokeRegistrationResultV1, OrbitError>>,
}

pub(super) struct KnowledgeAllocationRequest {
    capability: McpCapability,
    request: HubKnowledgeAllocationRequestV1,
    context: ToolSessionContext,
    response: mpsc::SyncSender<Result<HubKnowledgeAllocationV1, OrbitError>>,
}

pub(super) enum WorkerMessage {
    Call(CallRequest),
    Register(RegistrationRequest),
    Allocate(KnowledgeAllocationRequest),
    Shutdown,
}

struct CachedPeer {
    peer: Box<dyn HubPeer>,
    last_used: Duration,
}

/// Synchronous [`orbit_mcp::McpHost`] seam backed by one dedicated runtime
/// thread and at most one live peer for each scalar capability.
pub(super) struct HubLinkPool {
    hub_machine_id: String,
    pub(super) tx: Option<mpsc::SyncSender<WorkerMessage>>,
    worker: Option<JoinHandle<()>>,
}

impl HubLinkPool {
    pub(super) fn ssh(
        ssh_alias: String,
        hub_machine_id: String,
        schema_digests: BTreeMap<McpCapability, String>,
    ) -> Result<Self, OrbitError> {
        Self::with_factory(
            ssh_alias,
            hub_machine_id,
            schema_digests,
            Arc::new(SshHubPeerFactory),
            HubLinkLimits::default(),
            Arc::new(MonotonicClock::default()),
        )
    }

    pub(super) fn with_factory(
        ssh_alias: String,
        hub_machine_id: String,
        schema_digests: BTreeMap<McpCapability, String>,
        factory: Arc<dyn HubPeerFactory>,
        limits: HubLinkLimits,
        clock: Arc<dyn HubClock>,
    ) -> Result<Self, OrbitError> {
        let (tx, rx) = mpsc::sync_channel(limits.queue_capacity);
        let worker_clock = Arc::clone(&clock);
        let worker_hub_machine_id = hub_machine_id.clone();
        let worker = std::thread::Builder::new()
            .name("orbit-hub-link".to_string())
            .spawn(move || {
                run_worker(
                    rx,
                    ssh_alias,
                    worker_hub_machine_id,
                    schema_digests,
                    factory,
                    limits,
                    worker_clock,
                );
            })
            .map_err(|error| {
                OrbitError::HubUnavailable(format!("start hub link worker: {error}"))
            })?;
        Ok(Self {
            hub_machine_id,
            tx: Some(tx),
            worker: Some(worker),
        })
    }

    pub(super) fn hub_machine_id(&self) -> &str {
        &self.hub_machine_id
    }

    pub(super) fn call(
        &self,
        capability: McpCapability,
        name: &str,
        input: Value,
        context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        validate_remote_call_context(&context, capability)?;
        let mcp_call_id = context
            .mcp_call_id
            .clone()
            .ok_or_else(|| OrbitError::InvalidInput("remote call ID is missing".to_string()))?;
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.tx
            .as_ref()
            .ok_or_else(|| {
                OrbitError::HubUnavailable("hub link pool is shutting down".to_string())
            })?
            .try_send(WorkerMessage::Call(CallRequest {
                capability,
                name: name.to_string(),
                input,
                context,
                response: response_tx,
            }))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => OrbitError::HubUnavailable(
                    "hub link request queue is saturated before handoff".to_string(),
                ),
                mpsc::TrySendError::Disconnected(_) => OrbitError::HubUnavailable(
                    "hub link worker is unavailable before handoff".to_string(),
                ),
            })?;
        // Queue admission is the pre-handoff boundary. Once accepted, wait for
        // the worker's bounded connect/request/close operations to return a
        // definitive result instead of inventing a second caller deadline.
        response_rx
            .recv()
            .map_err(|error| OrbitError::OutcomeUnknown {
                mcp_call_id,
                message: format!("hub link worker disconnected after queue handoff: {error}"),
            })?
    }

    pub(super) fn register_spoke(
        &self,
        capability: McpCapability,
        registration: SpokeRegistrationRequestV1,
        context: ToolSessionContext,
    ) -> Result<SpokeRegistrationResultV1, OrbitError> {
        registration.validate()?;
        validate_remote_call_context(&context, capability)?;
        if context.workspace.is_some() || context.workspace_id.is_some() {
            return Err(OrbitError::InvalidInput(
                "private spoke registration is global and must not carry a workspace selector"
                    .to_string(),
            ));
        }
        if context.caller_machine_id.as_deref() != Some(&registration.identity.machine_id)
            || context.caller_host_id.as_deref() != Some(&registration.identity.host_id)
        {
            return Err(OrbitError::InvalidInput(
                "private registration identity must exactly match the trusted caller context"
                    .to_string(),
            ));
        }
        let mcp_call_id = context
            .mcp_call_id
            .clone()
            .ok_or_else(|| OrbitError::InvalidInput("remote call ID is missing".to_string()))?;
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.tx
            .as_ref()
            .ok_or_else(|| {
                OrbitError::HubUnavailable("hub link pool is shutting down".to_string())
            })?
            .try_send(WorkerMessage::Register(RegistrationRequest {
                capability,
                registration,
                context,
                response: response_tx,
            }))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => OrbitError::HubUnavailable(
                    "hub link request queue is saturated before handoff".to_string(),
                ),
                mpsc::TrySendError::Disconnected(_) => OrbitError::HubUnavailable(
                    "hub link worker is unavailable before handoff".to_string(),
                ),
            })?;
        response_rx
            .recv()
            .map_err(|error| OrbitError::OutcomeUnknown {
                mcp_call_id,
                message: format!("hub link worker disconnected after queue handoff: {error}"),
            })?
    }

    pub(super) fn allocate_knowledge_id(
        &self,
        capability: McpCapability,
        request: HubKnowledgeAllocationRequestV1,
        context: ToolSessionContext,
    ) -> Result<HubKnowledgeAllocationV1, OrbitError> {
        request.validate()?;
        validate_remote_call_context(&context, capability)?;
        if context.workspace_id.as_deref() != Some(request.workspace_id.as_str()) {
            return Err(OrbitError::InvalidInput(
                "private hub knowledge allocation workspace must exactly match the trusted remote context"
                    .to_string(),
            ));
        }
        let mcp_call_id = context
            .mcp_call_id
            .clone()
            .ok_or_else(|| OrbitError::InvalidInput("remote call ID is missing".to_string()))?;
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        self.tx
            .as_ref()
            .ok_or_else(|| {
                OrbitError::HubUnavailable("hub link pool is shutting down".to_string())
            })?
            .try_send(WorkerMessage::Allocate(KnowledgeAllocationRequest {
                capability,
                request,
                context,
                response: response_tx,
            }))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => OrbitError::HubUnavailable(
                    "hub link request queue is saturated before handoff".to_string(),
                ),
                mpsc::TrySendError::Disconnected(_) => OrbitError::HubUnavailable(
                    "hub link worker is unavailable before handoff".to_string(),
                ),
            })?;
        response_rx
            .recv()
            .map_err(|error| OrbitError::OutcomeUnknown {
                mcp_call_id,
                message: format!("hub link worker disconnected after queue handoff: {error}"),
            })?
    }
}

impl Drop for HubLinkPool {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.try_send(WorkerMessage::Shutdown);
            // A full queue may reject Shutdown. Dropping the final sender
            // still guarantees the worker exits after the bounded backlog.
            drop(tx);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    rx: mpsc::Receiver<WorkerMessage>,
    ssh_alias: String,
    hub_machine_id: String,
    schema_digests: BTreeMap<McpCapability, String>,
    factory: Arc<dyn HubPeerFactory>,
    limits: HubLinkLimits,
    clock: Arc<dyn HubClock>,
) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let worker = WorkerRuntime {
        ssh_alias: &ssh_alias,
        hub_machine_id: &hub_machine_id,
        schema_digests: &schema_digests,
        factory: factory.as_ref(),
        limits,
        clock: clock.as_ref(),
    };
    let mut peers: BTreeMap<McpCapability, CachedPeer> = BTreeMap::new();
    loop {
        let message = match rx.recv_timeout(worker.limits.idle_poll) {
            Ok(message) => message,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                runtime.block_on(reap_idle(
                    &mut peers,
                    worker.clock.now(),
                    worker.limits.idle,
                ));
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match message {
            WorkerMessage::Shutdown => break,
            WorkerMessage::Call(request) => {
                let result = runtime.block_on(process_call(&mut peers, &worker, &request));
                let _ = request.response.send(result);
            }
            WorkerMessage::Register(request) => {
                let result = runtime.block_on(process_registration(&mut peers, &worker, &request));
                let _ = request.response.send(result);
            }
            WorkerMessage::Allocate(request) => {
                let result = runtime.block_on(process_allocation(&mut peers, &worker, &request));
                let _ = request.response.send(result);
            }
        }
    }
    runtime.block_on(close_all(&mut peers));
}

struct WorkerRuntime<'a> {
    ssh_alias: &'a str,
    hub_machine_id: &'a str,
    schema_digests: &'a BTreeMap<McpCapability, String>,
    factory: &'a dyn HubPeerFactory,
    limits: HubLinkLimits,
    clock: &'a dyn HubClock,
}

async fn ensure_peer(
    peers: &mut BTreeMap<McpCapability, CachedPeer>,
    worker: &WorkerRuntime<'_>,
    capability: McpCapability,
) -> Result<(), OrbitError> {
    if peers
        .get(&capability)
        .is_some_and(|cached| cached.peer.is_closed())
        && let Some(mut stale) = peers.remove(&capability)
    {
        stale.peer.close().await;
    }
    if let std::collections::btree_map::Entry::Vacant(entry) = peers.entry(capability) {
        let schema_digest = worker.schema_digests.get(&capability).ok_or_else(|| {
            OrbitError::HubNegotiation(format!(
                "no local schema digest exists for capability '{capability}'"
            ))
        })?;
        let spec = HubSpawnSpec {
            ssh_alias: worker.ssh_alias.to_string(),
            hub_machine_id: worker.hub_machine_id.to_string(),
            capability,
            schema_digest: schema_digest.clone(),
        };
        let peer = worker.factory.connect(&spec, worker.limits).await?;
        entry.insert(CachedPeer {
            peer,
            last_used: worker.clock.now(),
        });
    }
    Ok(())
}

async fn process_registration(
    peers: &mut BTreeMap<McpCapability, CachedPeer>,
    worker: &WorkerRuntime<'_>,
    request: &RegistrationRequest,
) -> Result<SpokeRegistrationResultV1, OrbitError> {
    let now = worker.clock.now();
    reap_idle(peers, now, worker.limits.idle).await;
    ensure_peer(peers, worker, request.capability).await?;
    let cached = peers
        .get_mut(&request.capability)
        .ok_or_else(|| OrbitError::HubUnavailable("hub peer disappeared".to_string()))?;
    let result = match tokio::time::timeout(
        worker.limits.request,
        cached
            .peer
            .register_spoke(&request.registration, &request.context),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(OrbitError::OutcomeUnknown {
            mcp_call_id: request.context.mcp_call_id.clone().unwrap_or_default(),
            message: format!(
                "hub registration response exceeded the {} ms post-handoff deadline",
                worker.limits.request.as_millis()
            ),
        }),
    };
    cached.last_used = worker.clock.now();
    if matches!(
        result,
        Err(OrbitError::HubUnavailable(_)) | Err(OrbitError::OutcomeUnknown { .. })
    ) && let Some(mut failed) = peers.remove(&request.capability)
    {
        failed.peer.close().await;
    }
    result
}

async fn process_allocation(
    peers: &mut BTreeMap<McpCapability, CachedPeer>,
    worker: &WorkerRuntime<'_>,
    request: &KnowledgeAllocationRequest,
) -> Result<HubKnowledgeAllocationV1, OrbitError> {
    let now = worker.clock.now();
    reap_idle(peers, now, worker.limits.idle).await;
    ensure_peer(peers, worker, request.capability).await?;
    let cached = peers
        .get_mut(&request.capability)
        .ok_or_else(|| OrbitError::HubUnavailable("hub peer disappeared".to_string()))?;
    let result = match tokio::time::timeout(
        worker.limits.request,
        cached
            .peer
            .allocate_knowledge_id(&request.request, &request.context),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(OrbitError::OutcomeUnknown {
            mcp_call_id: request.context.mcp_call_id.clone().unwrap_or_default(),
            message: format!(
                "hub allocation response exceeded the {} ms post-handoff deadline",
                worker.limits.request.as_millis()
            ),
        }),
    };
    cached.last_used = worker.clock.now();
    if matches!(
        result,
        Err(OrbitError::HubUnavailable(_)) | Err(OrbitError::OutcomeUnknown { .. })
    ) && let Some(mut failed) = peers.remove(&request.capability)
    {
        failed.peer.close().await;
    }
    result
}

async fn process_call(
    peers: &mut BTreeMap<McpCapability, CachedPeer>,
    worker: &WorkerRuntime<'_>,
    request: &CallRequest,
) -> Result<Value, OrbitError> {
    let now = worker.clock.now();
    reap_idle(peers, now, worker.limits.idle).await;
    ensure_peer(peers, worker, request.capability).await?;
    let cached = peers
        .get_mut(&request.capability)
        .ok_or_else(|| OrbitError::HubUnavailable("hub peer disappeared".to_string()))?;
    let result = match tokio::time::timeout(
        worker.limits.request,
        cached
            .peer
            .call(&request.name, request.input.clone(), &request.context),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(OrbitError::OutcomeUnknown {
            mcp_call_id: request.context.mcp_call_id.clone().unwrap_or_default(),
            message: format!(
                "hub response exceeded the {} ms post-handoff deadline",
                worker.limits.request.as_millis()
            ),
        }),
    };
    cached.last_used = worker.clock.now();
    if matches!(
        result,
        Err(OrbitError::HubUnavailable(_)) | Err(OrbitError::OutcomeUnknown { .. })
    ) && let Some(mut failed) = peers.remove(&request.capability)
    {
        failed.peer.close().await;
    }
    result
}

async fn reap_idle(peers: &mut BTreeMap<McpCapability, CachedPeer>, now: Duration, idle: Duration) {
    let stale = peers
        .iter()
        .filter_map(|(capability, cached)| {
            (now.saturating_sub(cached.last_used) >= idle || cached.peer.is_closed())
                .then_some(*capability)
        })
        .collect::<Vec<_>>();
    for capability in stale {
        if let Some(mut cached) = peers.remove(&capability) {
            cached.peer.close().await;
        }
    }
}

async fn close_all(peers: &mut BTreeMap<McpCapability, CachedPeer>) {
    for (_, mut cached) in std::mem::take(peers) {
        cached.peer.close().await;
    }
}
