use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use orbit_common::types::{
    HUB_KNOWLEDGE_ALLOCATION_METHOD_V1, HUB_KNOWLEDGE_ALLOCATION_SCHEMA_VERSION,
    HubKnowledgeAllocationRequestV1, HubKnowledgeAllocationV1, KnowledgeIdKind, McpCapability,
    McpTransport, OrbitError, SpokeRegistrationRequestV1, SpokeRegistrationResultV1,
    ToolSessionContext, Workspace, WorkspaceRegistry, WorkspaceStatus,
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
use super::super::host::canonical_mcp_tool_definitions;
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

fn schema_digests() -> BTreeMap<McpCapability, String> {
    let definitions = canonical_mcp_tool_definitions().expect("definitions");
    BTreeMap::from([(
        McpCapability::Agent,
        hub_schema_digest(&definitions, McpCapability::Agent).expect("schema digest"),
    )])
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
}

impl HubPeerFactory for WireFactory {
    fn connect<'a>(
        &'a self,
        spec: &'a HubSpawnSpec,
        limits: HubLinkLimits,
    ) -> BoxFuture<'a, Result<Box<dyn HubPeer>, OrbitError>> {
        let hub_root = self.hub_root.clone();
        let spec = spec.clone();
        let allocations = Arc::clone(&self.allocations);
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
                hub_server_composition(host),
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
                allocations,
                request_timeout: limits.request,
                close_timeout: limits.close,
            }) as Box<dyn HubPeer>)
        })
    }
}

struct WirePeer {
    client: OrbitMcpClient,
    server_task: tokio::task::JoinHandle<()>,
    allocations: Arc<Mutex<Vec<(HubKnowledgeAllocationRequestV1, ToolSessionContext)>>>,
    request_timeout: Duration,
    close_timeout: Duration,
}

impl HubPeer for WirePeer {
    fn is_closed(&self) -> bool {
        self.client.is_closed()
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
    let pool = HubLinkPool::with_factory(
        "hub-alias".to_string(),
        "hm_hub".to_string(),
        schema_digests(),
        Arc::new(WireFactory {
            hub_root: hub_root.path().to_path_buf(),
            allocations: Arc::clone(&allocations),
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
fn outcome_unknown_is_not_replayed_and_is_resolved_by_internal_lookup() {
    let hub_root = tempfile::tempdir().expect("hub root");
    setup_hub(hub_root.path());
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
    let error = pool
        .allocate_knowledge_id(
            McpCapability::Agent,
            request("ws_alpha", KnowledgeIdKind::Learning),
            context("ws_alpha", "mcall-lost-allocation"),
        )
        .expect_err("response loss")
        .to_string();
    assert!(error.contains("outcome unknown"), "{error}");
    assert_eq!(*calls.lock().expect("calls"), 1, "link replayed request");
    drop(pool);

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
