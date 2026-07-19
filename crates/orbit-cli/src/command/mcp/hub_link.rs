//! Bounded spoke-to-hub SSH MCP link pool [ORB-10269].

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use orbit_common::types::{McpCapability, OrbitError, ToolSessionContext};
use orbit_common::utility::redaction::redact_sensitive_env_text;
use orbit_mcp::{HubClientExpectation, OrbitMcpClient};
use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

const STDERR_LIMIT: u64 = 8 * 1024;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy)]
struct HubLinkLimits {
    queue_capacity: usize,
    submission: Duration,
    initialize: Duration,
    request: Duration,
    idle: Duration,
    idle_poll: Duration,
    close: Duration,
    caller_wait: Duration,
}

impl Default for HubLinkLimits {
    fn default() -> Self {
        Self {
            queue_capacity: 8,
            submission: Duration::from_secs(2),
            initialize: Duration::from_secs(10),
            request: Duration::from_secs(30),
            idle: Duration::from_secs(60),
            idle_poll: Duration::from_secs(5),
            close: Duration::from_secs(2),
            caller_wait: Duration::from_secs(45),
        }
    }
}

trait HubClock: Send + Sync + 'static {
    fn now(&self) -> Duration;
}

struct MonotonicClock(Instant);

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
    fn argv(&self) -> Vec<String> {
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

    fn expectation(&self) -> HubClientExpectation {
        HubClientExpectation {
            hub_machine_id: self.hub_machine_id.clone(),
            effective_capability: self.capability,
            hub_schema_digest: self.schema_digest.clone(),
        }
    }
}

trait HubPeer: Send {
    fn is_closed(&self) -> bool;
    fn call<'a>(
        &'a mut self,
        name: &'a str,
        input: Value,
        context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<Value, OrbitError>>;
    fn close<'a>(&'a mut self) -> BoxFuture<'a, ()>;
}

trait HubPeerFactory: Send + Sync + 'static {
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

struct CallRequest {
    capability: McpCapability,
    name: String,
    input: Value,
    context: ToolSessionContext,
    queued_at: Duration,
    response: mpsc::SyncSender<Result<Value, OrbitError>>,
}

enum WorkerMessage {
    Call(CallRequest),
    Shutdown,
}

struct CachedPeer {
    peer: Box<dyn HubPeer>,
    last_used: Duration,
}

/// Synchronous [`orbit_mcp::McpHost`] seam backed by one dedicated runtime
/// thread and at most one live peer for each scalar capability.
pub(super) struct HubLinkPool {
    tx: Option<mpsc::SyncSender<WorkerMessage>>,
    worker: Option<JoinHandle<()>>,
    limits: HubLinkLimits,
    clock: Arc<dyn HubClock>,
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

    fn with_factory(
        ssh_alias: String,
        hub_machine_id: String,
        schema_digests: BTreeMap<McpCapability, String>,
        factory: Arc<dyn HubPeerFactory>,
        limits: HubLinkLimits,
        clock: Arc<dyn HubClock>,
    ) -> Result<Self, OrbitError> {
        let (tx, rx) = mpsc::sync_channel(limits.queue_capacity);
        let worker_clock = Arc::clone(&clock);
        let worker = std::thread::Builder::new()
            .name("orbit-hub-link".to_string())
            .spawn(move || {
                run_worker(
                    rx,
                    ssh_alias,
                    hub_machine_id,
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
            tx: Some(tx),
            worker: Some(worker),
            limits,
            clock,
        })
    }

    pub(super) fn call(
        &self,
        capability: McpCapability,
        name: &str,
        input: Value,
        context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        orbit_mcp::validate_remote_call_context(&context, capability)?;
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
                queued_at: self.clock.now(),
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
            .recv_timeout(self.limits.caller_wait)
            .map_err(|error| OrbitError::OutcomeUnknown {
                mcp_call_id,
                message: format!("hub link worker response deadline elapsed: {error}"),
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
    let mut peers: BTreeMap<McpCapability, CachedPeer> = BTreeMap::new();
    loop {
        let message = match rx.recv_timeout(limits.idle_poll) {
            Ok(message) => message,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                runtime.block_on(reap_idle(&mut peers, clock.now(), limits.idle));
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match message {
            WorkerMessage::Shutdown => break,
            WorkerMessage::Call(request) => {
                let result = runtime.block_on(process_call(
                    &mut peers,
                    &ssh_alias,
                    &hub_machine_id,
                    &schema_digests,
                    factory.as_ref(),
                    limits,
                    clock.as_ref(),
                    &request,
                ));
                let _ = request.response.send(result);
            }
        }
    }
    runtime.block_on(close_all(&mut peers));
}

async fn process_call(
    peers: &mut BTreeMap<McpCapability, CachedPeer>,
    ssh_alias: &str,
    hub_machine_id: &str,
    schema_digests: &BTreeMap<McpCapability, String>,
    factory: &dyn HubPeerFactory,
    limits: HubLinkLimits,
    clock: &dyn HubClock,
    request: &CallRequest,
) -> Result<Value, OrbitError> {
    let now = clock.now();
    if now.saturating_sub(request.queued_at) > limits.submission {
        return Err(OrbitError::HubUnavailable(
            "hub request expired in the bounded queue before transport handoff".to_string(),
        ));
    }
    reap_idle(peers, now, limits.idle).await;
    if peers
        .get(&request.capability)
        .is_some_and(|cached| cached.peer.is_closed())
        && let Some(mut stale) = peers.remove(&request.capability)
    {
        stale.peer.close().await;
    }
    if !peers.contains_key(&request.capability) {
        let schema_digest = schema_digests.get(&request.capability).ok_or_else(|| {
            OrbitError::HubNegotiation(format!(
                "no local schema digest exists for capability '{}'",
                request.capability
            ))
        })?;
        let spec = HubSpawnSpec {
            ssh_alias: ssh_alias.to_string(),
            hub_machine_id: hub_machine_id.to_string(),
            capability: request.capability,
            schema_digest: schema_digest.clone(),
        };
        let peer = factory.connect(&spec, limits).await?;
        peers.insert(
            request.capability,
            CachedPeer {
                peer,
                last_used: clock.now(),
            },
        );
    }
    let cached = peers
        .get_mut(&request.capability)
        .ok_or_else(|| OrbitError::HubUnavailable("hub peer disappeared".to_string()))?;
    let result = match tokio::time::timeout(
        limits.request,
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
                limits.request.as_millis()
            ),
        }),
    };
    cached.last_used = clock.now();
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::json;

    use super::*;

    #[derive(Default)]
    struct FakeFactory {
        connects: Arc<Mutex<Vec<HubSpawnSpec>>>,
        calls: Arc<Mutex<Vec<(McpCapability, String)>>>,
        fail_unknown_once: Arc<Mutex<bool>>,
        silent_once: Arc<Mutex<bool>>,
    }

    impl HubPeerFactory for FakeFactory {
        fn connect<'a>(
            &'a self,
            spec: &'a HubSpawnSpec,
            _limits: HubLinkLimits,
        ) -> BoxFuture<'a, Result<Box<dyn HubPeer>, OrbitError>> {
            self.connects.lock().expect("connects").push(spec.clone());
            let peer = FakePeer {
                capability: spec.capability,
                calls: Arc::clone(&self.calls),
                fail_unknown_once: Arc::clone(&self.fail_unknown_once),
                silent_once: Arc::clone(&self.silent_once),
                closed: false,
            };
            Box::pin(async move { Ok(Box::new(peer) as Box<dyn HubPeer>) })
        }
    }

    struct FakePeer {
        capability: McpCapability,
        calls: Arc<Mutex<Vec<(McpCapability, String)>>>,
        fail_unknown_once: Arc<Mutex<bool>>,
        silent_once: Arc<Mutex<bool>>,
        closed: bool,
    }

    impl HubPeer for FakePeer {
        fn is_closed(&self) -> bool {
            self.closed
        }

        fn call<'a>(
            &'a mut self,
            name: &'a str,
            _input: Value,
            context: &'a ToolSessionContext,
        ) -> BoxFuture<'a, Result<Value, OrbitError>> {
            self.calls
                .lock()
                .expect("calls")
                .push((self.capability, name.to_string()));
            let fail = {
                let mut guard = self.fail_unknown_once.lock().expect("failure flag");
                let fail = *guard;
                *guard = false;
                fail
            };
            let silent = {
                let mut guard = self.silent_once.lock().expect("silent flag");
                let silent = *guard;
                *guard = false;
                silent
            };
            let call_id = context.mcp_call_id.clone().unwrap_or_default();
            Box::pin(async move {
                if silent {
                    std::future::pending::<Result<Value, OrbitError>>().await
                } else if fail {
                    Err(OrbitError::OutcomeUnknown {
                        mcp_call_id: call_id,
                        message: "injected post-handoff loss".to_string(),
                    })
                } else {
                    Ok(json!({"ok": true}))
                }
            })
        }

        fn close<'a>(&'a mut self) -> BoxFuture<'a, ()> {
            self.closed = true;
            Box::pin(async {})
        }
    }

    fn context(capability: McpCapability, call_id: &str) -> ToolSessionContext {
        ToolSessionContext {
            workspace: Some("ws_orbit".to_string()),
            workspace_id: Some("ws_orbit".to_string()),
            caller_machine_id: Some("hm_spoke".to_string()),
            caller_host_id: Some("spoke".to_string()),
            transport: Some(orbit_common::types::McpTransport::SshMcp),
            effective_capabilities: std::collections::BTreeSet::from([capability]),
            origin_session_id: Some("session".to_string()),
            mcp_call_id: Some(call_id.to_string()),
            ..ToolSessionContext::default()
        }
    }

    fn test_pool(factory: Arc<FakeFactory>) -> HubLinkPool {
        test_pool_with(
            factory,
            HubLinkLimits::default(),
            Arc::new(MonotonicClock::default()),
        )
    }

    fn test_pool_with(
        factory: Arc<FakeFactory>,
        limits: HubLinkLimits,
        clock: Arc<dyn HubClock>,
    ) -> HubLinkPool {
        HubLinkPool::with_factory(
            "dk1".to_string(),
            "hm_hub".to_string(),
            BTreeMap::from([
                (McpCapability::Agent, "agent-digest".to_string()),
                (McpCapability::Operator, "operator-digest".to_string()),
            ]),
            factory,
            limits,
            clock,
        )
        .expect("pool")
    }

    #[derive(Default)]
    struct ManualClock(AtomicU64);

    impl ManualClock {
        fn advance(&self, duration: Duration) {
            self.0.fetch_add(
                u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
                Ordering::SeqCst,
            );
        }
    }

    impl HubClock for ManualClock {
        fn now(&self) -> Duration {
            Duration::from_millis(self.0.load(Ordering::SeqCst))
        }
    }

    #[test]
    fn fixed_ssh_argv_has_no_shell_or_configurable_fragments() {
        let spec = HubSpawnSpec {
            ssh_alias: "dk1".to_string(),
            hub_machine_id: "hm_hub".to_string(),
            capability: McpCapability::Operator,
            schema_digest: "digest".to_string(),
        };
        assert_eq!(
            spec.argv(),
            [
                "ssh",
                "dk1",
                "orbit",
                "mcp",
                "serve",
                "--hub",
                "--capabilities",
                "operator",
            ]
        );
    }

    #[test]
    fn reuses_one_peer_per_scalar_capability_and_separates_capabilities() {
        let factory = Arc::new(FakeFactory::default());
        let pool = test_pool(Arc::clone(&factory));
        pool.call(
            McpCapability::Agent,
            "orbit.task.show",
            json!({}),
            context(McpCapability::Agent, "mcall-1"),
        )
        .expect("first call");
        pool.call(
            McpCapability::Agent,
            "orbit.task.list",
            json!({}),
            context(McpCapability::Agent, "mcall-2"),
        )
        .expect("reused call");
        pool.call(
            McpCapability::Operator,
            "orbit.host.list",
            json!({}),
            context(McpCapability::Operator, "mcall-3"),
        )
        .expect("operator call");
        let connects = factory.connects.lock().expect("connects");
        assert_eq!(connects.len(), 2);
        assert_eq!(connects[0].capability, McpCapability::Agent);
        assert_eq!(connects[1].capability, McpCapability::Operator);
    }

    #[test]
    fn outcome_unknown_is_not_replayed_and_next_call_reconnects() {
        let factory = Arc::new(FakeFactory::default());
        *factory.fail_unknown_once.lock().expect("failure flag") = true;
        let pool = test_pool(Arc::clone(&factory));
        let error = pool
            .call(
                McpCapability::Agent,
                "orbit.task.update",
                json!({}),
                context(McpCapability::Agent, "mcall-original"),
            )
            .expect_err("unknown outcome");
        assert!(matches!(
            error,
            OrbitError::OutcomeUnknown { ref mcp_call_id, .. } if mcp_call_id == "mcall-original"
        ));
        assert_eq!(factory.calls.lock().expect("calls").len(), 1);
        pool.call(
            McpCapability::Agent,
            "orbit.task.show",
            json!({}),
            context(McpCapability::Agent, "mcall-next"),
        )
        .expect("later call reconnects");
        assert_eq!(factory.calls.lock().expect("calls").len(), 2);
        assert_eq!(factory.connects.lock().expect("connects").len(), 2);
    }

    #[test]
    fn fake_time_idle_expiry_evicts_and_reconnects() {
        let factory = Arc::new(FakeFactory::default());
        let clock = Arc::new(ManualClock::default());
        let mut limits = HubLinkLimits::default();
        limits.idle = Duration::from_secs(10);
        let pool = test_pool_with(
            Arc::clone(&factory),
            limits,
            Arc::clone(&clock) as Arc<dyn HubClock>,
        );
        pool.call(
            McpCapability::Agent,
            "orbit.task.show",
            json!({}),
            context(McpCapability::Agent, "mcall-before-idle"),
        )
        .expect("first call");
        clock.advance(Duration::from_secs(11));
        pool.call(
            McpCapability::Agent,
            "orbit.task.show",
            json!({}),
            context(McpCapability::Agent, "mcall-after-idle"),
        )
        .expect("call after idle");
        assert_eq!(factory.connects.lock().expect("connects").len(), 2);
    }

    #[test]
    fn silent_peer_times_out_and_bounded_queue_saturates_before_handoff() {
        let factory = Arc::new(FakeFactory::default());
        *factory.silent_once.lock().expect("silent flag") = true;
        let mut limits = HubLinkLimits::default();
        limits.queue_capacity = 1;
        limits.request = Duration::from_millis(50);
        limits.caller_wait = Duration::from_secs(1);
        let pool = Arc::new(test_pool_with(
            Arc::clone(&factory),
            limits,
            Arc::new(MonotonicClock::default()),
        ));
        let first_pool = Arc::clone(&pool);
        let first = std::thread::spawn(move || {
            first_pool.call(
                McpCapability::Agent,
                "orbit.task.update",
                json!({}),
                context(McpCapability::Agent, "mcall-silent"),
            )
        });
        while factory.calls.lock().expect("calls").is_empty() {
            std::thread::yield_now();
        }

        let (queued_tx, queued_rx) = mpsc::sync_channel(1);
        pool.tx
            .as_ref()
            .expect("pool sender")
            .try_send(WorkerMessage::Call(CallRequest {
                capability: McpCapability::Agent,
                name: "orbit.task.show".to_string(),
                input: json!({}),
                context: context(McpCapability::Agent, "mcall-queued"),
                queued_at: pool.clock.now(),
                response: queued_tx,
            }))
            .expect("one bounded queue slot");
        let saturated = pool
            .call(
                McpCapability::Agent,
                "orbit.task.show",
                json!({}),
                context(McpCapability::Agent, "mcall-saturated"),
            )
            .expect_err("queue is full");
        assert!(matches!(saturated, OrbitError::HubUnavailable(_)));
        let timed_out = first.join().expect("first caller").expect_err("timeout");
        assert!(matches!(
            timed_out,
            OrbitError::OutcomeUnknown { ref mcp_call_id, .. } if mcp_call_id == "mcall-silent"
        ));
        assert!(queued_rx.recv_timeout(Duration::from_secs(1)).is_ok());
    }
}
