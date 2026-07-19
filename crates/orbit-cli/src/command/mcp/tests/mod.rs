#![allow(missing_docs)]

// Content moved from inline #[cfg(test)] mod tests in mcp/mod.rs per ORB-00221.

mod e1;

use std::collections::{BTreeMap, BTreeSet};

use orbit_common::types::{
    LearningInjectionState, McpCapability, McpToolDefinition, McpToolPlacement, McpToolPolicy,
    McpToolPolicyError, McpToolScope, ToolSessionContext, mcp_advertised_tool_name,
    mcp_capability_placement_matrix, validate_mcp_tool_definitions,
};
use orbit_core::command::tool::ToolEntryPoint;
use orbit_core::{LearningSearchParams, OrbitError, OrbitRuntime};
use orbit_mcp::McpHost;
use serde::Deserialize;
use serde_json::{Value, json};

use super::host::{
    BrokerMcpHost, audited_mcp_call_with_session_context, canonical_mcp_tool_definitions,
    ensure_mcp_tool_exposed, is_mcp_tool_exposed, normalize_trusted_call_context,
    safe_mcp_tool_names,
};

#[test]
fn broker_checkoutless_task_call_uses_stable_id_and_one_trusted_audit() {
    use chrono::Utc;
    use orbit_common::types::{AuditEventStatus, Workspace, WorkspaceRegistry, WorkspaceStatus};

    let root = tempfile::tempdir().expect("global root");
    let workspace = Workspace {
        id: "ws_checkoutless".to_string(),
        name: "Checkoutless".to_string(),
        owner_machine_id: None,
        git_remote: None,
        ship_mode: None,
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    orbit_remote::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![workspace],
            ..Default::default()
        },
        &orbit_remote::workspace_registry::registry_path_for(root.path()),
    )
    .expect("workspace registry");
    orbit_core::runtime::HubCoordinationExecutor::register_workspace(
        root.path(),
        "ws_checkoutless",
        "checkoutless",
    )
    .expect("task workspace");

    let host = BrokerMcpHost::new(root.path().to_path_buf());
    let mut context = ToolSessionContext::trusted_local(
        Some("ws_checkoutless".to_string()),
        Some("hm_hub".to_string()),
        Some("hub".to_string()),
    );
    context.origin_session_id = Some("mcp-session-checkoutless".to_string());
    context.mcp_call_id = Some("mcall-checkoutless-add".to_string());
    let task = host
        .call_tool(
            "orbit.task.add",
            json!({
                "workspace": "ws_checkoutless",
                "title": "No checkout",
                "description": "Coordinate at hub",
                "model": "codex"
            }),
            context,
        )
        .expect("checkoutless task add");
    assert_eq!(task["title"], "No checkout");
    let id = task["id"].as_str().expect("task id");
    let mut update_context = ToolSessionContext::trusted_local(
        Some("ws_checkoutless".to_string()),
        Some("hm_hub".to_string()),
        Some("hub".to_string()),
    );
    update_context.mcp_call_id = Some("mcall-checkoutless-update".to_string());
    host.call_tool(
        "orbit.task.update",
        json!({"workspace": "ws_checkoutless", "id": id, "description": "Updated at hub", "model": "codex"}),
        update_context,
    )
    .expect("checkoutless task update");
    let mut show_context = ToolSessionContext::trusted_local(
        Some("ws_checkoutless".to_string()),
        Some("hm_hub".to_string()),
        Some("hub".to_string()),
    );
    show_context.mcp_call_id = Some("mcall-checkoutless-show".to_string());
    let shown = host
        .call_tool(
            "orbit.task.show",
            json!({"workspace": "ws_checkoutless", "id": id}),
            show_context,
        )
        .expect("checkoutless task show");
    assert_eq!(shown["description"], "Updated at hub");
    assert!(!root.path().join(".orbit").exists());

    let audit_path =
        orbit_core::config::resolved_audit_db_path(root.path(), root.path()).expect("audit path");
    let connection = rusqlite::Connection::open(audit_path).expect("audit store");
    let rows = connection
        .query_row(
            "SELECT COUNT(*), status, workspace_id, transport, mcp_call_id
             FROM audit_events WHERE tool_name = ?1 AND mcp_call_id = ?2",
            rusqlite::params!["orbit.task.add", "mcall-checkoutless-add"],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .expect("audit row");
    assert_eq!(rows.0, 1);
    assert_eq!(rows.1, AuditEventStatus::Success.to_string());
    assert_eq!(rows.2.as_deref(), Some("ws_checkoutless"));
    assert_eq!(rows.3.as_deref(), Some("local"));
    assert_eq!(rows.4.as_deref(), Some("mcall-checkoutless-add"));
}

#[test]
fn broker_with_context_requires_local_checkout_before_dispatch() {
    use chrono::Utc;
    use orbit_common::types::{Workspace, WorkspaceRegistry, WorkspaceStatus};

    let root = tempfile::tempdir().expect("global root");
    orbit_remote::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![Workspace {
                id: "ws_checkoutless".to_string(),
                name: "Checkoutless".to_string(),
                owner_machine_id: None,
                git_remote: None,
                ship_mode: None,
                base_branch: "agent-main".to_string(),
                status: WorkspaceStatus::Active,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            ..Default::default()
        },
        &orbit_remote::workspace_registry::registry_path_for(root.path()),
    )
    .expect("workspace registry");
    let host = BrokerMcpHost::new(root.path().to_path_buf());
    let error = host
        .call_tool(
            "orbit.task.show",
            json!({"workspace": "ws_checkoutless", "id": "ORB-00001", "with_context": true}),
            ToolSessionContext::trusted_local(
                Some("ws_checkoutless".to_string()),
                Some("hm_hub".to_string()),
                Some("hub".to_string()),
            ),
        )
        .expect_err("local-derived enrichment requires checkout");
    assert!(
        error
            .to_string()
            .contains("no single validated exact local checkout")
    );
    assert!(
        !root
            .path()
            .join("tasks/workspaces/ws_checkoutless/ORB-00001")
            .exists()
    );
}

#[test]
fn broker_spoke_with_context_fails_closed_before_local_resolution() {
    use chrono::Utc;
    use orbit_common::types::{Workspace, WorkspaceRegistry, WorkspaceStatus};
    use orbit_remote::{HostMode, NewHostIdentity, ensure_host_identity};

    let root = tempfile::tempdir().expect("global root");
    ensure_host_identity(root.path(), || {
        Ok(NewHostIdentity {
            host_id: "spoke".to_string(),
            mode: HostMode::Spoke,
        })
    })
    .expect("spoke identity");
    orbit_remote::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![Workspace {
                id: "ws_remote".to_string(),
                name: "Remote".to_string(),
                owner_machine_id: Some("hm_owner".to_string()),
                git_remote: None,
                ship_mode: None,
                base_branch: "agent-main".to_string(),
                status: WorkspaceStatus::Active,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            ..Default::default()
        },
        &orbit_remote::workspace_registry::registry_path_for(root.path()),
    )
    .expect("workspace registry");
    let host = BrokerMcpHost::new(root.path().to_path_buf());

    let error = host
        .call_tool(
            "orbit.task.show",
            json!({"workspace": "ws_remote", "id": "ORB-00001", "with_context": true}),
            ToolSessionContext::trusted_local(
                Some("ws_remote".to_string()),
                Some("hm_spoke".to_string()),
                Some("spoke".to_string()),
            ),
        )
        .expect_err("spoke must not fall through to a local coordination store");

    assert!(
        error
            .to_string()
            .contains("local coordination fallback is forbidden")
    );
    assert!(!root.path().join("tasks").exists());
}

#[test]
fn broker_spoke_hub_denial_writes_no_local_coordination_state() {
    use chrono::Utc;
    use orbit_common::types::{Workspace, WorkspaceRegistry, WorkspaceStatus};
    use orbit_remote::{HostMode, NewHostIdentity, ensure_host_identity};

    let root = tempfile::tempdir().expect("global root");
    ensure_host_identity(root.path(), || {
        Ok(NewHostIdentity {
            host_id: "spoke".to_string(),
            mode: HostMode::Spoke,
        })
    })
    .expect("spoke identity");
    orbit_remote::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![Workspace {
                id: "ws_remote".to_string(),
                name: "Remote".to_string(),
                owner_machine_id: Some("hm_remote_owner".to_string()),
                git_remote: None,
                ship_mode: None,
                base_branch: "agent-main".to_string(),
                status: WorkspaceStatus::Active,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            ..Default::default()
        },
        &orbit_remote::workspace_registry::registry_path_for(root.path()),
    )
    .expect("workspace registry");
    let host = BrokerMcpHost::new(root.path().to_path_buf());
    let mut context = ToolSessionContext::trusted_local(
        Some("ws_remote".to_string()),
        Some("hm_spoke".to_string()),
        Some("spoke".to_string()),
    );
    context.mcp_call_id = Some("mcall-spoke-denied".to_string());
    let error = host
        .call_tool(
            "orbit.task.add",
            json!({
                "workspace": "ws_remote",
                "title": "Must not land locally",
                "description": "Denied on spoke",
                "model": "codex"
            }),
            context,
        )
        .expect_err("spoke has no hub transport");
    assert!(error.to_string().contains("no MCP hub transport"));
    assert!(!root.path().join("tasks").exists());
    assert!(!root.path().join("frictions").exists());
}

#[test]
fn broker_capability_denial_precedes_coordination_store_open() {
    use chrono::Utc;
    use orbit_common::types::{McpCapability, Workspace, WorkspaceRegistry, WorkspaceStatus};

    let root = tempfile::tempdir().expect("global root");
    orbit_remote::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![Workspace {
                id: "ws_checkoutless".to_string(),
                name: "Checkoutless".to_string(),
                owner_machine_id: None,
                git_remote: None,
                ship_mode: None,
                base_branch: "agent-main".to_string(),
                status: WorkspaceStatus::Active,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
            ..Default::default()
        },
        &orbit_remote::workspace_registry::registry_path_for(root.path()),
    )
    .expect("workspace registry");
    let host = BrokerMcpHost::new(root.path().to_path_buf());
    let mut context = ToolSessionContext::trusted_local(
        Some("ws_checkoutless".to_string()),
        Some("hm_hub".to_string()),
        Some("hub".to_string()),
    );
    context.effective_capabilities = BTreeSet::from([McpCapability::Runner]);
    let error = host
        .call_tool(
            "orbit.task.add",
            json!({
                "workspace": "ws_checkoutless",
                "title": "Denied",
                "description": "Must not write",
                "model": "codex"
            }),
            context,
        )
        .expect_err("runner capability is not task-authority");
    assert!(error.to_string().contains("MCP capability denied"));
    assert!(!root.path().join("tasks").exists());
}

struct RuntimeMcpHost {
    runtime: OrbitRuntime,
}

impl McpHost for RuntimeMcpHost {
    fn list_mcp_tool_definitions(&self) -> Result<Vec<McpToolDefinition>, OrbitError> {
        self.runtime.list_mcp_tool_definitions()
    }

    fn call_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        audited_mcp_call_with_session_context(
            &self.runtime,
            name,
            input,
            normalize_trusted_call_context(session_context),
        )
    }

    fn call_in_process_tool(
        &self,
        name: &str,
        input: Value,
        session_context: ToolSessionContext,
        dispatch: &mut dyn FnMut(Value, ToolSessionContext) -> Result<Value, OrbitError>,
    ) -> Result<Value, OrbitError> {
        let session_context = normalize_trusted_call_context(session_context);
        let dispatch_context = session_context.clone();
        self.runtime
            .execute_in_process_tool_dispatch(
                name,
                input,
                ToolEntryPoint::Mcp,
                session_context,
                |input| {
                    ensure_mcp_tool_exposed(name)?;
                    dispatch(input, dispatch_context)
                },
            )
            .map(|outcome| outcome.value)
    }

    fn learning_candidates_for_path(
        &self,
        path: &str,
        _session_context: ToolSessionContext,
    ) -> Result<Value, OrbitError> {
        let rows = self.runtime.search_learnings(LearningSearchParams {
            path: Some(path.to_string()),
            tag: None,
            query: None,
            limit: None,
        })?;
        Ok(Value::Array(
            rows.into_iter()
                .map(|row| {
                    json!({
                        "id": row.learning.id,
                        "summary": row.learning.summary,
                        "priority": row.learning.priority,
                        "updated_at": row.learning.updated_at.to_rfc3339(),
                    })
                })
                .collect(),
        ))
    }

    fn get_session_learning_state(
        &self,
        session_id: &str,
    ) -> Result<Option<LearningInjectionState>, OrbitError> {
        self.runtime.get_session_learning_state(session_id)
    }

    fn upsert_session_learning_state(
        &self,
        session_id: &str,
        state: &LearningInjectionState,
    ) -> Result<(), OrbitError> {
        self.runtime
            .upsert_session_learning_state(session_id, state)
    }
}

const EXPECTED_INACTIVE_TOOL_NAMES: &[&str] = &[
    "orbit.docs.index",
    "orbit.docs.migrate",
    "orbit.docs.add",
    "orbit.docs.list",
    "orbit.docs.show",
    "orbit.task.locks",
    "orbit.task.locks.release",
    "orbit.task.locks.reserve",
    "orbit.semantic.index",
    "orbit.semantic.install",
    "orbit.semantic.stats",
    "orbit.learning.sync",
    "orbit.learning.list",
    "orbit.friction.stats",
    // ORB-00289: trimmed admin/destructive tools — CLI path retains them.
    "orbit.adr.list",
    "orbit.semantic.uninstall",
    "orbit.task.delete",
    "orbit.task.lint",
    "orbit.learning.prune",
    // Agent-surface narrowing: human-decision tools — CLI-only.
    "orbit.task.reject",
    "orbit.friction.resolve",
];

// ORB-00289 + agent-surface narrowing: admin/destructive and triage tools
// (`orbit.adr.list`, `orbit.semantic.uninstall`, `orbit.task.delete`,
// `orbit.task.lint`, `orbit.task.reject`, `orbit.learning.prune`,
// `orbit.friction.resolve`) deliberately omitted — retained on
// the CLI / `runtime.run_tool` path only.
const REQUIRED_AGENT_FACING_TOOL_NAMES: &[&str] = &[
    "orbit.search",
    "orbit.task.add",
    "orbit.task.approve",
    "orbit.task.artifact.put",
    "orbit.task.show",
    "orbit.task.update",
    "orbit.task.list",
    "orbit.task.review_thread.add",
    "orbit.task.review_thread.list",
    "orbit.task.review_thread.reply",
    "orbit.task.review_thread.resolve",
    "orbit.task.start",
    // ORB-00391: the v1 orbit.graph.* builtins were decommissioned. The agent
    // graph surface is now served by the in-process orbit-graph (v2) adapter in
    // orbit-mcp (see crates/orbit-mcp/src/adapter/graph.rs and its tests), not by
    // the orbit-tools runtime registry, so no orbit.graph.* tool appears here.
    "orbit.adr.add",
    "orbit.adr.show",
    "orbit.adr.supersede",
    "orbit.adr.update",
    "orbit.learning.add",
    "orbit.learning.show",
    "orbit.learning.update",
    "orbit.friction.add",
    "orbit.friction.list",
    "orbit.friction.show",
    "orbit.friction.tags",
    "orbit.friction.update",
];

fn is_runtime_mcp_category_tool(name: &str) -> bool {
    name == "orbit.search"
        || name.starts_with("orbit.task.")
        || name.starts_with("orbit.friction.")
        || name.starts_with("orbit.graph.")
        || name.starts_with("orbit.adr.")
        || name.starts_with("orbit.semantic.")
        || name.starts_with("orbit.docs.")
        || name.starts_with("orbit.learning.")
}

#[test]
fn inactive_tools_are_not_in_the_mcp_safe_surface() {
    let safe_names: BTreeSet<String> = safe_mcp_tool_names().into_iter().collect();
    assert_eq!(EXPECTED_INACTIVE_TOOL_NAMES.len(), 21);

    for name in EXPECTED_INACTIVE_TOOL_NAMES {
        assert!(
            !safe_names.contains(*name),
            "inactive tool leaked into safe MCP names: {name}"
        );
        assert!(
            !is_mcp_tool_exposed(name),
            "inactive tool exposed by MCP preflight: {name}"
        );
    }
}

#[test]
fn safe_surface_matches_runtime_graph_and_task_tools() {
    let runtime = OrbitRuntime::in_memory().expect("build test runtime");
    let names: BTreeSet<String> = runtime
        .list_tools()
        .expect("list tools")
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    let all_names: BTreeSet<String> = runtime
        .list_all_tools()
        .expect("list all tools")
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    let safe_names: BTreeSet<String> = safe_mcp_tool_names().into_iter().collect();
    let inactive_names: BTreeSet<&str> = EXPECTED_INACTIVE_TOOL_NAMES.iter().copied().collect();

    for name in safe_mcp_tool_names()
        .into_iter()
        .filter(|name| !name.starts_with("orbit.graph."))
    {
        assert!(
            names.contains(&name),
            "MCP-candidate tool missing from runtime registry: {name}"
        );
    }

    for name in EXPECTED_INACTIVE_TOOL_NAMES {
        assert!(
            !names.contains(*name),
            "inactive tool leaked into default runtime list: {name}"
        );
        assert!(
            all_names.contains(*name),
            "inactive tool should remain registered for inspection: {name}"
        );
        assert!(
            !is_mcp_tool_exposed(name),
            "inactive tool exposed by MCP preflight: {name}"
        );
    }

    for name in REQUIRED_AGENT_FACING_TOOL_NAMES {
        assert!(
            names.contains(*name),
            "required agent-facing tool missing from runtime registry: {name}"
        );
        assert!(
            safe_names.contains(*name),
            "required agent-facing tool missing from safe MCP names: {name}"
        );
        assert!(
            is_mcp_tool_exposed(name),
            "required agent-facing tool rejected by MCP preflight: {name}"
        );
    }

    for name in names
        .iter()
        .filter(|name| is_runtime_mcp_category_tool(name))
    {
        let should_expose = !inactive_names.contains(name.as_str());
        assert!(
            safe_names.contains(name.as_str()) == should_expose,
            "runtime tool MCP exposure mismatch for {name}"
        );
        assert!(
            is_mcp_tool_exposed(name) == should_expose,
            "runtime tool MCP preflight mismatch for {name}"
        );
    }

    for name in [
        "orbit.graph.add",
        "orbit.graph.delete",
        "orbit.graph.move",
        "orbit.graph.write",
    ] {
        assert!(
            !names.contains(name),
            "runtime exposes graph write tool: {name}"
        );
        assert!(!is_mcp_tool_exposed(name));
    }

    assert!(!is_mcp_tool_exposed("orbit.state.get"));
    assert!(!is_mcp_tool_exposed("demo.hello"));
}

#[test]
fn graph_adapter_names_have_schema_adjacent_canonical_definitions() {
    let safe_names: BTreeSet<String> = safe_mcp_tool_names().into_iter().collect();
    let adapter_names: BTreeSet<&str> = orbit_mcp::graph_tool_names().iter().copied().collect();
    let configured_names: BTreeSet<&str> = safe_names
        .iter()
        .map(String::as_str)
        .filter(|name| name.starts_with("orbit.graph."))
        .collect();

    assert_eq!(adapter_names, configured_names);
    assert!(adapter_names.iter().all(|name| safe_names.contains(*name)));
    for name in adapter_names {
        assert!(is_mcp_tool_exposed(name));
    }
}

#[test]
fn runtime_mcp_host_lists_safe_tools_and_no_graph_surface_after_v2_cutover() {
    let runtime = OrbitRuntime::in_memory().expect("build test runtime");
    let host = RuntimeMcpHost { runtime };
    let listed: BTreeSet<String> = host
        .list_mcp_tool_definitions()
        .expect("list valid MCP definitions")
        .into_iter()
        .map(|definition| definition.schema.name)
        .collect();

    for name in safe_mcp_tool_names()
        .into_iter()
        .filter(|name| !name.starts_with("orbit.graph."))
    {
        assert!(
            listed.contains(&name),
            "client-visible MCP tool list missing safe tool: {name}"
        );
    }

    for name in REQUIRED_AGENT_FACING_TOOL_NAMES {
        assert!(
            listed.contains(*name),
            "client-visible MCP tool list missing required agent-facing tool: {name}"
        );
    }

    for name in EXPECTED_INACTIVE_TOOL_NAMES {
        assert!(
            !listed.contains(*name),
            "client-visible MCP tool list exposes inactive ops tool: {name}"
        );
    }

    // ORB-00391: the orbit-tools runtime host still owns no `orbit.graph.*`
    // implementation. The orbit-mcp adapter now replaces any accidentally
    // re-exposed known graph schema, but this assertion preserves the intended
    // crate ownership boundary.
    assert!(
        !listed.iter().any(|name| name.starts_with("orbit.graph.")),
        "host must expose no orbit.graph.* tool after the v2 cutover, found: {:?}",
        listed
            .iter()
            .filter(|name| name.starts_with("orbit.graph."))
            .collect::<Vec<_>>()
    );
}

#[derive(Debug, Deserialize)]
struct McpConformanceFixture {
    capabilities: McpConformanceCapabilities,
    scopes: McpConformanceScopes,
    private_connector: McpConformancePrivateConnector,
    hub_schema_digest: McpConformanceHubDigest,
    tools: BTreeMap<String, McpConformancePolicy>,
}

#[derive(Debug, Deserialize)]
struct McpConformancePrivateConnector {
    spoke_registration: McpConformanceSpokeRegistration,
}

#[derive(Debug, Deserialize)]
struct McpConformanceSpokeRegistration {
    method: String,
    schema_version: u32,
    advertised: bool,
    ordinary_tools_call: bool,
    allowed_capabilities: BTreeSet<McpCapability>,
    unknown_caller_only_operation: bool,
    ordinary_calls_require_active_registration: bool,
    only_path_bearing_fields: Vec<String>,
    cache_refresh: String,
}

#[derive(Debug, Deserialize)]
struct McpConformanceHubDigest {
    domain_tag: String,
    contract_revision: u32,
    canonical_registry_revision: u32,
    golden_vector: McpConformanceGoldenVector,
}

#[derive(Debug, Deserialize)]
struct McpConformanceGoldenVector {
    capability: McpCapability,
    canonical_json: String,
    expected_sha256: String,
}

#[derive(Debug, Deserialize)]
struct McpConformanceCapabilities {
    allowed_values: BTreeSet<McpCapability>,
}

#[derive(Debug, Deserialize)]
struct McpConformanceScopes {
    allowed_values: BTreeSet<McpToolScope>,
}

#[derive(Debug, Deserialize)]
struct McpConformancePolicy {
    placement: McpToolPlacement,
    scope: McpToolScope,
    allowed_capabilities: BTreeSet<McpCapability>,
}

#[test]
fn canonical_mcp_policy_conforms_to_frozen_v1_fixture() {
    assert!(
        serde_yaml::from_str::<McpConformancePolicy>(
            "{ placement: hub, scope: workspace-required, allowed_capabilities: [unknown] }"
        )
        .is_err(),
        "unknown capabilities must fail typed fixture parsing"
    );
    assert!(
        serde_yaml::from_str::<McpConformancePolicy>(
            "{ placement: hub, scope: workspace-required }"
        )
        .is_err(),
        "missing capability policy must fail typed fixture parsing"
    );
    assert!(
        serde_yaml::from_str::<McpConformancePolicy>(
            "{ placement: hub, scope: unknown, allowed_capabilities: [operator] }"
        )
        .is_err(),
        "unknown scopes must fail typed fixture parsing"
    );
    assert!(
        serde_yaml::from_str::<McpConformancePolicy>(
            "{ placement: hub, allowed_capabilities: [operator] }"
        )
        .is_err(),
        "missing scope metadata must fail typed fixture parsing"
    );

    let fixture: McpConformanceFixture = serde_yaml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/design/mcp-bridge/references/conformance-v1.yaml"
    )))
    .expect("frozen MCP conformance fixture must use known typed policy values");
    assert_eq!(
        fixture.capabilities.allowed_values,
        BTreeSet::from([
            McpCapability::Agent,
            McpCapability::Operator,
            McpCapability::Runner,
        ])
    );
    assert_eq!(
        fixture.scopes.allowed_values,
        BTreeSet::from([McpToolScope::WorkspaceRequired, McpToolScope::Global])
    );
    let registration = fixture.private_connector.spoke_registration;
    assert_eq!(
        registration.method,
        orbit_common::types::SPOKE_REGISTRATION_METHOD_V1
    );
    assert_eq!(
        registration.schema_version,
        orbit_common::types::SPOKE_REGISTRATION_SCHEMA_VERSION
    );
    assert!(!registration.advertised);
    assert!(!registration.ordinary_tools_call);
    assert_eq!(
        registration.allowed_capabilities,
        BTreeSet::from([
            McpCapability::Agent,
            McpCapability::Operator,
            McpCapability::Runner,
        ])
    );
    assert!(registration.unknown_caller_only_operation);
    assert!(registration.ordinary_calls_require_active_registration);
    assert_eq!(registration.only_path_bearing_fields, ["presence[].root"]);
    assert_eq!(
        registration.cache_refresh,
        "definitive-complete-response-only"
    );
    assert!(
        !fixture
            .tools
            .contains_key(orbit_common::types::SPOKE_REGISTRATION_METHOD_V1),
        "private registration must never enter the canonical tool matrix"
    );
    assert_eq!(
        fixture.hub_schema_digest.domain_tag,
        orbit_mcp::HUB_SCHEMA_DOMAIN
    );
    assert_eq!(
        fixture.hub_schema_digest.contract_revision,
        orbit_mcp::MCP_CONTRACT_REVISION
    );
    assert_eq!(
        fixture.hub_schema_digest.canonical_registry_revision,
        orbit_mcp::CANONICAL_MCP_REGISTRY_REVISION
    );
    let vector_definition = McpToolDefinition::new(
        orbit_common::types::ToolSchema {
            name: "orbit.task.show".to_string(),
            description: "Show one task".to_string(),
            parameters: vec![orbit_common::types::ToolParam {
                name: "id".to_string(),
                description: "Task ID".to_string(),
                param_type: "string".to_string(),
                required: true,
            }],
            builtin: true,
        },
        McpToolPolicy::agent_and_operator(McpToolPlacement::Hub),
    )
    .expect("golden definition");
    let vector = &fixture.hub_schema_digest.golden_vector;
    let mut expected_bytes = format!("{}\0", fixture.hub_schema_digest.domain_tag).into_bytes();
    expected_bytes.extend_from_slice(vector.canonical_json.as_bytes());
    assert_eq!(
        orbit_mcp::canonical_hub_schema_bytes(
            std::slice::from_ref(&vector_definition),
            vector.capability
        )
        .expect("canonical golden bytes"),
        expected_bytes
    );
    assert_eq!(
        orbit_mcp::hub_schema_digest(&[vector_definition], vector.capability)
            .expect("golden digest"),
        vector.expected_sha256
    );

    let definitions =
        canonical_mcp_tool_definitions().expect("canonical MCP definitions are valid");
    assert!(matches!(
        validate_mcp_tool_definitions(&[definitions[0].clone(), definitions[0].clone()]),
        Err(McpToolPolicyError::DuplicateCanonicalName(_))
    ));
    let empty_policy =
        serde_yaml::from_str::<McpToolPolicy>("{ placement: hub, allowed_capabilities: [] }")
            .expect("typed policy can deserialize an invalid empty set for validation coverage");
    let empty_definition = McpToolDefinition {
        schema: definitions[0].schema.clone(),
        policy: empty_policy,
    };
    assert_eq!(
        validate_mcp_tool_definitions(&[empty_definition]),
        Err(McpToolPolicyError::EmptyCapabilities)
    );
    let mut dotted = definitions[0].clone();
    dotted.schema.name = "demo.name".to_string();
    let mut underscored = definitions[1].clone();
    underscored.schema.name = "demo_name".to_string();
    assert!(matches!(
        validate_mcp_tool_definitions(&[dotted, underscored]),
        Err(McpToolPolicyError::DuplicateAdvertisedName(_))
    ));
    let canonical_names: BTreeSet<String> = definitions
        .iter()
        .map(|definition| definition.schema.name.clone())
        .collect();
    assert_eq!(canonical_names.len(), definitions.len());

    let advertised_names: BTreeSet<String> = definitions
        .iter()
        .map(|definition| mcp_advertised_tool_name(&definition.schema.name))
        .collect();
    assert_eq!(advertised_names.len(), definitions.len());
    assert!(advertised_names.iter().all(|name| !name.is_empty()));

    let fixture_names: BTreeSet<String> = fixture.tools.keys().cloned().collect();
    assert_eq!(canonical_names, fixture_names);
    for definition in &definitions {
        let expected = fixture
            .tools
            .get(&definition.schema.name)
            .expect("every advertised canonical name has fixture policy");
        assert_eq!(
            definition.policy.placement(),
            expected.placement,
            "{}",
            definition.schema.name
        );
        assert_eq!(
            definition.policy.allowed_capabilities(),
            &expected.allowed_capabilities,
            "{}",
            definition.schema.name
        );
        assert_eq!(
            definition.policy.scope(),
            expected.scope,
            "{}",
            definition.schema.name
        );
        assert!(
            !expected.allowed_capabilities.is_empty(),
            "{} has an empty capability set",
            definition.schema.name
        );
    }

    let matrix = mcp_capability_placement_matrix(&definitions)
        .expect("capability matrix derives from valid definitions");
    for definition in &definitions {
        for capability in definition.policy.allowed_capabilities() {
            assert!(
                matrix[&definition.policy.placement()][capability]
                    .contains(&definition.schema.name),
                "matrix omitted {} at {:?}/{:?}",
                definition.schema.name,
                definition.policy.placement(),
                capability
            );
        }
    }

    let runner_only = McpToolPolicy::new(McpToolPlacement::Hub, [McpCapability::Runner])
        .expect("runner-only is a valid non-empty capability set");
    assert_eq!(
        runner_only.allowed_capabilities(),
        &BTreeSet::from([McpCapability::Runner])
    );
    assert_eq!(runner_only.scope(), McpToolScope::WorkspaceRequired);
    let operator_only = McpToolPolicy::operator_only(McpToolPlacement::Hub);
    assert_eq!(
        operator_only.allowed_capabilities(),
        &BTreeSet::from([McpCapability::Operator])
    );
    assert_eq!(operator_only.scope(), McpToolScope::WorkspaceRequired);

    let global_operator = operator_only.with_scope(McpToolScope::Global);
    assert_eq!(global_operator.scope(), McpToolScope::Global);
}

mod audited_mcp_call_tests {
    use std::collections::BTreeSet;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::time::Instant;

    use orbit_cmd::learning_hook::{
        HookOutputFormat, ORBIT_LEARNING_PER_CALL_CAP_ENV, ORBIT_LEARNING_SESSION_CAP_ENV,
        ORBIT_SESSION_ID_ENV, run_pretooluse_input,
    };
    use orbit_common::types::{
        AuditEventStatus, HostRegistration, LearningInjectionCaps, LearningInjectionState,
        LearningReminder, LearningScope, McpCapability, McpTransport, ToolSessionContext,
    };
    use orbit_core::LearningEvidence;
    use orbit_core::{LearningCreateParams, OrbitError, OrbitRuntime};
    use orbit_mcp::McpHost;
    use serde_json::json;

    use super::super::host::audited_mcp_call;
    use super::RuntimeMcpHost;

    // ORB-00289: the previous `create_task` helper + the three
    // `task_delete_*_over_mcp` tests asserted that `orbit.task.delete` was
    // dispatchable via MCP. That contract was removed when the tool moved to
    // CLI-only (inactive on the agent surface); the generic
    // `inactive_tool_is_rejected_over_mcp_dispatch` test below now covers
    // the rejection-on-inactive contract for every inactive tool, and the
    // delete business logic (force flag, protected statuses, audit row
    // shape) is exercised through `runtime.run_tool` in
    // `orbit-core/.../orbit_tool_host/{task_tools_tests, tests/task_tools}`.

    #[test]
    fn preflight_failure_for_unknown_tool_records_failure_audit_row() {
        let _guard = EnvGuard::set(&[
            ("ORBIT_AGENT_NAME", None),
            ("ORBIT_AGENT_MODEL", None),
            ("ORBIT_MANAGED_RUN_CONTEXT", None),
            ("ORBIT_RUN_ID", None),
        ]);
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        // The runtime is the source of truth for the audit store; the
        // wrapper writes to the same backing store the MCP host shares.
        let result = audited_mcp_call(&runtime, "orbit.state.get", json!({}));
        assert!(
            result.is_err(),
            "preflight rejects unknown / unexposed tool"
        );

        let events = runtime
            .list_audit_events(None, Some("orbit.state.get".to_string()), None, None, 16)
            .expect("list audit events");
        assert_eq!(events.len(), 1, "preflight failure produced one audit row");
        let row = &events[0];
        assert_eq!(row.command, "tool");
        assert_eq!(row.subcommand.as_deref(), Some("run-mcp"));
        assert_eq!(row.tool_name.as_deref(), Some("orbit.state.get"));
        assert_eq!(row.status, AuditEventStatus::Denied);
        assert_eq!(row.role, "unverified");
        assert_eq!(row.transport, Some(McpTransport::Local));
        assert_eq!(
            row.effective_capabilities,
            [McpCapability::Agent].into_iter().collect()
        );
        assert!(row.origin_session_id.is_some());
        assert!(row.mcp_call_id.is_some());
        assert_eq!(row.exit_code, 1);
        assert!(row.error_message.is_some());
        assert!(
            row.duration_ms >= 1,
            "duration_ms clamped to >= 1 (got {})",
            row.duration_ms
        );
    }

    #[test]
    fn happy_path_dispatch_records_one_audit_row_via_runtime() {
        let _guard = EnvGuard::set(&[
            ("ORBIT_AGENT_NAME", None),
            ("ORBIT_AGENT_MODEL", None),
            ("ORBIT_MANAGED_RUN_CONTEXT", None),
            ("ORBIT_RUN_ID", None),
        ]);
        // ORB-00202: migrated from deleted `orbit.task.search` to
        // `orbit.search`, the unified replacement.
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let host = RuntimeMcpHost {
            runtime: runtime.clone(),
        };
        let value = host
            .call_tool(
                "orbit.search",
                json!({ "query": "anything", "kind": "task" }),
                Default::default(),
            )
            .expect("dispatch ok");
        assert!(
            value.get("results").is_some(),
            "orbit.search returns wrapped results"
        );

        let events = runtime
            .list_audit_events(None, Some("orbit.search".to_string()), None, None, 16)
            .expect("list audit events");
        assert_eq!(events.len(), 1, "exactly one audit row for happy path");
        assert_eq!(events[0].subcommand.as_deref(), Some("run-mcp"));
        assert_eq!(events[0].status, AuditEventStatus::Success);
        assert_eq!(events[0].role, "unverified");
        assert_eq!(events[0].transport, Some(McpTransport::Local));
        assert!(events[0].mcp_call_id.is_some());
    }

    #[test]
    fn runtime_mcp_host_executes_global_registry_discovery_without_session_workspace() {
        let _guard = EnvGuard::set(&[
            ("ORBIT_AGENT_NAME", None),
            ("ORBIT_AGENT_MODEL", None),
            ("ORBIT_MANAGED_RUN_CONTEXT", None),
            ("ORBIT_RUN_ID", None),
        ]);
        let root = tempfile::tempdir().expect("runtime root");
        let global_root = root.path().join("global");
        let workspace_root = root.path().join("workspace");
        std::fs::create_dir_all(&global_root).expect("global root");
        std::fs::create_dir_all(&workspace_root).expect("workspace root");
        let runtime = orbit_remote::runtime::RemoteRuntimeFactory::open_resolved_checkout(
            &global_root,
            &workspace_root,
            &workspace_root,
            orbit_core::runtime::WorkspaceRuntimeBinding {
                workspace_id: "ws_runtime".to_string(),
                repo_root: root.path().join("repo"),
                ship_mode: orbit_core::ShipMode::Local,
            },
        )
        .expect("build Remote-composed test runtime");
        let store = orbit_remote::RemoteStore::from_store(
            runtime.sqlite_store().expect("open real SQLite store"),
        )
        .expect("adopt Remote registry store");
        store
            .register_hub(&HostRegistration {
                machine_id: "hm_hub".to_string(),
                host_id: "hub".to_string(),
                labels: BTreeSet::from(["coordination".to_string()]),
            })
            .expect("register hub");
        store
            .register_host(&HostRegistration {
                machine_id: "hm_owner".to_string(),
                host_id: "owner".to_string(),
                labels: BTreeSet::from(["execution".to_string()]),
            })
            .expect("register owner");
        store
            .bind_workspace_owner("ws_checkoutless", "hm_owner")
            .expect("bind workspace owner without a local checkout");
        let expected_revision = store.registry_revision().expect("registry revision");
        assert!(expected_revision > 0);

        let host = RuntimeMcpHost {
            runtime: runtime.clone(),
        };
        let call = |tool_name: &str, call_id: &str| {
            let mut context = ToolSessionContext::trusted_local(
                None,
                Some("hm_hub".to_string()),
                Some("hub".to_string()),
            );
            context.effective_capabilities = BTreeSet::from([McpCapability::Operator]);
            context.origin_session_id = Some("mcp-session-global-discovery".to_string());
            context.mcp_call_id = Some(call_id.to_string());
            host.call_tool(tool_name, json!({}), context)
                .expect("global registry discovery succeeds")
        };

        let hosts = call("orbit.host.list", "mcall-host-list");
        let workspaces = call("orbit.workspace.list", "mcall-workspace-list");
        for value in [&hosts, &workspaces] {
            assert_eq!(value["hub_machine_id"], "hm_hub");
            assert_eq!(value["registry_revision"].as_u64(), Some(expected_revision));
            let serialized = serde_json::to_string(value).expect("serialize result");
            for forbidden in ["repo_root", "orbit_dir", "path_overrides", "crews", "model"] {
                assert!(
                    !serialized.contains(forbidden),
                    "global discovery leaked {forbidden}: {serialized}"
                );
            }
        }
        let host_rows = hosts["hosts"].as_array().expect("host rows");
        assert_eq!(host_rows.len(), 2);
        assert!(
            host_rows
                .iter()
                .any(|row| row["machine_id"] == "hm_hub" && row["host_id"] == "hub")
        );
        assert!(
            host_rows
                .iter()
                .any(|row| row["machine_id"] == "hm_owner" && row["host_id"] == "owner")
        );
        let workspace_rows = workspaces["workspaces"].as_array().expect("workspace rows");
        assert_eq!(workspace_rows.len(), 1);
        assert_eq!(workspace_rows[0]["workspace_id"], "ws_checkoutless");
        assert_eq!(workspace_rows[0]["owner_machine_id"], "hm_owner");
        assert_eq!(workspace_rows[0]["owner_host_id"], "owner");

        for (tool_name, call_id) in [
            ("orbit.host.list", "mcall-host-list"),
            ("orbit.workspace.list", "mcall-workspace-list"),
        ] {
            let events = runtime
                .list_audit_events(None, Some(tool_name.to_string()), None, None, 16)
                .expect("list discovery audit events");
            assert_eq!(events.len(), 1, "exactly one audit row for {tool_name}");
            let row = &events[0];
            assert_eq!(row.status, AuditEventStatus::Success);
            assert_eq!(row.workspace_id, None);
            assert_eq!(row.caller_machine_id.as_deref(), Some("hm_hub"));
            assert_eq!(row.process_machine_id.as_deref(), Some("hm_hub"));
            assert_eq!(row.transport, Some(McpTransport::Local));
            assert_eq!(
                row.effective_capabilities,
                BTreeSet::from([McpCapability::Operator])
            );
            assert_eq!(row.mcp_call_id.as_deref(), Some(call_id));
        }
    }

    #[test]
    fn in_process_graph_dispatch_records_success_and_failure_audit_rows() {
        let _guard = EnvGuard::set(&[
            ("ORBIT_AGENT_NAME", None),
            ("ORBIT_AGENT_MODEL", None),
            ("ORBIT_MANAGED_RUN_CONTEXT", None),
            ("ORBIT_RUN_ID", None),
        ]);
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let host = RuntimeMcpHost {
            runtime: runtime.clone(),
        };

        let mut success_dispatch =
            |input: serde_json::Value, _session_context: ToolSessionContext| {
                assert_eq!(input["query"], "dispatch");
                Ok(json!({ "matches": [] }))
            };
        let mut success_context = ToolSessionContext::trusted_local(
            Some("ws_orbit".to_string()),
            Some("hm_local".to_string()),
            Some("local-host".to_string()),
        );
        success_context.origin_session_id = Some("mcp-session-graph".to_string());
        success_context.mcp_call_id = Some("mcall-graph-success".to_string());
        host.call_in_process_tool(
            "orbit.graph.search",
            json!({ "query": "dispatch", "model": "codex" }),
            success_context,
            &mut success_dispatch,
        )
        .expect("allowlisted graph call succeeds");

        let mut failure_dispatch =
            |_input: serde_json::Value, _session_context: ToolSessionContext| {
                Err(OrbitError::InvalidInput("invalid selector".to_string()))
            };
        let mut failure_context = ToolSessionContext::trusted_local(
            Some("ws_orbit".to_string()),
            Some("hm_local".to_string()),
            Some("local-host".to_string()),
        );
        failure_context.origin_session_id = Some("mcp-session-graph".to_string());
        failure_context.mcp_call_id = Some("mcall-graph-failure".to_string());
        host.call_in_process_tool(
            "orbit.graph.show",
            json!({ "selector": "invalid", "model": "codex" }),
            failure_context,
            &mut failure_dispatch,
        )
        .expect_err("graph implementation failure propagates");

        for (name, expected_status) in [
            ("orbit.graph.search", AuditEventStatus::Success),
            ("orbit.graph.show", AuditEventStatus::Failure),
        ] {
            let events = runtime
                .list_audit_events(None, Some(name.to_string()), None, None, 16)
                .expect("list graph audit events");
            assert_eq!(events.len(), 1, "exactly one audit row for {name}");
            let row = &events[0];
            assert_eq!(row.subcommand.as_deref(), Some("run-mcp"));
            assert_eq!(row.tool_name.as_deref(), Some(name));
            assert_eq!(row.role, "unverified");
            assert_eq!(row.status, expected_status);
            assert!(row.duration_ms >= 1);
            assert_eq!(row.workspace_id.as_deref(), Some("ws_orbit"));
            assert_eq!(row.caller_machine_id.as_deref(), Some("hm_local"));
            assert_eq!(row.process_machine_id.as_deref(), Some("hm_local"));
            assert_eq!(row.transport, Some(McpTransport::Local));
            assert_eq!(row.origin_session_id.as_deref(), Some("mcp-session-graph"));
            assert!(row.mcp_call_id.as_deref().is_some_and(|call_id| {
                call_id == "mcall-graph-success" || call_id == "mcall-graph-failure"
            }));
        }
    }

    #[test]
    fn unallowlisted_graph_tool_is_rejected_before_in_process_dispatch() {
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let host = RuntimeMcpHost {
            runtime: runtime.clone(),
        };
        let mut called = false;
        let mut dispatch = |_input: serde_json::Value, _session_context: ToolSessionContext| {
            called = true;
            Ok(json!({}))
        };

        let error = host
            .call_in_process_tool(
                "orbit.graph.pack",
                json!({ "model": "codex" }),
                Default::default(),
                &mut dispatch,
            )
            .expect_err("unallowlisted graph tool is rejected");
        assert!(matches!(error, OrbitError::NotFound { .. }));
        assert!(!called, "rejected graph implementation must not run");

        let events = runtime
            .list_audit_events(None, Some("orbit.graph.pack".to_string()), None, None, 16)
            .expect("list graph preflight audit event");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].subcommand.as_deref(), Some("run-mcp"));
        assert_eq!(events[0].status, AuditEventStatus::Denied);
    }

    #[test]
    fn orbit_search_is_exposed_to_mcp_dispatch() {
        // ORB-00202: `orbit.learning.search` was deleted; the unified
        // `orbit.search` surface is exposed instead.
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let value = audited_mcp_call(&runtime, "orbit.search", json!({ "query": "anything" }))
            .expect("orbit.search dispatch ok");
        assert!(
            value.get("results").is_some(),
            "orbit.search returns wrapped results"
        );
    }

    #[test]
    fn runtime_mcp_host_and_cli_hook_share_session_learning_state() {
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let learning = runtime
            .create_learning(LearningCreateParams {
                summary: "Use the shared state table.".to_string(),
                scope: LearningScope {
                    paths: vec!["crates/orbit-core/src/lib.rs".to_string()],
                    ..Default::default()
                },
                body: String::new(),
                evidence: Vec::<LearningEvidence>::new(),
                created_by: Some("codex".to_string()),
                priority: Some(7),
            })
            .expect("create learning");
        let host = RuntimeMcpHost {
            runtime: runtime.clone(),
        };
        let candidates = host
            .learning_candidates_for_path("crates/orbit-core/src/lib.rs", Default::default())
            .expect("mcp learning candidates");
        let candidates = candidates
            .as_array()
            .expect("candidate array")
            .iter()
            .map(|item| LearningReminder {
                id: item
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .expect("candidate id")
                    .to_string(),
                summary: item
                    .get("summary")
                    .and_then(serde_json::Value::as_str)
                    .expect("candidate summary")
                    .to_string(),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            candidates
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            [learning.id.as_str()]
        );

        let caps = LearningInjectionCaps {
            per_call: 5,
            per_session_hard: 20,
        };
        let mut mcp_state = LearningInjectionState::default();
        let admitted = mcp_state.admit_reminders(&candidates, caps);
        assert_eq!(
            admitted
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            [learning.id.as_str()]
        );
        host.upsert_session_learning_state("session-shared", &mcp_state)
            .expect("mcp writes shared session state");

        let _guard = EnvGuard::set(&[
            (ORBIT_SESSION_ID_ENV, Some("session-shared")),
            (ORBIT_LEARNING_PER_CALL_CAP_ENV, Some("5")),
            (ORBIT_LEARNING_SESSION_CAP_ENV, Some("20")),
            ("ORBIT_ACTIVE_TASK_ID", None),
            ("ORBIT_TASK_ID", None),
        ]);
        let stdin = json!({
            "tool_name": "mcp__orbit__fs_read",
            "tool_input": {
                "path": "crates/orbit-core/src/lib.rs"
            }
        })
        .to_string();
        let output =
            run_pretooluse_input(&runtime, &stdin, HookOutputFormat::Codex, Instant::now())
                .expect("cli hook succeeds");
        assert_eq!(output, None);

        let persisted = runtime
            .get_session_learning_state("session-shared")
            .expect("read shared session state")
            .expect("session state exists");
        assert_eq!(persisted.count, 1);
        assert!(persisted.emitted_ids.contains(&learning.id));
    }

    #[test]
    fn inactive_tool_is_rejected_over_mcp_dispatch() {
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let error = audited_mcp_call(&runtime, "orbit.learning.list", json!({ "model": "codex" }))
            .expect_err("inactive tool is not callable over MCP");
        assert!(error.to_string().contains("tool"));

        let events = runtime
            .list_audit_events(
                None,
                Some("orbit.learning.list".to_string()),
                None,
                None,
                16,
            )
            .expect("list audit events");
        assert_eq!(events.len(), 1, "preflight failure produced one audit row");
        assert_eq!(events[0].subcommand.as_deref(), Some("run-mcp"));
        assert_eq!(events[0].status, AuditEventStatus::Denied);
    }

    #[test]
    fn friction_list_is_active_for_operator_dispatch() {
        let runtime = OrbitRuntime::in_memory().expect("build test runtime");
        let value = audited_mcp_call(&runtime, "orbit.friction.list", json!({ "limit": 1 }))
            .expect("canonical operator triage read");
        assert_eq!(value, json!([]));
    }

    // ORB-00391: the former `mcp_graph_search_accepts_allow_fuzzy_and_returns_result_shape`
    // test exercised the v1 orbit-knowledge `orbit.graph.search` builtin over the
    // host dispatch path. That builtin was decommissioned; the v2 graph search is
    // served by the in-process orbit-graph adapter in orbit-mcp and is covered by
    // `orbit-mcp/src/adapter/tests/graph.rs` (`graph_tools_invoke_in_process_fixture`).

    struct EnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(values: &[(&'static str, Option<&str>)]) -> Self {
            static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
            let lock = LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let saved = values
                .iter()
                .map(|(name, _)| (*name, std::env::var(name).ok()))
                .collect::<Vec<_>>();
            for (name, value) in values {
                // SAFETY: EnvGuard serializes process-wide mutations and restores them on drop.
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
            Self { _lock: lock, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, value) in &self.saved {
                // SAFETY: EnvGuard holds the serialization lock until saved values are restored.
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
    }
}
