//! Bounded client-to-owner SSH MCP link pool [ORB-10269, ORB-10727].

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use orbit_common::types::{McpCapability, OrbitError, ToolSessionContext};
use orbit_common::utility::redaction::redact_sensitive_env_text;
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use super::owner_client::{OrbitMcpClient, OwnerClientExpectation, validate_remote_call_context};

const STDERR_LIMIT: u64 = 8 * 1024;

// This connector seam is crate-internal so sibling tests can supply deterministic peers.
pub(super) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy)]
pub(super) struct OwnerLinkLimits {
    pub(super) queue_capacity: usize,
    pub(super) initialize: Duration,
    pub(super) request: Duration,
    pub(super) idle: Duration,
    pub(super) idle_poll: Duration,
    pub(super) close: Duration,
}

impl Default for OwnerLinkLimits {
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

pub(super) trait OwnerClock: Send + Sync + 'static {
    fn now(&self) -> Duration;
}

pub(super) struct MonotonicClock(Instant);

impl Default for MonotonicClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}

impl OwnerClock for MonotonicClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OwnerSpawnSpec {
    pub(super) ssh_alias: String,
    pub(super) owner_machine_id: String,
    pub(super) capability: McpCapability,
    pub(super) schema_digest: String,
}

impl OwnerSpawnSpec {
    pub(super) fn argv(&self) -> Vec<String> {
        vec![
            "ssh".to_string(),
            self.ssh_alias.clone(),
            "orbit".to_string(),
            "mcp".to_string(),
            "serve".to_string(),
            "--owner".to_string(),
            "--capabilities".to_string(),
            self.capability.to_string(),
        ]
    }

    pub(super) fn expectation(&self) -> OwnerClientExpectation {
        OwnerClientExpectation {
            owner_machine_id: self.owner_machine_id.clone(),
            effective_capability: self.capability,
            owner_schema_digest: self.schema_digest.clone(),
        }
    }
}

pub(super) trait OwnerPeer: Send {
    fn is_closed(&self) -> bool;
    fn call<'a>(
        &'a mut self,
        name: &'a str,
        input: Value,
        context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<Value, OrbitError>>;
    fn close<'a>(&'a mut self) -> BoxFuture<'a, ()>;
}

pub(super) trait OwnerPeerFactory: Send + Sync + 'static {
    fn connect<'a>(
        &'a self,
        spec: &'a OwnerSpawnSpec,
        limits: OwnerLinkLimits,
    ) -> BoxFuture<'a, Result<Box<dyn OwnerPeer>, OrbitError>>;
}

#[derive(Default)]
struct SshOwnerPeerFactory;

impl OwnerPeerFactory for SshOwnerPeerFactory {
    fn connect<'a>(
        &'a self,
        spec: &'a OwnerSpawnSpec,
        limits: OwnerLinkLimits,
    ) -> BoxFuture<'a, Result<Box<dyn OwnerPeer>, OrbitError>> {
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
                OrbitError::OwnerUnavailable(format!(
                    "failed to start fixed SSH owner command for alias '{}': {error}",
                    spec.ssh_alias
                ))
            })?;
            let write = child.stdin.take().ok_or_else(|| {
                OrbitError::OwnerUnavailable("SSH owner process has no stdin".to_string())
            })?;
            let read = child.stdout.take().ok_or_else(|| {
                OrbitError::OwnerUnavailable("SSH owner process has no stdout".to_string())
            })?;
            let stderr = child.stderr.take().ok_or_else(|| {
                OrbitError::OwnerUnavailable("SSH owner process has no stderr".to_string())
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
            Ok(Box::new(SshOwnerPeer {
                client,
                child,
                stderr_task,
                request_timeout: limits.request,
                close_timeout: limits.close,
            }) as Box<dyn OwnerPeer>)
        })
    }
}

struct SshOwnerPeer {
    client: OrbitMcpClient,
    child: Child,
    stderr_task: tokio::task::JoinHandle<String>,
    request_timeout: Duration,
    close_timeout: Duration,
}

impl OwnerPeer for SshOwnerPeer {
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
                    OrbitError::OwnerUnavailable(format!("inspect SSH owner process: {error}"))
                })?
                .is_some()
            {
                return Err(OrbitError::OwnerUnavailable(
                    "SSH owner process exited before request handoff".to_string(),
                ));
            }
            self.client
                .call_tool(name, input, context, self.request_timeout)
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

pub(super) enum WorkerMessage {
    Call(CallRequest),
    Shutdown,
}

struct CachedPeer {
    peer: Box<dyn OwnerPeer>,
    last_used: Duration,
}

/// Synchronous [`orbit_mcp::McpHost`] seam backed by one dedicated runtime
/// thread and at most one live peer for each scalar capability.
pub(super) struct OwnerLinkPool {
    pub(super) tx: Option<mpsc::SyncSender<WorkerMessage>>,
    worker: Option<JoinHandle<()>>,
}

impl OwnerLinkPool {
    pub(super) fn ssh(
        ssh_alias: String,
        owner_machine_id: String,
        schema_digests: BTreeMap<McpCapability, String>,
    ) -> Result<Self, OrbitError> {
        Self::with_factory(
            ssh_alias,
            owner_machine_id,
            schema_digests,
            Arc::new(SshOwnerPeerFactory),
            OwnerLinkLimits::default(),
            Arc::new(MonotonicClock::default()),
        )
    }

    pub(super) fn with_factory(
        ssh_alias: String,
        owner_machine_id: String,
        schema_digests: BTreeMap<McpCapability, String>,
        factory: Arc<dyn OwnerPeerFactory>,
        limits: OwnerLinkLimits,
        clock: Arc<dyn OwnerClock>,
    ) -> Result<Self, OrbitError> {
        let (tx, rx) = mpsc::sync_channel(limits.queue_capacity);
        let worker_clock = Arc::clone(&clock);
        let worker = std::thread::Builder::new()
            .name("orbit-owner-link".to_string())
            .spawn(move || {
                run_worker(
                    rx,
                    ssh_alias,
                    owner_machine_id,
                    schema_digests,
                    factory,
                    limits,
                    worker_clock,
                );
            })
            .map_err(|error| {
                OrbitError::OwnerUnavailable(format!("start owner link worker: {error}"))
            })?;
        Ok(Self {
            tx: Some(tx),
            worker: Some(worker),
        })
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
                OrbitError::OwnerUnavailable("owner link pool is shutting down".to_string())
            })?
            .try_send(WorkerMessage::Call(CallRequest {
                capability,
                name: name.to_string(),
                input,
                context,
                response: response_tx,
            }))
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => OrbitError::OwnerUnavailable(
                    "owner link request queue is saturated before handoff".to_string(),
                ),
                mpsc::TrySendError::Disconnected(_) => OrbitError::OwnerUnavailable(
                    "owner link worker is unavailable before handoff".to_string(),
                ),
            })?;
        // Queue admission is the pre-handoff boundary. Once accepted, wait for
        // the worker's bounded connect/request/close operations to return a
        // definitive result instead of inventing a second caller deadline.
        response_rx
            .recv()
            .map_err(|error| OrbitError::OutcomeUnknown {
                mcp_call_id,
                message: format!("owner link worker disconnected after queue handoff: {error}"),
            })?
    }
}

impl Drop for OwnerLinkPool {
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
    owner_machine_id: String,
    schema_digests: BTreeMap<McpCapability, String>,
    factory: Arc<dyn OwnerPeerFactory>,
    limits: OwnerLinkLimits,
    clock: Arc<dyn OwnerClock>,
) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let worker = WorkerRuntime {
        ssh_alias: &ssh_alias,
        owner_machine_id: &owner_machine_id,
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
        }
    }
    runtime.block_on(close_all(&mut peers));
}

struct WorkerRuntime<'a> {
    ssh_alias: &'a str,
    owner_machine_id: &'a str,
    schema_digests: &'a BTreeMap<McpCapability, String>,
    factory: &'a dyn OwnerPeerFactory,
    limits: OwnerLinkLimits,
    clock: &'a dyn OwnerClock,
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
            OrbitError::OwnerNegotiation(format!(
                "no local schema digest exists for capability '{capability}'"
            ))
        })?;
        let spec = OwnerSpawnSpec {
            ssh_alias: worker.ssh_alias.to_string(),
            owner_machine_id: worker.owner_machine_id.to_string(),
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
        .ok_or_else(|| OrbitError::OwnerUnavailable("owner peer disappeared".to_string()))?;
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
                "owner response exceeded the {} ms post-handoff deadline",
                worker.limits.request.as_millis()
            ),
        }),
    };
    cached.last_used = worker.clock.now();
    if matches!(
        result,
        Err(OrbitError::OwnerUnavailable(_)) | Err(OrbitError::OutcomeUnknown { .. })
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
