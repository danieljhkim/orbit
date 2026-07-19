use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use orbit_common::types::{
    AuditEventStatus, HostRegistration, HostStatus, SPOKE_REGISTRATION_SCHEMA_VERSION, Workspace,
    WorkspacePresenceDeclaration, WorkspaceRegistry, WorkspaceStatus,
};
use orbit_core::routines::load_host_identity;
use orbit_core::{RegistryCacheService, RegistryCacheState};
use orbit_mcp::{McpHost, OrbitToolServer};
use rmcp::ServiceExt;
use serde_json::json;

use super::super::host::{BrokerMcpHost, canonical_mcp_tool_definitions};
use super::super::hub::HubMcpHost;
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

#[derive(Default)]
struct RmcpHubFactory {
    hub_root: PathBuf,
    connects: Mutex<Vec<HubSpawnSpec>>,
    wire_calls: Arc<Mutex<Vec<(String, Value, ToolSessionContext)>>>,
}

impl RmcpHubFactory {
    fn new(hub_root: PathBuf) -> Self {
        Self {
            hub_root,
            ..Self::default()
        }
    }
}

impl HubPeerFactory for RmcpHubFactory {
    fn connect<'a>(
        &'a self,
        spec: &'a HubSpawnSpec,
        limits: HubLinkLimits,
    ) -> BoxFuture<'a, Result<Box<dyn HubPeer>, OrbitError>> {
        self.connects.lock().expect("connects").push(spec.clone());
        let hub_root = self.hub_root.clone();
        let spec = spec.clone();
        let wire_calls = Arc::clone(&self.wire_calls);
        Box::pin(async move {
            let host = Arc::new(HubMcpHost::new(hub_root, spec.capability)?);
            let mut trusted = ToolSessionContext::trusted_local(
                None,
                Some(host.identity().machine_id.clone()),
                Some(host.identity().host_id.clone()),
            );
            trusted.effective_capabilities = BTreeSet::from([spec.capability]);
            let server = OrbitToolServer::new_with_context(host, trusted);
            let (server_io, client_io) = tokio::io::duplex(256 * 1024);
            let server_task = tokio::spawn(async move {
                if let Ok(running) = server.serve(server_io).await {
                    let _ = running.waiting().await;
                }
            });
            let (read, write) = tokio::io::split(client_io);
            let client =
                OrbitMcpClient::connect(read, write, &spec.expectation(), limits.initialize)
                    .await?;
            Ok(Box::new(RmcpHubPeer {
                client,
                server_task,
                wire_calls,
                request_timeout: limits.request,
                close_timeout: limits.close,
            }) as Box<dyn HubPeer>)
        })
    }
}

struct RmcpHubPeer {
    client: OrbitMcpClient,
    server_task: tokio::task::JoinHandle<()>,
    wire_calls: Arc<Mutex<Vec<(String, Value, ToolSessionContext)>>>,
    request_timeout: Duration,
    close_timeout: Duration,
}

impl HubPeer for RmcpHubPeer {
    fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    fn call<'a>(
        &'a mut self,
        name: &'a str,
        input: Value,
        context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<Value, OrbitError>> {
        self.wire_calls.lock().expect("wire calls").push((
            name.to_string(),
            input.clone(),
            context.clone(),
        ));
        Box::pin(async move {
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
        self.wire_calls.lock().expect("wire calls").push((
            orbit_common::types::SPOKE_REGISTRATION_METHOD_V1.to_string(),
            serde_json::to_value(request).expect("serialize registration"),
            context.clone(),
        ));
        Box::pin(async move {
            self.client
                .register_spoke(request, context, self.request_timeout)
                .await
        })
    }

    fn close<'a>(&'a mut self) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let _ = self.client.close(self.close_timeout).await;
            self.server_task.abort();
        })
    }
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

    fn register_spoke<'a>(
        &'a mut self,
        _request: &'a SpokeRegistrationRequestV1,
        context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<SpokeRegistrationResultV1, OrbitError>> {
        self.calls.lock().expect("calls").push((
            self.capability,
            orbit_common::types::SPOKE_REGISTRATION_METHOD_V1.to_string(),
        ));
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
                std::future::pending::<Result<SpokeRegistrationResultV1, OrbitError>>().await
            } else if fail {
                Err(OrbitError::OutcomeUnknown {
                    mcp_call_id: call_id,
                    message: "injected post-handoff registration loss".to_string(),
                })
            } else {
                Ok(SpokeRegistrationResultV1::failed(
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                    "fake",
                    "fake definitive result",
                ))
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

fn registration_context(capability: McpCapability, call_id: &str) -> ToolSessionContext {
    ToolSessionContext {
        caller_machine_id: Some("hm_spoke".to_string()),
        caller_host_id: Some("spoke".to_string()),
        transport: Some(orbit_common::types::McpTransport::SshMcp),
        effective_capabilities: std::collections::BTreeSet::from([capability]),
        origin_session_id: Some("session".to_string()),
        mcp_call_id: Some(call_id.to_string()),
        ..ToolSessionContext::default()
    }
}

fn registration() -> SpokeRegistrationRequestV1 {
    SpokeRegistrationRequestV1 {
        schema_version: orbit_common::types::SPOKE_REGISTRATION_SCHEMA_VERSION,
        identity: orbit_common::types::HostRegistration {
            machine_id: "hm_spoke".to_string(),
            host_id: "spoke".to_string(),
            labels: std::collections::BTreeSet::new(),
        },
        presence: Vec::new(),
        profiles: Vec::new(),
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
fn private_registration_is_queued_once_on_one_verified_capability_peer() {
    let factory = Arc::new(FakeFactory::default());
    let pool = test_pool(Arc::clone(&factory));
    let result = pool
        .register_spoke(
            McpCapability::Agent,
            registration(),
            registration_context(McpCapability::Agent, "mcall-register"),
        )
        .expect("definitive registration result");
    assert!(!result.complete);
    assert_eq!(factory.connects.lock().expect("connects").len(), 1);
    assert_eq!(factory.calls.lock().expect("calls").len(), 1);
    assert_eq!(
        factory.calls.lock().expect("calls")[0].1,
        orbit_common::types::SPOKE_REGISTRATION_METHOD_V1
    );
}

#[test]
fn private_registration_outcome_unknown_is_not_replayed() {
    let factory = Arc::new(FakeFactory::default());
    *factory.fail_unknown_once.lock().expect("failure flag") = true;
    let pool = test_pool(Arc::clone(&factory));
    let error = pool
        .register_spoke(
            McpCapability::Agent,
            registration(),
            registration_context(McpCapability::Agent, "mcall-register-loss"),
        )
        .expect_err("unknown registration outcome");
    assert!(matches!(
        error,
        OrbitError::OutcomeUnknown { ref mcp_call_id, .. }
            if mcp_call_id == "mcall-register-loss"
    ));
    assert_eq!(factory.calls.lock().expect("calls").len(), 1);
    assert_eq!(factory.connects.lock().expect("connects").len(), 1);
}

#[test]
fn fake_time_idle_expiry_evicts_and_reconnects() {
    let factory = Arc::new(FakeFactory::default());
    let clock = Arc::new(ManualClock::default());
    let limits = HubLinkLimits {
        idle: Duration::from_secs(10),
        ..HubLinkLimits::default()
    };
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
    let limits = HubLinkLimits {
        queue_capacity: 1,
        request: Duration::from_millis(50),
        ..HubLinkLimits::default()
    };
    let clock = Arc::new(ManualClock::default());
    let pool = Arc::new(test_pool_with(
        Arc::clone(&factory),
        limits,
        Arc::clone(&clock) as Arc<dyn HubClock>,
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
            response: queued_tx,
        }))
        .expect("one bounded queue slot");
    clock.advance(Duration::from_secs(3));
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
    assert!(
        queued_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("queued response")
            .is_ok(),
        "accepted queue wait must not become a pre-handoff expiry"
    );
}

fn write_canary_identity(root: &Path, mode: &str, machine_id: &str, host_id: &str) {
    std::fs::write(
        root.join("host.toml"),
        format!(
            "schema_version = 1\nmachine_id = \"{machine_id}\"\nhost_id = \"{host_id}\"\nmode = \"{mode}\"\n"
        ),
    )
    .expect("host identity");
}

fn canary_workspace() -> Workspace {
    Workspace {
        id: "ws_canary".to_string(),
        name: "RMCP canary".to_string(),
        owner_machine_id: Some("hm_hub".to_string()),
        git_remote: None,
        ship_mode: None,
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn save_canary_registry(root: &Path, registry: &WorkspaceRegistry) {
    orbit_core::workspace_registry::save_registry_to(
        registry,
        &orbit_core::workspace_registry::registry_path_for(root),
    )
    .expect("workspace registry");
}

fn canary_context(capability: McpCapability, call_id: &str) -> ToolSessionContext {
    let mut context = ToolSessionContext::trusted_local(
        None,
        Some("hm_spoke".to_string()),
        Some("spoke".to_string()),
    );
    context.effective_capabilities = BTreeSet::from([capability]);
    context.origin_session_id = Some("session-canary".to_string());
    context.mcp_call_id = Some(call_id.to_string());
    context
}

#[test]
fn spoke_rmcp_coordination_canary_is_hub_only_and_preserves_provenance() {
    let hub = tempfile::tempdir().expect("hub root");
    let spoke = tempfile::tempdir().expect("spoke root");
    write_canary_identity(hub.path(), "hub", "hm_hub", "hub");
    write_canary_identity(spoke.path(), "spoke", "hm_spoke", "spoke");

    let registry = WorkspaceRegistry {
        workspaces: vec![canary_workspace()],
        ..WorkspaceRegistry::default()
    };
    save_canary_registry(hub.path(), &registry);
    save_canary_registry(spoke.path(), &registry);

    let registry_service = orbit_core::host_registry::host_registry_service_at(hub.path())
        .expect("hub registry service");
    registry_service
        .register_hub_identity(
            &load_host_identity(hub.path()).expect("hub identity"),
            BTreeSet::new(),
        )
        .expect("register hub");
    registry_service
        .bind_workspace_owner(&registry, "ws_canary", "hm_hub")
        .expect("bind owner");
    orbit_core::runtime::HubCoordinationExecutor::register_workspace(
        hub.path(),
        "ws_canary",
        "canary",
    )
    .expect("hub coordination workspace");

    let definitions = canonical_mcp_tool_definitions().expect("canonical definitions");
    let digests = [McpCapability::Agent, McpCapability::Operator]
        .into_iter()
        .map(|capability| {
            orbit_mcp::hub_schema_digest(&definitions, capability)
                .map(|digest| (capability, digest))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()
        .expect("hub schema digests");
    let factory = Arc::new(RmcpHubFactory::new(hub.path().to_path_buf()));
    let pool = HubLinkPool::with_factory(
        "unused-hermetic-alias".to_string(),
        "hm_hub".to_string(),
        digests,
        Arc::clone(&factory) as Arc<dyn HubPeerFactory>,
        HubLinkLimits::default(),
        Arc::new(MonotonicClock::default()),
    )
    .expect("hub link pool");

    let spoke_checkout = spoke.path().join("checkout");
    std::fs::create_dir_all(&spoke_checkout).expect("spoke checkout fixture");
    let registration = SpokeRegistrationRequestV1 {
        schema_version: SPOKE_REGISTRATION_SCHEMA_VERSION,
        identity: HostRegistration {
            machine_id: "hm_spoke".to_string(),
            host_id: "spoke".to_string(),
            labels: BTreeSet::from(["canary".to_string()]),
        },
        presence: vec![WorkspacePresenceDeclaration {
            workspace_id: "ws_canary".to_string(),
            root: spoke_checkout.clone(),
            last_verified: Utc::now(),
        }],
        profiles: Vec::new(),
    };
    let registration_result = pool
        .register_spoke(McpCapability::Agent, registration, {
            let mut context = canary_context(McpCapability::Agent, "mcall-canary-register");
            context.transport = Some(orbit_common::types::McpTransport::SshMcp);
            context.process_machine_id = None;
            context.process_host_id = None;
            context
        })
        .expect("register spoke over duplex RMCP");
    assert!(registration_result.complete);
    assert_eq!(
        registration_result
            .host
            .as_ref()
            .map(|host| (&host.machine_id, host.status)),
        Some((&"hm_spoke".to_string(), HostStatus::Active))
    );
    let snapshot = registration_result.snapshot.expect("sanitized snapshot");
    let snapshot_json = serde_json::to_string(&snapshot).expect("snapshot JSON");
    assert!(!snapshot_json.contains(&spoke_checkout.to_string_lossy().to_string()));
    RegistryCacheService::new(spoke.path())
        .refresh(snapshot, Utc::now())
        .expect("refresh spoke cache after definitive registration");
    assert!(matches!(
        RegistryCacheService::new(spoke.path())
            .load(Utc::now(), chrono::Duration::minutes(5))
            .expect("load spoke cache"),
        RegistryCacheState::Current { .. }
    ));

    let broker = BrokerMcpHost::new_with_hub_link(spoke.path().to_path_buf(), pool);
    let mut audited_calls = Vec::<(&str, &str, McpCapability, Option<&str>)>::new();
    let mut responses = Vec::new();
    {
        let mut call = |name: &'static str,
                        input: Value,
                        call_id: &'static str,
                        capability: McpCapability,
                        workspace: Option<&'static str>| {
            let result = broker
                .call_tool(name, input, canary_context(capability, call_id))
                .unwrap_or_else(|error| panic!("{name} through spoke broker: {error}"));
            audited_calls.push((name, call_id, capability, workspace));
            responses.push(result.clone());
            result
        };

        let hosts = call(
            "orbit.host.list",
            json!({}),
            "mcall-canary-host-list",
            McpCapability::Operator,
            None,
        );
        assert_eq!(hosts["hub_machine_id"], "hm_hub");
        assert_eq!(hosts["hosts"].as_array().expect("hosts").len(), 2);

        let workspaces = call(
            "orbit.workspace.list",
            json!({}),
            "mcall-canary-workspace-list",
            McpCapability::Operator,
            None,
        );
        assert_eq!(workspaces["workspaces"][0]["workspace_id"], "ws_canary");
        assert_eq!(workspaces["workspaces"][0]["owner_machine_id"], "hm_hub");

        let task = call(
            "orbit.task.add",
            json!({
                "workspace": "ws_canary",
                "title": "RMCP canary",
                "description": "Hub only",
                "model": "codex"
            }),
            "mcall-canary-task-add",
            McpCapability::Agent,
            Some("ws_canary"),
        );
        let task_id = task["id"].as_str().expect("task id").to_string();

        let tasks = call(
            "orbit.task.list",
            json!({"workspace": "ws_canary", "limit": 10}),
            "mcall-canary-task-list",
            McpCapability::Agent,
            Some("ws_canary"),
        );
        assert_eq!(tasks["items"][0]["id"], task_id);

        let updated = call(
            "orbit.task.update",
            json!({
                "workspace": "ws_canary",
                "id": task_id,
                "plan": "1. Prove hub routing",
                "model": "codex"
            }),
            "mcall-canary-task-update",
            McpCapability::Agent,
            Some("ws_canary"),
        );
        assert_eq!(updated["plan"], "1. Prove hub routing");

        let artifact_source = spoke.path().join("caller-artifact.txt");
        std::fs::write(&artifact_source, "spoke payload").expect("artifact source");
        call(
            "orbit.task.artifact.put",
            json!({
                "workspace": "ws_canary",
                "id": task_id,
                "source_path": artifact_source,
                "path": "reports/result.txt",
                "model": "codex"
            }),
            "mcall-canary-artifact-put",
            McpCapability::Agent,
            Some("ws_canary"),
        );

        let shown = call(
            "orbit.task.show",
            json!({
                "workspace": "ws_canary",
                "id": task_id,
                "fields": ["plan", "artifacts"]
            }),
            "mcall-canary-task-show",
            McpCapability::Agent,
            Some("ws_canary"),
        );
        assert_eq!(shown["plan"], "1. Prove hub routing");
        assert_eq!(shown["artifacts"][0]["path"], "reports/result.txt");

        let reviewed = call(
            "orbit.task.review_thread.add",
            json!({
                "workspace": "ws_canary",
                "id": task_id,
                "body": "Canary finding",
                "path": "src/lib.rs",
                "line": "7",
                "model": "codex"
            }),
            "mcall-canary-review-add",
            McpCapability::Agent,
            Some("ws_canary"),
        );
        let thread_id = reviewed["review_threads"][0]["thread_id"]
            .as_str()
            .expect("thread id")
            .to_string();
        let threads = call(
            "orbit.task.review_thread.list",
            json!({"workspace": "ws_canary", "id": task_id, "status": "open"}),
            "mcall-canary-review-list",
            McpCapability::Agent,
            Some("ws_canary"),
        );
        assert_eq!(threads["items"][0]["thread_id"], thread_id);
        let replied = call(
            "orbit.task.review_thread.reply",
            json!({
                "workspace": "ws_canary",
                "id": task_id,
                "thread_id": thread_id,
                "body": "Canary reply",
                "model": "codex"
            }),
            "mcall-canary-review-reply",
            McpCapability::Agent,
            Some("ws_canary"),
        );
        assert_eq!(
            replied["review_threads"][0]["messages"]
                .as_array()
                .expect("messages")
                .len(),
            2
        );
        let resolved = call(
            "orbit.task.review_thread.resolve",
            json!({
                "workspace": "ws_canary",
                "id": task_id,
                "thread_id": thread_id,
                "model": "codex"
            }),
            "mcall-canary-review-resolve",
            McpCapability::Agent,
            Some("ws_canary"),
        );
        assert_eq!(resolved["review_threads"][0]["status"], "resolved");

        let friction = call(
            "orbit.friction.add",
            json!({
                "workspace": "ws_canary",
                "body": "RMCP canary friction",
                "tags": ["tooling"],
                "model": "codex"
            }),
            "mcall-canary-friction-add",
            McpCapability::Agent,
            Some("ws_canary"),
        );
        assert!(friction.get("path").is_none(), "hub response is path-free");
        let friction_id = friction["id"].as_str().expect("friction id").to_string();
        let frictions = call(
            "orbit.friction.list",
            json!({
                "workspace": "ws_canary",
                "q": "RMCP canary friction",
                "limit": 10
            }),
            "mcall-canary-friction-list",
            McpCapability::Operator,
            Some("ws_canary"),
        );
        assert_eq!(frictions["items"][0]["id"], friction_id);
        assert!(frictions["items"][0].get("path").is_none());
        let friction_shown = call(
            "orbit.friction.show",
            json!({"workspace": "ws_canary", "id": friction_id}),
            "mcall-canary-friction-show",
            McpCapability::Operator,
            Some("ws_canary"),
        );
        assert_eq!(friction_shown["body"], "RMCP canary friction");
        let friction_updated = call(
            "orbit.friction.update",
            json!({
                "workspace": "ws_canary",
                "id": friction_id,
                "status": "triaged",
                "body": "RMCP canary triaged"
            }),
            "mcall-canary-friction-update",
            McpCapability::Operator,
            Some("ws_canary"),
        );
        assert_eq!(friction_updated["status"], "triaged");
        assert_eq!(friction_updated["body"], "RMCP canary triaged");
    }

    let wire_calls = factory.wire_calls.lock().expect("wire calls");
    assert_eq!(wire_calls.len(), 16, "one registration plus 15 calls");
    let (_, artifact_frame, artifact_context) = wire_calls
        .iter()
        .find(|(name, _, _)| name == "orbit.task.artifact.put")
        .expect("artifact frame");
    assert!(artifact_frame.get("source_path").is_none());
    assert!(artifact_frame.get("artifacts").is_some());
    assert_eq!(artifact_context.workspace_id.as_deref(), Some("ws_canary"));
    let registration_frame = wire_calls
        .iter()
        .find(|(name, _, _)| name == orbit_common::types::SPOKE_REGISTRATION_METHOD_V1)
        .expect("registration frame");
    assert!(
        serde_json::to_string(&registration_frame.1)
            .expect("registration JSON")
            .contains(&spoke_checkout.to_string_lossy().to_string()),
        "authenticated presence publication is the sole path-bearing frame"
    );
    let ordinary_wire_json = serde_json::to_string(
        &wire_calls
            .iter()
            .filter(|(name, _, _)| name != orbit_common::types::SPOKE_REGISTRATION_METHOD_V1)
            .collect::<Vec<_>>(),
    )
    .expect("ordinary wire JSON");
    assert!(!ordinary_wire_json.contains(&spoke.path().to_string_lossy().to_string()));
    assert!(!ordinary_wire_json.contains(&hub.path().to_string_lossy().to_string()));
    drop(wire_calls);

    let response_json = serde_json::to_string(&responses).expect("response JSON");
    assert!(!response_json.contains(&spoke.path().to_string_lossy().to_string()));
    assert!(!response_json.contains(&hub.path().to_string_lossy().to_string()));
    let cache_bytes = std::fs::read(RegistryCacheService::new(spoke.path()).cache_path())
        .expect("spoke registry cache");
    let cache_text = String::from_utf8(cache_bytes).expect("cache UTF-8");
    assert!(!cache_text.contains(&spoke.path().to_string_lossy().to_string()));
    assert!(!cache_text.contains(&hub.path().to_string_lossy().to_string()));

    let task_store = hub.path().join("tasks/index.sqlite");
    let task_connection = rusqlite::Connection::open(task_store).expect("hub task store");
    let task_count: i64 = task_connection
        .query_row(
            "SELECT COUNT(*) FROM task_bundle_index WHERE workspace_id = 'ws_canary'",
            [],
            |row| row.get(0),
        )
        .expect("hub task count");
    assert_eq!(task_count, 1);
    assert!(hub.path().join("frictions/workspaces/ws_canary").exists());
    assert!(!spoke.path().join("orbit.db").exists());
    assert!(!spoke.path().join("tasks").exists());
    assert!(!spoke.path().join("frictions").exists());

    let audit_connection =
        rusqlite::Connection::open(hub.path().join("orbit.db")).expect("hub audit store");
    for (name, call_id, capability, workspace) in audited_calls {
        let row = audit_connection
            .query_row(
                "SELECT COUNT(*), status, workspace_id, caller_machine_id, caller_host_id,
                        process_machine_id, process_host_id, transport, capabilities_json,
                        origin_session_id, mcp_call_id, tool_name
                 FROM audit_events WHERE mcp_call_id = ?1",
                [call_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                    ))
                },
            )
            .unwrap_or_else(|error| panic!("audit row for {call_id}: {error}"));
        assert_eq!(row.0, 1, "one canonical audit for {call_id}");
        assert_eq!(row.1, AuditEventStatus::Success.to_string());
        assert_eq!(row.2.as_deref(), workspace);
        assert_eq!(row.3.as_deref(), Some("hm_spoke"));
        assert_eq!(row.4.as_deref(), Some("spoke"));
        assert_eq!(row.5.as_deref(), Some("hm_hub"));
        assert_eq!(row.6.as_deref(), Some("hub"));
        assert_eq!(row.7.as_deref(), Some("ssh-mcp"));
        assert_eq!(
            row.8.as_deref(),
            Some(match capability {
                McpCapability::Agent => "[\"agent\"]",
                McpCapability::Operator => "[\"operator\"]",
                McpCapability::Runner => unreachable!("canary has no runner call"),
            })
        );
        assert_eq!(row.9.as_deref(), Some("session-canary"));
        assert_eq!(row.10.as_deref(), Some(call_id));
        assert_eq!(row.11.as_deref(), Some(name));
    }

    let registration_audit: i64 = audit_connection
        .query_row(
            "SELECT COUNT(*) FROM audit_events
             WHERE mcp_call_id = 'mcall-canary-register'
               AND tool_name = ?1
               AND status = 'success'
               AND caller_machine_id = 'hm_spoke'
               AND process_machine_id = 'hm_hub'",
            [orbit_common::types::SPOKE_REGISTRATION_METHOD_V1],
            |row| row.get(0),
        )
        .expect("registration audit");
    assert_eq!(registration_audit, 1);
    assert_eq!(factory.connects.lock().expect("connects").len(), 2);
}
