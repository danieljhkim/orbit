use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use chrono::Utc;
use orbit_common::types::{
    AuditEventStatus, McpCapability, OrbitError, ToolSessionContext, Workspace, WorkspaceRegistry,
    WorkspaceStatus,
};
use orbit_mcp::{McpHost, OrbitToolServer};
use rmcp::ServiceExt;
use serde_json::{Value, json};

use super::super::host::{BrokerMcpHost, OwnerRoute, canonical_mcp_tool_definitions};
use super::super::owner::OwnerMcpHost;
use super::super::owner_client::OrbitMcpClient;
use super::super::owner_link::{
    BoxFuture, CallRequest, MonotonicClock, OwnerClock, OwnerLinkLimits, OwnerLinkPool, OwnerPeer,
    OwnerPeerFactory, OwnerSpawnSpec, WorkerMessage,
};
use super::super::owner_server_composition;

#[derive(Default)]
struct FakeFactory {
    connects: Arc<Mutex<Vec<OwnerSpawnSpec>>>,
    calls: Arc<Mutex<Vec<(McpCapability, String)>>>,
    fail_unknown_once: Arc<Mutex<bool>>,
    silent_once: Arc<Mutex<bool>>,
}

impl OwnerPeerFactory for FakeFactory {
    fn connect<'a>(
        &'a self,
        spec: &'a OwnerSpawnSpec,
        _limits: OwnerLinkLimits,
    ) -> BoxFuture<'a, Result<Box<dyn OwnerPeer>, OrbitError>> {
        self.connects.lock().expect("connects").push(spec.clone());
        let peer = FakePeer {
            capability: spec.capability,
            calls: Arc::clone(&self.calls),
            fail_unknown_once: Arc::clone(&self.fail_unknown_once),
            silent_once: Arc::clone(&self.silent_once),
            closed: false,
        };
        Box::pin(async move { Ok(Box::new(peer) as Box<dyn OwnerPeer>) })
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
struct RmcpOwnerFactory {
    owner_root: PathBuf,
    connects: Mutex<Vec<OwnerSpawnSpec>>,
    wire_calls: Arc<Mutex<Vec<(String, Value, ToolSessionContext)>>>,
}

impl RmcpOwnerFactory {
    fn new(owner_root: PathBuf) -> Self {
        Self {
            owner_root,
            ..Self::default()
        }
    }
}

impl OwnerPeerFactory for RmcpOwnerFactory {
    fn connect<'a>(
        &'a self,
        spec: &'a OwnerSpawnSpec,
        limits: OwnerLinkLimits,
    ) -> BoxFuture<'a, Result<Box<dyn OwnerPeer>, OrbitError>> {
        self.connects.lock().expect("connects").push(spec.clone());
        let owner_root = self.owner_root.clone();
        let spec = spec.clone();
        let wire_calls = Arc::clone(&self.wire_calls);
        Box::pin(async move {
            let host = Arc::new(OwnerMcpHost::new(owner_root, spec.capability)?);
            let mut trusted = ToolSessionContext::trusted_local(
                None,
                Some(host.identity().machine_id.clone()),
                Some(host.identity().host_id.clone()),
            );
            trusted.effective_capabilities = BTreeSet::from([spec.capability]);
            let composition = owner_server_composition(Arc::clone(&host));
            let server =
                OrbitToolServer::new_with_context_and_composition(host, trusted, composition);
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
            Ok(Box::new(RmcpOwnerPeer {
                client,
                server_task,
                wire_calls,
                request_timeout: limits.request,
                close_timeout: limits.close,
            }) as Box<dyn OwnerPeer>)
        })
    }
}

struct RmcpOwnerPeer {
    client: OrbitMcpClient,
    server_task: tokio::task::JoinHandle<()>,
    wire_calls: Arc<Mutex<Vec<(String, Value, ToolSessionContext)>>>,
    request_timeout: Duration,
    close_timeout: Duration,
}

impl OwnerPeer for RmcpOwnerPeer {
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

    fn close<'a>(&'a mut self) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let _ = self.client.close(self.close_timeout).await;
            self.server_task.abort();
        })
    }
}

impl OwnerPeer for FakePeer {
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
        caller_machine_id: Some("hm_client".to_string()),
        caller_host_id: Some("client".to_string()),
        transport: Some(orbit_common::types::McpTransport::SshMcp),
        effective_capabilities: std::collections::BTreeSet::from([capability]),
        origin_session_id: Some("session".to_string()),
        mcp_call_id: Some(call_id.to_string()),
        ..ToolSessionContext::default()
    }
}

fn test_pool(factory: Arc<FakeFactory>) -> OwnerLinkPool {
    test_pool_with(
        factory,
        OwnerLinkLimits::default(),
        Arc::new(MonotonicClock::default()),
    )
}

fn test_pool_with(
    factory: Arc<FakeFactory>,
    limits: OwnerLinkLimits,
    clock: Arc<dyn OwnerClock>,
) -> OwnerLinkPool {
    OwnerLinkPool::with_factory(
        "dk1".to_string(),
        "hm_owner".to_string(),
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

impl OwnerClock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

#[test]
fn fixed_ssh_argv_has_no_shell_or_configurable_fragments() {
    let spec = OwnerSpawnSpec {
        ssh_alias: "dk1".to_string(),
        owner_machine_id: "hm_owner".to_string(),
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
            "--owner",
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
        "orbit.workspace.list",
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
    let limits = OwnerLinkLimits {
        idle: Duration::from_secs(10),
        ..OwnerLinkLimits::default()
    };
    let pool = test_pool_with(
        Arc::clone(&factory),
        limits,
        Arc::clone(&clock) as Arc<dyn OwnerClock>,
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
    let limits = OwnerLinkLimits {
        queue_capacity: 1,
        request: Duration::from_millis(50),
        ..OwnerLinkLimits::default()
    };
    let clock = Arc::new(ManualClock::default());
    let pool = Arc::new(test_pool_with(
        Arc::clone(&factory),
        limits,
        Arc::clone(&clock) as Arc<dyn OwnerClock>,
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
        .try_send(WorkerMessage::Call(Box::new(CallRequest {
            capability: McpCapability::Agent,
            name: "orbit.task.show".to_string(),
            input: json!({}),
            context: context(McpCapability::Agent, "mcall-queued"),
            response: queued_tx,
        })))
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
    assert!(matches!(saturated, OrbitError::OwnerUnavailable(_)));
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

fn write_canary_identity(root: &Path, machine_id: &str, host_id: &str) {
    std::fs::write(
        root.join("host.toml"),
        format!(
            "schema_version = 2\nmachine_id = \"{machine_id}\"\nhost_id = \"{host_id}\"\ntask_prefix = \"ORB\"\n"
        ),
    )
    .expect("host identity");
}

fn canary_workspace() -> Workspace {
    Workspace {
        id: "ws_canary".to_string(),
        name: "RMCP canary".to_string(),
        owner_machine_id: Some("hm_owner".to_string()),
        git_remote: None,
        ship_mode: None,
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn save_canary_registry(root: &Path, registry: &WorkspaceRegistry) {
    crate::workspace_registry::save_registry_to(
        registry,
        &crate::workspace_registry::registry_path_for(root),
    )
    .expect("workspace registry");
}

fn canary_context(capability: McpCapability, call_id: &str) -> ToolSessionContext {
    let mut context = ToolSessionContext::trusted_local(
        None,
        Some("hm_client".to_string()),
        Some("client".to_string()),
    );
    context.effective_capabilities = BTreeSet::from([capability]);
    context.origin_session_id = Some("session-canary".to_string());
    context.mcp_call_id = Some(call_id.to_string());
    context
}

/// End-to-end owner routing over a real duplex RMCP peer.
///
/// ORB-10727 recomposes what this canary proves. The task surface still crosses
/// exactly one route to the workspace's owner and lands in the owner's
/// coordination store with the caller's provenance intact. What changed:
/// `orbit.workspace.list` is now `local-derived` and answers from the caller's
/// own registry without touching the wire, and the friction family is
/// owner-placed but not part of the task surface, so a workspace owned
/// elsewhere refuses it by name instead of forwarding it.
#[test]
fn owner_rmcp_coordination_canary_routes_only_the_task_surface_and_preserves_provenance() {
    let owner = tempfile::tempdir().expect("owner root");
    let client = tempfile::tempdir().expect("client root");
    write_canary_identity(owner.path(), "hm_owner", "owner");
    write_canary_identity(client.path(), "hm_client", "client");

    let registry = WorkspaceRegistry {
        workspaces: vec![canary_workspace()],
        ..WorkspaceRegistry::default()
    };
    save_canary_registry(owner.path(), &registry);
    save_canary_registry(client.path(), &registry);

    orbit_core::runtime::HubCoordinationExecutor::register_workspace(
        owner.path(),
        "ws_canary",
        "canary",
    )
    .expect("owner coordination workspace");

    let definitions = canonical_mcp_tool_definitions().expect("canonical definitions");
    let digests = [McpCapability::Agent, McpCapability::Operator]
        .into_iter()
        .map(|capability| {
            super::super::contract::owner_schema_digest(&definitions, capability)
                .map(|digest| (capability, digest))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()
        .expect("owner schema digests");
    let allowed_capabilities = digests.keys().copied().collect::<BTreeSet<_>>();
    let factory = Arc::new(RmcpOwnerFactory::new(owner.path().to_path_buf()));
    let pool = OwnerLinkPool::with_factory(
        "unused-hermetic-alias".to_string(),
        "hm_owner".to_string(),
        digests,
        Arc::clone(&factory) as Arc<dyn OwnerPeerFactory>,
        OwnerLinkLimits::default(),
        Arc::new(MonotonicClock::default()),
    )
    .expect("owner link pool");

    let broker = BrokerMcpHost::new_with_owner_routes(
        client.path().to_path_buf(),
        BTreeMap::from([(
            "hm_owner".to_string(),
            OwnerRoute {
                allowed_capabilities,
                pool: Arc::new(pool),
            },
        )]),
    );
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
                .unwrap_or_else(|error| panic!("{name} through client broker: {error}"));
            audited_calls.push((name, call_id, capability, workspace));
            responses.push(result.clone());
            result
        };

        let task = call(
            "orbit.task.add",
            json!({
                "workspace": "ws_canary",
                "title": "RMCP canary",
                "description": "Owner only",
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
                "plan": "1. Prove owner routing",
                "model": "codex"
            }),
            "mcall-canary-task-update",
            McpCapability::Agent,
            Some("ws_canary"),
        );
        assert_eq!(updated["plan"], "1. Prove owner routing");

        let artifact_source = client.path().join("caller-artifact.txt");
        std::fs::write(&artifact_source, "client payload").expect("artifact source");
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
        assert_eq!(shown["plan"], "1. Prove owner routing");
        assert_eq!(shown["artifacts"][0]["path"], "reports/result.txt");
    }

    // Owner-placed but outside the task surface: refused by name, never
    // relayed. ORB-10729 pins the three coordination writes the v1 boundary
    // deliberately excludes — friction lifecycle, workflow dispatch, and
    // alongside a friction read.
    for (name, capability) in [
        ("orbit.friction.add", McpCapability::Agent),
        ("orbit.friction.list", McpCapability::Operator),
        ("orbit.workflow.ship", McpCapability::Operator),
    ] {
        let refusal = broker
            .call_tool(
                name,
                json!({"workspace": "ws_canary", "body": "x", "model": "codex"}),
                canary_context(capability, "mcall-canary-refused"),
            )
            .expect_err("owner-placed non-task tool must be refused for a remote owner");
        let message = refusal.to_string();
        assert!(
            message.contains("hm_owner"),
            "{name} refusal must name the owning machine: {message}"
        );
        assert!(
            message.contains("only task tools may cross it"),
            "{name} refusal must explain the route limit: {message}"
        );
    }

    let wire_calls = factory.wire_calls.lock().expect("wire calls");
    assert_eq!(
        wire_calls
            .iter()
            .map(|(name, _, _)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "orbit.task.add",
            "orbit.task.list",
            "orbit.task.update",
            "orbit.task.artifact.put",
            "orbit.task.show",
        ],
        "only the task surface crosses the route"
    );
    let (_, artifact_frame, artifact_context) = wire_calls
        .iter()
        .find(|(name, _, _)| name == "orbit.task.artifact.put")
        .expect("artifact frame");
    assert!(artifact_frame.get("source_path").is_none());
    assert!(artifact_frame.get("artifacts").is_some());
    assert_eq!(artifact_context.workspace_id.as_deref(), Some("ws_canary"));
    let wire_json = serde_json::to_string(&*wire_calls).expect("wire JSON");
    assert!(!wire_json.contains(&client.path().to_string_lossy().to_string()));
    assert!(!wire_json.contains(&owner.path().to_string_lossy().to_string()));
    drop(wire_calls);

    let response_json = serde_json::to_string(&responses).expect("response JSON");
    assert!(!response_json.contains(&client.path().to_string_lossy().to_string()));
    assert!(!response_json.contains(&owner.path().to_string_lossy().to_string()));

    let task_store = owner.path().join("tasks/index.sqlite");
    let task_connection = rusqlite::Connection::open(task_store).expect("owner task store");
    let task_count: i64 = task_connection
        .query_row(
            "SELECT COUNT(*) FROM task_bundle_index WHERE workspace_id = 'ws_canary'",
            [],
            |row| row.get(0),
        )
        .expect("owner task count");
    assert_eq!(task_count, 1);
    assert!(!client.path().join("tasks").exists());
    assert!(!client.path().join("frictions").exists());

    let audit_connection =
        rusqlite::Connection::open(owner.path().join("orbit.db")).expect("owner audit store");
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
        assert_eq!(row.3.as_deref(), Some("hm_client"));
        assert_eq!(row.4.as_deref(), Some("client"));
        assert_eq!(row.5.as_deref(), Some("hm_owner"));
        assert_eq!(row.6.as_deref(), Some("owner"));
        assert_eq!(row.7.as_deref(), Some("ssh-mcp"));
        assert_eq!(
            row.8.as_deref(),
            Some(match capability {
                McpCapability::Agent => "[\"agent\"]",
                McpCapability::Operator => "[\"operator\"]",
                McpCapability::Runner => unreachable!("the bridge grants no runner session"),
            })
        );
        assert_eq!(row.9.as_deref(), Some("session-canary"));
        assert_eq!(row.10.as_deref(), Some(call_id));
        assert_eq!(row.11.as_deref(), Some(name));
    }
    assert_eq!(factory.connects.lock().expect("connects").len(), 1);
}
