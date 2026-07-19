use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use orbit_common::types::{
    AuditEventStatus, HUB_KNOWLEDGE_ALLOCATION_METHOD_V1, HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION,
    HubKnowledgeAllocationRequestV1, HubKnowledgeAllocationV1, KnowledgeIdKind, McpCapability,
    McpTransport, OrbitError, SpokeRegistrationRequestV1, SpokeRegistrationResultV1,
    ToolSessionContext, Workspace, WorkspaceCheckout, WorkspaceCheckoutRole, WorkspaceRegistry,
    WorkspaceStatus,
};
use orbit_mcp::{McpCustomRequestError, McpCustomRequestHandler, McpHost, OrbitToolServer};
use rmcp::ServiceExt;
use serde_json::json;

use crate::persistence::KnowledgeWorkspaceInventory;
use crate::{
    HOST_IDENTITY_SCHEMA_VERSION, HostIdentity, HostMode, HubKnowledgeSequenceService,
    load_host_identity,
};

use super::super::contract::hub_schema_digest;
use super::super::host::{BrokerMcpHost, canonical_mcp_tool_definitions};
use super::super::hub::HubMcpHost;
use super::super::hub_client::OrbitMcpClient;
use super::super::hub_link::{
    BoxFuture, HubClock, HubLinkLimits, HubLinkPool, HubPeer, HubPeerFactory, HubSpawnSpec,
    MonotonicClock,
};
use super::super::hub_server_composition;
use super::super::transport::PrivateHubRequestHandler;

fn write_identity(root: &Path, mode: &str, machine_id: &str, host_id: &str) {
    std::fs::write(
        root.join("host.toml"),
        format!(
            "schema_version = 1\nmachine_id = \"{machine_id}\"\nhost_id = \"{host_id}\"\nmode = \"{mode}\"\n"
        ),
    )
    .expect("host identity");
}

fn setup_hub(root: &Path) {
    write_identity(root, "hub", "hm_hub", "hub");
    let identity = load_host_identity(root).expect("hub identity");
    let registry = crate::host_registry_service_at(root).expect("registry service");
    registry
        .register_hub_identity(&identity, BTreeSet::new())
        .expect("register hub");
    registry
        .register_identity(
            &HostIdentity {
                schema_version: HOST_IDENTITY_SCHEMA_VERSION,
                machine_id: "hm_spoke".to_string(),
                host_id: "spoke".to_string(),
                mode: HostMode::Spoke,
            },
            BTreeSet::new(),
        )
        .expect("register spoke");
    crate::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces: ["ws_alpha", "ws_beta"]
                .into_iter()
                .map(|workspace_id| Workspace {
                    id: workspace_id.to_string(),
                    name: workspace_id.to_string(),
                    owner_machine_id: Some("hm_spoke".to_string()),
                    git_remote: None,
                    ship_mode: None,
                    base_branch: "agent-main".to_string(),
                    status: WorkspaceStatus::Active,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                })
                .collect(),
            ..WorkspaceRegistry::default()
        },
        &crate::workspace_registry::registry_path_for(root),
    )
    .expect("workspace registry");
    HubKnowledgeSequenceService::at(root)
        .expect("allocator service")
        .activate(
            ["ws_alpha", "ws_beta"]
                .into_iter()
                .map(|workspace_id| KnowledgeWorkspaceInventory {
                    workspace_id: workspace_id.to_string(),
                    ids: Vec::new(),
                })
                .collect(),
        )
        .expect("activate allocator");
}

fn request(workspace_id: &str, kind: KnowledgeIdKind) -> HubKnowledgeAllocationRequestV1 {
    HubKnowledgeAllocationRequestV1 {
        schema_version: HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION,
        workspace_id: workspace_id.to_string(),
        kind,
        model: Some("gpt-test".to_string()),
    }
}

fn context(workspace_id: &str, call_id: &str) -> ToolSessionContext {
    ToolSessionContext {
        workspace: Some(workspace_id.to_string()),
        workspace_id: Some(workspace_id.to_string()),
        caller_machine_id: Some("hm_spoke".to_string()),
        caller_host_id: Some("spoke".to_string()),
        process_machine_id: None,
        process_host_id: None,
        transport: Some(McpTransport::SshMcp),
        effective_capabilities: BTreeSet::from([McpCapability::Agent]),
        origin_session_id: Some("spoke-session".to_string()),
        mcp_call_id: Some(call_id.to_string()),
        leased_run: None,
    }
}

fn local_context(
    selector: &str,
    call_id: &str,
    machine_id: &str,
    host_id: &str,
) -> ToolSessionContext {
    let mut context = ToolSessionContext::trusted_local(
        None,
        Some(machine_id.to_string()),
        Some(host_id.to_string()),
    );
    context.workspace = Some(selector.to_string());
    context.origin_session_id = Some(format!("session-{machine_id}"));
    context.mcp_call_id = Some(call_id.to_string());
    context
}

fn schema_digests() -> BTreeMap<McpCapability, String> {
    let definitions = canonical_mcp_tool_definitions().expect("definitions");
    BTreeMap::from([(
        McpCapability::Agent,
        hub_schema_digest(&definitions, McpCapability::Agent).expect("schema digest"),
    )])
}

fn workspace_record(workspace_id: &str, owner_machine_id: &str) -> Workspace {
    Workspace {
        id: workspace_id.to_string(),
        name: workspace_id.to_string(),
        owner_machine_id: Some(owner_machine_id.to_string()),
        git_remote: None,
        ship_mode: Some("local".to_string()),
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn init_owner_checkout(base: &Path, workspace_id: &str) -> WorkspaceCheckout {
    let repo_root = base.join(format!("{workspace_id}-checkout"));
    std::fs::create_dir_all(&repo_root).expect("checkout root");
    let status = Command::new("git")
        .args(["init", "-b", "agent-main"])
        .arg(&repo_root)
        .status()
        .expect("git init");
    assert!(status.success());
    for (key, value) in [
        ("user.name", "Orbit Test"),
        ("user.email", "orbit@example.invalid"),
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&repo_root)
                .args(["config", key, value])
                .status()
                .expect("git config")
                .success()
        );
    }
    std::fs::write(repo_root.join("README.md"), b"fixture\n").expect("README");
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&repo_root)
            .args(["add", "README.md"])
            .status()
            .expect("git add")
            .success()
    );
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&repo_root)
            .args(["commit", "-m", "fixture"])
            .status()
            .expect("git commit")
            .success()
    );
    let orbit_dir = repo_root.join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("Orbit root");
    std::fs::write(
        orbit_dir.join("config.yaml"),
        format!("schema_version: 1\nworkspace_id: {workspace_id}\n"),
    )
    .expect("workspace identity");
    WorkspaceCheckout::owner(workspace_id.to_string(), repo_root, orbit_dir)
}

fn add_linked_worktree(checkout: &WorkspaceCheckout, destination: &Path) {
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&checkout.repo_root)
            .args(["worktree", "add", "-b", "selected-owner"])
            .arg(destination)
            .status()
            .expect("git worktree add")
            .success()
    );
}

fn save_workspace_registry(
    global_root: &Path,
    workspace: Workspace,
    checkout: Option<WorkspaceCheckout>,
) {
    crate::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![workspace],
            checkouts: checkout.into_iter().collect(),
            ..WorkspaceRegistry::default()
        },
        &crate::workspace_registry::registry_path_for(global_root),
    )
    .expect("workspace registry");
}

fn add_hub_owned_workspace(hub_root: &Path, workspace_id: &str, checkout: WorkspaceCheckout) {
    let registry_path = crate::workspace_registry::registry_path_for(hub_root);
    let mut registry =
        crate::workspace_registry::load_registry_from(&registry_path).expect("load hub registry");
    registry
        .workspaces
        .push(workspace_record(workspace_id, "hm_hub"));
    registry.checkouts.push(checkout);
    crate::workspace_registry::save_registry_to(&registry, &registry_path)
        .expect("save hub registry");
    HubKnowledgeSequenceService::at(hub_root)
        .expect("hub allocator")
        .reconcile_workspace(KnowledgeWorkspaceInventory {
            workspace_id: workspace_id.to_string(),
            ids: Vec::new(),
        })
        .expect("reconcile hub workspace");
}

fn limits() -> HubLinkLimits {
    HubLinkLimits {
        queue_capacity: 4,
        initialize: Duration::from_secs(3),
        request: Duration::from_secs(3),
        idle: Duration::from_secs(30),
        idle_poll: Duration::from_millis(10),
        close: Duration::from_secs(1),
    }
}

fn wire_pool(
    hub_root: &Path,
    allocations: Arc<Mutex<Vec<(HubKnowledgeAllocationRequestV1, ToolSessionContext)>>>,
    tool_calls: Arc<Mutex<Vec<(String, serde_json::Value, ToolSessionContext)>>>,
    connects: Arc<Mutex<usize>>,
) -> HubLinkPool {
    HubLinkPool::with_factory(
        "hub-alias".to_string(),
        "hm_hub".to_string(),
        schema_digests(),
        Arc::new(WireFactory {
            hub_root: hub_root.to_path_buf(),
            allocations,
            tool_calls,
            connects,
        }),
        limits(),
        Arc::new(MonotonicClock::default()) as Arc<dyn HubClock>,
    )
    .expect("link pool")
}

fn snapshot_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut entries = std::fs::read_dir(directory)
            .expect("snapshot directory")
            .map(|entry| entry.expect("snapshot entry"))
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            let kind = entry.file_type().expect("file type");
            if kind.is_dir() {
                visit(root, &path, files);
            } else if kind.is_file() {
                files.insert(
                    path.strip_prefix(root)
                        .expect("relative path")
                        .to_path_buf(),
                    std::fs::read(path).expect("file bytes"),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

struct WireFactory {
    hub_root: PathBuf,
    allocations: Arc<Mutex<Vec<(HubKnowledgeAllocationRequestV1, ToolSessionContext)>>>,
    tool_calls: Arc<Mutex<Vec<(String, serde_json::Value, ToolSessionContext)>>>,
    connects: Arc<Mutex<usize>>,
}

impl HubPeerFactory for WireFactory {
    fn connect<'a>(
        &'a self,
        spec: &'a HubSpawnSpec,
        limits: HubLinkLimits,
    ) -> BoxFuture<'a, Result<Box<dyn HubPeer>, OrbitError>> {
        *self.connects.lock().expect("connect count") += 1;
        let hub_root = self.hub_root.clone();
        let spec = spec.clone();
        let allocations = Arc::clone(&self.allocations);
        let tool_calls = Arc::clone(&self.tool_calls);
        Box::pin(async move {
            let host = Arc::new(HubMcpHost::new(hub_root, spec.capability)?);
            let mut trusted = ToolSessionContext::trusted_local(
                None,
                Some(host.identity().machine_id.clone()),
                Some(host.identity().host_id.clone()),
            );
            trusted.effective_capabilities = BTreeSet::from([spec.capability]);
            let server = OrbitToolServer::new_with_context_and_composition(
                Arc::clone(&host) as Arc<dyn McpHost>,
                trusted,
                hub_server_composition(Arc::clone(&host)),
            );
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
            Ok(Box::new(WirePeer {
                client,
                server_task,
                host,
                allocations,
                tool_calls,
                request_timeout: limits.request,
                close_timeout: limits.close,
            }) as Box<dyn HubPeer>)
        })
    }
}

struct WirePeer {
    client: OrbitMcpClient,
    server_task: tokio::task::JoinHandle<()>,
    host: Arc<HubMcpHost>,
    allocations: Arc<Mutex<Vec<(HubKnowledgeAllocationRequestV1, ToolSessionContext)>>>,
    tool_calls: Arc<Mutex<Vec<(String, serde_json::Value, ToolSessionContext)>>>,
    request_timeout: Duration,
    close_timeout: Duration,
}

impl HubPeer for WirePeer {
    fn is_closed(&self) -> bool {
        self.client.is_closed()
    }

    fn call<'a>(
        &'a mut self,
        name: &'a str,
        input: serde_json::Value,
        context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<serde_json::Value, OrbitError>> {
        self.tool_calls.lock().expect("tool calls").push((
            name.to_string(),
            input.clone(),
            context.clone(),
        ));
        let result = self
            .host
            .compose_preallocated_knowledge_add(name, input, context.clone());
        Box::pin(async move { result })
    }

    fn register_spoke<'a>(
        &'a mut self,
        _request: &'a SpokeRegistrationRequestV1,
        _context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<SpokeRegistrationResultV1, OrbitError>> {
        Box::pin(async {
            Err(OrbitError::InvalidInput(
                "unexpected registration".to_string(),
            ))
        })
    }

    fn allocate_knowledge_id<'a>(
        &'a mut self,
        request: &'a HubKnowledgeAllocationRequestV1,
        context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<HubKnowledgeAllocationV1, OrbitError>> {
        self.allocations
            .lock()
            .expect("allocations")
            .push((request.clone(), context.clone()));
        Box::pin(async move {
            self.client
                .allocate_knowledge_id(request, context, self.request_timeout)
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

struct CommitThenLoseFactory {
    hub_root: PathBuf,
    calls: Arc<Mutex<usize>>,
}

impl HubPeerFactory for CommitThenLoseFactory {
    fn connect<'a>(
        &'a self,
        spec: &'a HubSpawnSpec,
        _limits: HubLinkLimits,
    ) -> BoxFuture<'a, Result<Box<dyn HubPeer>, OrbitError>> {
        let host = HubMcpHost::new(self.hub_root.clone(), spec.capability);
        let calls = Arc::clone(&self.calls);
        Box::pin(async move {
            Ok(Box::new(CommitThenLosePeer { host: host?, calls }) as Box<dyn HubPeer>)
        })
    }
}

struct CommitThenLosePeer {
    host: HubMcpHost,
    calls: Arc<Mutex<usize>>,
}

impl HubPeer for CommitThenLosePeer {
    fn is_closed(&self) -> bool {
        false
    }

    fn call<'a>(
        &'a mut self,
        _name: &'a str,
        _input: serde_json::Value,
        _context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<serde_json::Value, OrbitError>> {
        Box::pin(async { Err(OrbitError::InvalidInput("unexpected tool call".to_string())) })
    }

    fn register_spoke<'a>(
        &'a mut self,
        _request: &'a SpokeRegistrationRequestV1,
        _context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<SpokeRegistrationResultV1, OrbitError>> {
        Box::pin(async {
            Err(OrbitError::InvalidInput(
                "unexpected registration".to_string(),
            ))
        })
    }

    fn allocate_knowledge_id<'a>(
        &'a mut self,
        request: &'a HubKnowledgeAllocationRequestV1,
        context: &'a ToolSessionContext,
    ) -> BoxFuture<'a, Result<HubKnowledgeAllocationV1, OrbitError>> {
        *self.calls.lock().expect("call count") += 1;
        let committed = self
            .host
            .private_allocate_knowledge_id(request.clone(), context.clone());
        let call_id = context.mcp_call_id.clone().unwrap_or_default();
        Box::pin(async move {
            committed?;
            Err(OrbitError::OutcomeUnknown {
                mcp_call_id: call_id,
                message: "injected response loss after commit".to_string(),
            })
        })
    }

    fn close<'a>(&'a mut self) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

#[test]
fn connector_private_two_root_allocation_is_path_free_checkoutless_and_not_advertised() {
    let hub_root = tempfile::tempdir().expect("hub root");
    let spoke_root = tempfile::tempdir().expect("spoke root");
    setup_hub(hub_root.path());
    write_identity(spoke_root.path(), "spoke", "hm_spoke", "spoke");
    std::fs::write(spoke_root.path().join("marker"), b"unchanged").expect("spoke marker");
    let spoke_before = snapshot_files(spoke_root.path());
    let allocations = Arc::new(Mutex::new(Vec::new()));
    let tool_calls = Arc::new(Mutex::new(Vec::new()));
    let connects = Arc::new(Mutex::new(0));
    let pool = HubLinkPool::with_factory(
        "hub-alias".to_string(),
        "hm_hub".to_string(),
        schema_digests(),
        Arc::new(WireFactory {
            hub_root: hub_root.path().to_path_buf(),
            allocations: Arc::clone(&allocations),
            tool_calls: Arc::clone(&tool_calls),
            connects: Arc::clone(&connects),
        }),
        limits(),
        Arc::new(MonotonicClock::default()) as Arc<dyn HubClock>,
    )
    .expect("link pool");
    let alpha = pool
        .allocate_knowledge_id(
            McpCapability::Agent,
            request("ws_alpha", KnowledgeIdKind::Adr),
            context("ws_alpha", "mcall-wire-alpha"),
        )
        .expect("alpha wire allocation");
    let beta = pool
        .allocate_knowledge_id(
            McpCapability::Agent,
            request("ws_beta", KnowledgeIdKind::Adr),
            context("ws_beta", "mcall-wire-beta"),
        )
        .expect("beta wire allocation");
    assert_eq!(alpha.id, "ADR-0001");
    assert_eq!(beta.id, "ADR-0002");
    drop(pool);
    assert_eq!(snapshot_files(spoke_root.path()), spoke_before);
    let captured = allocations.lock().expect("allocations");
    assert_eq!(captured.len(), 2, "connector replayed allocation");
    assert!(tool_calls.lock().expect("tool calls").is_empty());
    assert_eq!(*connects.lock().expect("connects"), 1);
    assert_eq!(captured[0].0.workspace_id, "ws_alpha");
    assert_eq!(captured[1].0.workspace_id, "ws_beta");
    let encoded = serde_json::to_value(&captured[0].0).expect("request json");
    assert_eq!(
        encoded
            .as_object()
            .expect("request object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "kind".to_string(),
            "model".to_string(),
            "schema_version".to_string(),
            "workspace_id".to_string(),
        ])
    );
    assert!(
        !encoded
            .to_string()
            .contains(hub_root.path().to_string_lossy().as_ref())
    );
    assert!(
        !encoded
            .to_string()
            .contains(spoke_root.path().to_string_lossy().as_ref())
    );
    let encoded_result = serde_json::to_value(&alpha).expect("allocation result json");
    assert_eq!(
        encoded_result
            .as_object()
            .expect("allocation result object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "allocated_at".to_string(),
            "id".to_string(),
            "kind".to_string(),
            "mcp_call_id".to_string(),
            "schema_version".to_string(),
            "sequence".to_string(),
            "workspace_id".to_string(),
        ])
    );
    assert!(
        !encoded_result
            .to_string()
            .contains(hub_root.path().to_string_lossy().as_ref())
    );
    assert!(
        !encoded_result
            .to_string()
            .contains(spoke_root.path().to_string_lossy().as_ref())
    );

    let hub =
        HubMcpHost::new(hub_root.path().to_path_buf(), McpCapability::Agent).expect("hub host");
    let names = hub
        .list_mcp_tool_definitions()
        .expect("hub definitions")
        .into_iter()
        .map(|definition| definition.schema.name)
        .collect::<BTreeSet<_>>();
    assert!(!names.contains(HUB_KNOWLEDGE_ALLOCATION_METHOD_V1));
    assert!(!names.contains("orbit.knowledge.allocation.lookup"));
}

#[test]
fn local_owner_allocates_once_then_finalizes_both_kinds_in_exact_selected_worktree() {
    let hub_root = tempfile::tempdir().expect("hub root");
    let spoke_root = tempfile::tempdir().expect("spoke root");
    setup_hub(hub_root.path());
    write_identity(spoke_root.path(), "spoke", "hm_spoke", "spoke");
    let workspace = workspace_record("ws_alpha", "hm_spoke");
    let checkout = init_owner_checkout(spoke_root.path(), "ws_alpha");
    let selected = spoke_root.path().join("selected-worktree");
    add_linked_worktree(&checkout, &selected);
    save_workspace_registry(spoke_root.path(), workspace.clone(), Some(checkout.clone()));

    let allocations = Arc::new(Mutex::new(Vec::new()));
    let tool_calls = Arc::new(Mutex::new(Vec::new()));
    let connects = Arc::new(Mutex::new(0));
    let pool = wire_pool(
        hub_root.path(),
        Arc::clone(&allocations),
        Arc::clone(&tool_calls),
        Arc::clone(&connects),
    );
    let broker = BrokerMcpHost::new_with_hub_link(spoke_root.path().to_path_buf(), pool);
    let selector = selected.to_string_lossy().into_owned();

    let adr = broker
        .preallocated_knowledge_call(
            "orbit.adr.add",
            json!({
                "workspace": selector,
                "title": "Global ID",
                "owner": "codex",
                "body": "Body",
                "model": "codex"
            }),
            local_context(&selector, "mcall-owner-adr", "hm_spoke", "spoke"),
        )
        .expect("ADR add");
    let learning = broker
        .preallocated_knowledge_call(
            "orbit.learning.add",
            json!({
                "workspace": selector,
                "summary": "Global learning ID",
                "scope": {"paths": ["src/**"], "tags": ["global"]},
                "body": "Body",
                "model": "codex"
            }),
            local_context(&selector, "mcall-owner-learning", "hm_spoke", "spoke"),
        )
        .expect("learning add");

    assert_eq!(adr["id"], "ADR-0001");
    assert_eq!(learning["id"], "L-0001");
    assert!(
        selected
            .join(".orbit/adrs/proposed/ADR-0001/adr.yaml")
            .is_file()
    );
    assert!(
        selected
            .join(".orbit/learnings/L-0001/learning.yaml")
            .is_file()
    );
    assert!(
        !checkout
            .repo_root
            .join(".orbit/adrs/proposed/ADR-0001")
            .exists()
    );
    assert!(!checkout.repo_root.join(".orbit/learnings/L-0001").exists());

    drop(broker);
    let captured = allocations.lock().expect("allocations");
    assert_eq!(captured.len(), 2);
    assert!(tool_calls.lock().expect("tool calls").is_empty());
    assert_eq!(*connects.lock().expect("connects"), 1);
    for (request, context) in captured.iter() {
        assert_eq!(request.workspace_id, "ws_alpha");
        assert_eq!(
            request.kind.as_str(),
            if request.kind == KnowledgeIdKind::Adr {
                "adr"
            } else {
                "learning"
            }
        );
        assert_eq!(context.workspace_id.as_deref(), Some("ws_alpha"));
        let encoded = serde_json::to_string(request).expect("request JSON");
        assert!(!encoded.contains(selected.to_string_lossy().as_ref()));
        assert!(!encoded.contains(checkout.repo_root.to_string_lossy().as_ref()));
    }

    let runtime = crate::runtime::RemoteRuntimeFactory::open_registered_checkout(
        spoke_root.path(),
        &workspace,
        &checkout,
    )
    .expect("reopen owner runtime");
    let audits = runtime
        .list_audit_events_with_kind(
            None,
            None,
            Some("knowledge_finalization".to_string()),
            None,
            None,
            10,
        )
        .expect("owner audits");
    assert_eq!(audits.len(), 2);
    for audit in audits {
        assert_eq!(audit.workspace_id.as_deref(), Some("ws_alpha"));
        assert_eq!(audit.caller_machine_id.as_deref(), Some("hm_spoke"));
        assert_eq!(audit.process_machine_id.as_deref(), Some("hm_spoke"));
        let (expected_kind, expected_id) = match audit.mcp_call_id.as_deref() {
            Some("mcall-owner-adr") => ("adr", "ADR-0001"),
            Some("mcall-owner-learning") => ("learning", "L-0001"),
            correlation => panic!("unexpected correlation: {correlation:?}"),
        };
        assert_eq!(audit.status, AuditEventStatus::Success);
        assert_eq!(audit.target_id.as_deref(), Some(expected_id));
        let arguments: serde_json::Value = serde_json::from_str(
            audit
                .arguments_json
                .as_deref()
                .expect("finalization arguments"),
        )
        .expect("finalization arguments JSON");
        assert_eq!(arguments["workspace_id"], "ws_alpha");
        assert_eq!(arguments["kind"], expected_kind);
        assert_eq!(arguments["allocated_id"], expected_id);
        assert_eq!(
            arguments["mcp_call_id"],
            audit.mcp_call_id.as_deref().expect("correlation")
        );
    }
}

#[test]
fn supplied_id_collisions_preserve_owner_artifacts_and_leave_consumed_hub_gaps() {
    let hub_root = tempfile::tempdir().expect("hub root");
    let spoke_root = tempfile::tempdir().expect("spoke root");
    setup_hub(hub_root.path());
    write_identity(spoke_root.path(), "spoke", "hm_spoke", "spoke");
    let checkout = init_owner_checkout(spoke_root.path(), "ws_alpha");
    let workspace = workspace_record("ws_alpha", "hm_spoke");
    save_workspace_registry(spoke_root.path(), workspace.clone(), Some(checkout.clone()));

    let allocations = Arc::new(Mutex::new(Vec::new()));
    let tool_calls = Arc::new(Mutex::new(Vec::new()));
    let connects = Arc::new(Mutex::new(0));
    let broker = BrokerMcpHost::new_with_hub_link(
        spoke_root.path().to_path_buf(),
        wire_pool(
            hub_root.path(),
            Arc::clone(&allocations),
            Arc::clone(&tool_calls),
            Arc::clone(&connects),
        ),
    );
    let selector = checkout.repo_root.to_string_lossy().into_owned();

    let compatibility_runtime = crate::runtime::RemoteRuntimeFactory::open_registered_checkout(
        spoke_root.path(),
        &workspace,
        &checkout,
    )
    .expect("compatibility runtime");
    let existing_adr = compatibility_runtime
        .run_tool(
            "orbit.adr.add",
            json!({
                "title": "Compatibility ADR",
                "owner": "codex",
                "body": "original ADR body"
            }),
        )
        .expect("compatibility ADR");
    let existing_learning = compatibility_runtime
        .run_tool(
            "orbit.learning.add",
            json!({
                "summary": "Compatibility learning",
                "scope": {},
                "body": "original learning body"
            }),
        )
        .expect("compatibility learning");
    assert_eq!(existing_adr["id"], "ADR-0001");
    assert_eq!(existing_learning["id"], "L-0001");
    let adr_dir = checkout.repo_root.join(".orbit/adrs/proposed/ADR-0001");
    let learning_dir = checkout.repo_root.join(".orbit/learnings/L-0001");
    let adr_before = snapshot_files(&adr_dir);
    let learning_before = snapshot_files(&learning_dir);

    let adr_error = broker
        .preallocated_knowledge_call(
            "orbit.adr.add",
            json!({
                "workspace": selector,
                "title": "Must not overwrite",
                "owner": "codex",
                "body": "replacement",
                "model": "codex"
            }),
            local_context(&selector, "mcall-collision-adr", "hm_spoke", "spoke"),
        )
        .expect_err("ADR collision")
        .to_string();
    assert!(adr_error.contains("ADR-0001"), "{adr_error}");
    assert!(adr_error.contains("remains consumed"), "{adr_error}");
    assert_eq!(snapshot_files(&adr_dir), adr_before);

    let learning_error = broker
        .preallocated_knowledge_call(
            "orbit.learning.add",
            json!({
                "workspace": selector,
                "summary": "Must not overwrite",
                "scope": {},
                "body": "replacement",
                "model": "codex"
            }),
            local_context(&selector, "mcall-collision-learning", "hm_spoke", "spoke"),
        )
        .expect_err("learning collision")
        .to_string();
    assert!(learning_error.contains("L-0001"), "{learning_error}");
    assert!(
        learning_error.contains("remains consumed"),
        "{learning_error}"
    );
    assert_eq!(snapshot_files(&learning_dir), learning_before);

    let next_adr = broker
        .preallocated_knowledge_call(
            "orbit.adr.add",
            json!({
                "workspace": selector,
                "title": "After the gap",
                "owner": "codex",
                "body": "new body",
                "model": "codex"
            }),
            local_context(&selector, "mcall-next-adr", "hm_spoke", "spoke"),
        )
        .expect("next ADR");
    let next_learning = broker
        .preallocated_knowledge_call(
            "orbit.learning.add",
            json!({
                "workspace": selector,
                "summary": "After the gap",
                "scope": {},
                "body": "new body",
                "model": "codex"
            }),
            local_context(&selector, "mcall-next-learning", "hm_spoke", "spoke"),
        )
        .expect("next learning");
    assert_eq!(next_adr["id"], "ADR-0002");
    assert_eq!(next_learning["id"], "L-0002");
    drop(broker);

    let service = HubKnowledgeSequenceService::at(hub_root.path()).expect("hub service");
    for (call_id, kind, id) in [
        ("mcall-collision-adr", KnowledgeIdKind::Adr, "ADR-0001"),
        (
            "mcall-collision-learning",
            KnowledgeIdKind::Learning,
            "L-0001",
        ),
        ("mcall-next-adr", KnowledgeIdKind::Adr, "ADR-0002"),
        ("mcall-next-learning", KnowledgeIdKind::Learning, "L-0002"),
    ] {
        let allocation = service
            .allocation_by_call(call_id)
            .expect("allocation lookup")
            .expect("consumed allocation");
        assert_eq!(allocation.workspace_id, "ws_alpha");
        assert_eq!(allocation.kind, kind);
        assert_eq!(allocation.id, id);
        assert_eq!(allocation.mcp_call_id, call_id);
    }
    assert_eq!(allocations.lock().expect("allocations").len(), 4);
    assert!(tool_calls.lock().expect("tool calls").is_empty());
    assert_eq!(*connects.lock().expect("connects"), 1);
}

#[test]
fn post_allocation_index_failures_clean_owner_state_and_consume_gap_ids() {
    let hub_root = tempfile::tempdir().expect("hub root");
    let spoke_root = tempfile::tempdir().expect("spoke root");
    setup_hub(hub_root.path());
    write_identity(spoke_root.path(), "spoke", "hm_spoke", "spoke");
    let checkout = init_owner_checkout(spoke_root.path(), "ws_alpha");
    let workspace = workspace_record("ws_alpha", "hm_spoke");
    save_workspace_registry(spoke_root.path(), workspace.clone(), Some(checkout.clone()));
    let runtime = crate::runtime::RemoteRuntimeFactory::open_registered_checkout(
        spoke_root.path(),
        &workspace,
        &checkout,
    )
    .expect("initialize owner stores");
    drop(runtime);
    let owner_index =
        rusqlite::Connection::open(spoke_root.path().join("orbit.db")).expect("owner index");
    owner_index
        .execute_batch(
            "CREATE TRIGGER fail_f2_adr_index
             BEFORE INSERT ON adrs WHEN NEW.id = 'ADR-0001'
             BEGIN SELECT RAISE(ABORT, 'injected F2 ADR index failure'); END;
             CREATE TRIGGER fail_f2_learning_index
             BEFORE INSERT ON learnings_index WHEN NEW.id = 'L-0001'
             BEGIN SELECT RAISE(ABORT, 'injected F2 learning index failure'); END;",
        )
        .expect("failure triggers");

    let allocations = Arc::new(Mutex::new(Vec::new()));
    let tool_calls = Arc::new(Mutex::new(Vec::new()));
    let connects = Arc::new(Mutex::new(0));
    let broker = BrokerMcpHost::new_with_hub_link(
        spoke_root.path().to_path_buf(),
        wire_pool(
            hub_root.path(),
            Arc::clone(&allocations),
            Arc::clone(&tool_calls),
            Arc::clone(&connects),
        ),
    );
    let selector = checkout.repo_root.to_string_lossy().into_owned();

    let adr_error = broker
        .preallocated_knowledge_call(
            "orbit.adr.add",
            json!({
                "workspace": selector,
                "title": "Injected failure",
                "owner": "codex",
                "body": "must be removed",
                "model": "codex"
            }),
            local_context(&selector, "mcall-fail-adr", "hm_spoke", "spoke"),
        )
        .expect_err("ADR index failure")
        .to_string();
    let learning_error = broker
        .preallocated_knowledge_call(
            "orbit.learning.add",
            json!({
                "workspace": selector,
                "summary": "Injected failure",
                "scope": {},
                "body": "must be removed",
                "model": "codex"
            }),
            local_context(&selector, "mcall-fail-learning", "hm_spoke", "spoke"),
        )
        .expect_err("learning index failure")
        .to_string();
    assert!(adr_error.contains("ADR-0001"), "{adr_error}");
    assert!(adr_error.contains("remains consumed"), "{adr_error}");
    assert!(learning_error.contains("L-0001"), "{learning_error}");
    assert!(
        learning_error.contains("remains consumed"),
        "{learning_error}"
    );
    assert!(
        !checkout
            .repo_root
            .join(".orbit/adrs/proposed/ADR-0001")
            .exists()
    );
    assert!(!checkout.repo_root.join(".orbit/learnings/L-0001").exists());
    let projection_db = rusqlite::Connection::open(checkout.orbit_dir.join("state/semantic.db"))
        .expect("projection database");
    let projection_count: i64 = projection_db
        .query_row(
            "SELECT COUNT(*) FROM id_allocations WHERE id IN ('ADR-0001', 'L-0001')",
            [],
            |row| row.get(0),
        )
        .expect("projection count");
    assert_eq!(projection_count, 0);
    let adr_index_count: i64 = owner_index
        .query_row(
            "SELECT COUNT(*) FROM adrs WHERE id = 'ADR-0001'",
            [],
            |row| row.get(0),
        )
        .expect("ADR index count");
    let learning_index_count: i64 = owner_index
        .query_row(
            "SELECT COUNT(*) FROM learnings_index WHERE id = 'L-0001'",
            [],
            |row| row.get(0),
        )
        .expect("learning index count");
    assert_eq!((adr_index_count, learning_index_count), (0, 0));

    owner_index
        .execute_batch("DROP TRIGGER fail_f2_adr_index; DROP TRIGGER fail_f2_learning_index;")
        .expect("remove failure triggers");
    let next_adr = broker
        .preallocated_knowledge_call(
            "orbit.adr.add",
            json!({
                "workspace": selector,
                "title": "After failure",
                "owner": "codex",
                "body": "persists",
                "model": "codex"
            }),
            local_context(&selector, "mcall-after-fail-adr", "hm_spoke", "spoke"),
        )
        .expect("ADR after gap");
    let next_learning = broker
        .preallocated_knowledge_call(
            "orbit.learning.add",
            json!({
                "workspace": selector,
                "summary": "After failure",
                "scope": {},
                "body": "persists",
                "model": "codex"
            }),
            local_context(&selector, "mcall-after-fail-learning", "hm_spoke", "spoke"),
        )
        .expect("learning after gap");
    assert_eq!(next_adr["id"], "ADR-0002");
    assert_eq!(next_learning["id"], "L-0002");
    drop(broker);

    let audit_runtime = crate::runtime::RemoteRuntimeFactory::open_registered_checkout(
        spoke_root.path(),
        &workspace,
        &checkout,
    )
    .expect("audit runtime");
    let audits = audit_runtime
        .list_audit_events_with_kind(
            None,
            None,
            Some("knowledge_finalization".to_string()),
            None,
            None,
            10,
        )
        .expect("finalization audits");
    for (call_id, kind, id) in [
        ("mcall-fail-adr", "adr", "ADR-0001"),
        ("mcall-fail-learning", "learning", "L-0001"),
    ] {
        let audit = audits
            .iter()
            .find(|audit| audit.mcp_call_id.as_deref() == Some(call_id))
            .expect("failure audit");
        assert_eq!(audit.status, AuditEventStatus::Failure);
        assert_eq!(audit.workspace_id.as_deref(), Some("ws_alpha"));
        assert_eq!(audit.caller_machine_id.as_deref(), Some("hm_spoke"));
        assert_eq!(audit.process_machine_id.as_deref(), Some("hm_spoke"));
        assert_eq!(audit.target_id.as_deref(), Some(id));
        let arguments: serde_json::Value = serde_json::from_str(
            audit
                .arguments_json
                .as_deref()
                .expect("finalization arguments"),
        )
        .expect("finalization arguments JSON");
        assert_eq!(arguments["workspace_id"], "ws_alpha");
        assert_eq!(arguments["kind"], kind);
        assert_eq!(arguments["allocated_id"], id);
        assert_eq!(arguments["mcp_call_id"], call_id);
    }

    let service = HubKnowledgeSequenceService::at(hub_root.path()).expect("hub service");
    for (call_id, expected_id) in [
        ("mcall-fail-adr", "ADR-0001"),
        ("mcall-fail-learning", "L-0001"),
        ("mcall-after-fail-adr", "ADR-0002"),
        ("mcall-after-fail-learning", "L-0002"),
    ] {
        assert_eq!(
            service
                .allocation_by_call(call_id)
                .expect("allocation lookup")
                .expect("consumed allocation")
                .id,
            expected_id
        );
    }
    assert_eq!(allocations.lock().expect("allocations").len(), 4);
    assert!(tool_calls.lock().expect("tool calls").is_empty());
    assert_eq!(*connects.lock().expect("connects"), 1);
}

#[test]
fn hub_owned_add_dispatches_once_and_finalizes_only_in_hub_checkout() {
    let hub_root = tempfile::tempdir().expect("hub root");
    let spoke_root = tempfile::tempdir().expect("spoke root");
    setup_hub(hub_root.path());
    let hub_checkout = init_owner_checkout(hub_root.path(), "ws_hub");
    add_hub_owned_workspace(hub_root.path(), "ws_hub", hub_checkout.clone());
    write_identity(spoke_root.path(), "spoke", "hm_spoke", "spoke");
    save_workspace_registry(
        spoke_root.path(),
        workspace_record("ws_hub", "hm_hub"),
        None,
    );

    let allocations = Arc::new(Mutex::new(Vec::new()));
    let tool_calls = Arc::new(Mutex::new(Vec::new()));
    let connects = Arc::new(Mutex::new(0));
    let pool = wire_pool(
        hub_root.path(),
        Arc::clone(&allocations),
        Arc::clone(&tool_calls),
        Arc::clone(&connects),
    );
    let broker = BrokerMcpHost::new_with_hub_link(spoke_root.path().to_path_buf(), pool);

    let compatibility_error = broker
        .call_tool(
            "orbit.adr.add",
            json!({
                "workspace": "ws_hub",
                "title": "Compatibility stays inactive",
                "owner": "codex",
                "body": "Body"
            }),
            local_context("ws_hub", "mcall-public-compat", "hm_spoke", "spoke"),
        )
        .expect_err("F2 gate remains inactive")
        .to_string();
    assert!(
        compatibility_error.contains("exact local checkout"),
        "{compatibility_error}"
    );
    assert!(
        HubKnowledgeSequenceService::at(hub_root.path())
            .expect("allocator")
            .allocation_by_call("mcall-public-compat")
            .expect("lookup")
            .is_none()
    );

    let response = broker
        .preallocated_knowledge_call(
            "orbit.adr.add",
            json!({
                "workspace": "ws_hub",
                "title": "Hub owned",
                "owner": "codex",
                "body": "Body",
                "model": "codex"
            }),
            local_context("ws_hub", "mcall-hub-owned", "hm_spoke", "spoke"),
        )
        .expect("hub-owned add");
    assert_eq!(response["id"], "ADR-0001");
    drop(broker);

    assert!(allocations.lock().expect("allocations").is_empty());
    let calls = tool_calls.lock().expect("tool calls");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "orbit.adr.add");
    assert_eq!(calls[0].1["workspace"], "ws_hub");
    assert_eq!(calls[0].2.workspace_id.as_deref(), Some("ws_hub"));
    assert!(
        !calls[0]
            .1
            .to_string()
            .contains(hub_checkout.repo_root.to_string_lossy().as_ref())
    );
    assert_eq!(*connects.lock().expect("connects"), 1);
    assert!(
        hub_checkout
            .repo_root
            .join(".orbit/adrs/proposed/ADR-0001/adr.yaml")
            .is_file()
    );
    assert!(!spoke_root.path().join(".orbit/adrs").exists());
}

#[test]
fn replica_or_other_spoke_owner_is_rejected_before_allocation_or_connection() {
    let hub_root = tempfile::tempdir().expect("hub root");
    let spoke_root = tempfile::tempdir().expect("spoke root");
    setup_hub(hub_root.path());
    write_identity(spoke_root.path(), "spoke", "hm_spoke", "spoke");
    let mut checkout = init_owner_checkout(spoke_root.path(), "ws_other");
    checkout.role = Some(WorkspaceCheckoutRole::Replica);
    checkout.owner_machine_id = Some("hm_other".to_string());
    save_workspace_registry(
        spoke_root.path(),
        workspace_record("ws_other", "hm_other"),
        Some(checkout.clone()),
    );
    let allocations = Arc::new(Mutex::new(Vec::new()));
    let tool_calls = Arc::new(Mutex::new(Vec::new()));
    let connects = Arc::new(Mutex::new(0));
    let pool = wire_pool(
        hub_root.path(),
        Arc::clone(&allocations),
        Arc::clone(&tool_calls),
        Arc::clone(&connects),
    );
    let broker = BrokerMcpHost::new_with_hub_link(spoke_root.path().to_path_buf(), pool);
    let selector = checkout.repo_root.to_string_lossy().into_owned();

    let error = broker
        .preallocated_knowledge_call(
            "orbit.learning.add",
            json!({
                "workspace": selector,
                "summary": "Must refuse",
                "scope": {},
                "model": "codex"
            }),
            local_context(&selector, "mcall-replica-denied", "hm_spoke", "spoke"),
        )
        .expect_err("replica refusal")
        .to_string();
    assert!(error.contains("hm_other"), "{error}");
    assert!(error.contains("replica"), "{error}");
    drop(broker);
    assert!(allocations.lock().expect("allocations").is_empty());
    assert!(tool_calls.lock().expect("tool calls").is_empty());
    assert_eq!(*connects.lock().expect("connects"), 0);
    assert!(!checkout.repo_root.join(".orbit/learnings/L-0001").exists());
}

#[test]
fn outcome_unknown_is_not_replayed_and_is_resolved_by_internal_lookup() {
    let hub_root = tempfile::tempdir().expect("hub root");
    let spoke_root = tempfile::tempdir().expect("spoke root");
    setup_hub(hub_root.path());
    write_identity(spoke_root.path(), "spoke", "hm_spoke", "spoke");
    let checkout = init_owner_checkout(spoke_root.path(), "ws_alpha");
    save_workspace_registry(
        spoke_root.path(),
        workspace_record("ws_alpha", "hm_spoke"),
        Some(checkout.clone()),
    );
    let calls = Arc::new(Mutex::new(0));
    let pool = HubLinkPool::with_factory(
        "hub-alias".to_string(),
        "hm_hub".to_string(),
        schema_digests(),
        Arc::new(CommitThenLoseFactory {
            hub_root: hub_root.path().to_path_buf(),
            calls: Arc::clone(&calls),
        }),
        limits(),
        Arc::new(MonotonicClock::default()) as Arc<dyn HubClock>,
    )
    .expect("link pool");
    let broker = BrokerMcpHost::new_with_hub_link(spoke_root.path().to_path_buf(), pool);
    let selector = checkout.repo_root.to_string_lossy().into_owned();
    let error = broker
        .preallocated_knowledge_call(
            "orbit.learning.add",
            json!({
                "workspace": selector,
                "summary": "Response will be lost",
                "scope": {},
                "model": "codex"
            }),
            local_context(&selector, "mcall-lost-allocation", "hm_spoke", "spoke"),
        )
        .expect_err("response loss")
        .to_string();
    assert!(error.contains("outcome unknown"), "{error}");
    assert_eq!(*calls.lock().expect("calls"), 1, "link replayed request");
    assert!(!checkout.repo_root.join(".orbit/learnings/L-0001").exists());
    let local_db = rusqlite::Connection::open(checkout.orbit_dir.join("state/semantic.db"))
        .expect("local semantic database");
    let projection_count: i64 = local_db
        .query_row(
            "SELECT COUNT(*) FROM id_allocations WHERE kind = 'learning' AND id = 'L-0001'",
            [],
            |row| row.get(0),
        )
        .expect("projection count");
    assert_eq!(projection_count, 0, "response loss finalized locally");
    drop(broker);

    let allocation = HubKnowledgeSequenceService::at(hub_root.path())
        .expect("service")
        .allocation_by_call("mcall-lost-allocation")
        .expect("lookup")
        .expect("committed allocation");
    assert_eq!(allocation.id, "L-0001");
}

#[test]
fn runner_private_allocation_is_denied_before_sequence_mutation() {
    let hub_root = tempfile::tempdir().expect("hub root");
    setup_hub(hub_root.path());
    let hub = HubMcpHost::new(hub_root.path().to_path_buf(), McpCapability::Runner)
        .expect("runner hub host");
    let mut runner_context = context("ws_alpha", "mcall-runner-denied");
    runner_context.effective_capabilities = BTreeSet::from([McpCapability::Runner]);
    let error = hub
        .private_allocate_knowledge_id(request("ws_alpha", KnowledgeIdKind::Adr), runner_context)
        .expect_err("runner allocation must fail")
        .to_string();
    assert!(error.contains("agent or operator"), "{error}");
    let state = crate::remote_store_at(hub_root.path())
        .expect("store")
        .knowledge_allocator_state()
        .expect("state");
    assert_eq!(state.adr_next_sequence, 1);
    assert!(
        HubKnowledgeSequenceService::at(hub_root.path())
            .expect("service")
            .allocation_by_call("mcall-runner-denied")
            .expect("lookup")
            .is_none()
    );
}

#[test]
fn multiple_effective_capabilities_are_denied_before_sequence_mutation() {
    let hub_root = tempfile::tempdir().expect("hub root");
    setup_hub(hub_root.path());
    let service = HubKnowledgeSequenceService::at(hub_root.path()).expect("service");
    let mut ambiguous_context = context("ws_alpha", "mcall-ambiguous-capability");
    ambiguous_context.effective_capabilities =
        BTreeSet::from([McpCapability::Agent, McpCapability::Operator]);
    let error = service
        .allocate(
            &request("ws_alpha", KnowledgeIdKind::Adr),
            &ambiguous_context,
        )
        .expect_err("multi-capability allocation must fail")
        .to_string();
    assert!(error.contains("exactly one"), "{error}");
    let state = crate::remote_store_at(hub_root.path())
        .expect("store")
        .knowledge_allocator_state()
        .expect("state");
    assert_eq!(state.adr_next_sequence, 1);
    assert!(
        service
            .allocation_by_call("mcall-ambiguous-capability")
            .expect("lookup")
            .is_none()
    );
}

#[test]
fn private_handler_rejects_unknown_kind_before_sequence_mutation() {
    let hub_root = tempfile::tempdir().expect("hub root");
    setup_hub(hub_root.path());
    let hub = Arc::new(
        HubMcpHost::new(hub_root.path().to_path_buf(), McpCapability::Agent)
            .expect("agent hub host"),
    );
    let handler = PrivateHubRequestHandler::new(hub);
    let error = handler
        .call(
            HUB_KNOWLEDGE_ALLOCATION_METHOD_V1,
            Some(json!({
                "schema_version": HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION,
                "workspace_id": "ws_alpha",
                "kind": "friction",
                "model": "gpt-test",
            })),
            context("ws_alpha", "mcall-invalid-kind"),
        )
        .expect_err("unknown kind must fail typed deserialization");
    assert!(
        matches!(error, McpCustomRequestError::InvalidParams { .. }),
        "{error:?}"
    );
    let state = crate::remote_store_at(hub_root.path())
        .expect("store")
        .knowledge_allocator_state()
        .expect("state");
    assert_eq!(state.adr_next_sequence, 1);
    assert_eq!(state.learning_next_sequence, 1);
    assert!(
        HubKnowledgeSequenceService::at(hub_root.path())
            .expect("service")
            .allocation_by_call("mcall-invalid-kind")
            .expect("lookup")
            .is_none()
    );
}
