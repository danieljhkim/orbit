#![allow(missing_docs)]

// Content moved from inline #[cfg(test)] mod tests in mcp/mod.rs per ORB-00221.

mod contract;
mod discovery;
mod e1;
mod learning;
mod owner_client;
mod owner_link;
mod proxy;
mod schema;
mod serve;
mod support;
mod transport;

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
    crate::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![workspace],
            ..Default::default()
        },
        &crate::workspace_registry::registry_path_for(root.path()),
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

/// ORB-10727 [ADR-0355]: a replica checkout is exactly a workspace whose owner
/// is another machine, so owner placement — not a separate replica special case
/// — is what refuses it. With no configured route, every owner-placed tool is
/// refused by name, reads included: a silent empty read would misreport the
/// owner's state as "nothing here".
#[test]
fn broker_refuses_every_owner_placed_tool_for_a_remotely_owned_workspace() {
    use chrono::Utc;
    use orbit_common::types::{Workspace, WorkspaceCheckout, WorkspaceRegistry, WorkspaceStatus};

    let root = tempfile::tempdir().expect("global root");
    crate::ensure_host_identity(root.path(), || {
        Ok(crate::NewHostIdentity {
            host_id: "replica".to_string(),
            task_prefix: "RP".to_string(),
        })
    })
    .expect("host identity");
    let workspace = Workspace {
        id: "ws_replica".to_string(),
        name: "Replica".to_string(),
        owner_machine_id: Some("hm_owner".to_string()),
        git_remote: None,
        ship_mode: None,
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    crate::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![workspace.clone()],
            checkouts: vec![WorkspaceCheckout {
                workspace_id: workspace.id.clone(),
                repo_root: root.path().join("checkout"),
                orbit_dir: root.path().join("checkout/.orbit"),
                role: Some(orbit_common::types::WorkspaceCheckoutRole::Replica),
                owner_machine_id: Some("hm_owner".to_string()),
                path_overrides: Vec::new(),
            }],
            ..Default::default()
        },
        &crate::workspace_registry::registry_path_for(root.path()),
    )
    .expect("replica registry");
    let host = BrokerMcpHost::new(root.path().to_path_buf());
    let context = ToolSessionContext::trusted_local(
        Some(workspace.id.clone()),
        Some("hm_replica".to_string()),
        Some("replica".to_string()),
    );

    let mut refused = 0;
    for definition in canonical_mcp_tool_definitions().expect("registered tools") {
        if definition.policy.placement() != McpToolPlacement::Owner
            || definition.policy.scope() != McpToolScope::WorkspaceRequired
        {
            continue;
        }
        let name = definition.schema.name;
        let mut call_context = context.clone();
        call_context.effective_capabilities = BTreeSet::from([*definition
            .policy
            .allowed_capabilities()
            .first()
            .expect("owner-placed capability")]);
        let error = host
            .call_tool(
                &name,
                json!({"workspace": workspace.id, "model": "codex"}),
                call_context,
            )
            .expect_err("every owner-placed tool must be refused without a route");
        let message = error.to_string();
        assert!(message.contains("hm_owner"), "{name}: {message}");
        assert!(
            message.contains("no [[owner]] route"),
            "{name} must name the missing route: {message}"
        );
        refused += 1;
    }
    assert!(refused > 0, "the registry must contain owner-placed tools");
}

#[test]
fn broker_merged_search_tags_duplicate_ids_with_their_workspace() {
    use std::process::Command;

    use chrono::Utc;
    use orbit_common::types::{
        LearningScope, Workspace, WorkspaceCheckout, WorkspaceRegistry, WorkspaceStatus,
    };
    use orbit_store::LearningCreateParams;
    use orbit_store::sqlite::task_registry::{WorkspaceConfig, write_workspace_config};

    let root = tempfile::tempdir().expect("test root");
    let global_root = root.path().join("global");
    std::fs::create_dir_all(&global_root).expect("global root");
    let mut workspaces = Vec::new();
    let mut checkouts = Vec::new();
    let mut learning_ids = Vec::new();

    for id in ["alpha", "beta"] {
        let repo_root = root.path().join(id);
        let initialized = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(root.path())
            .arg(&repo_root)
            .status()
            .expect("initialize workspace repository");
        assert!(initialized.success(), "git init failed for {id}");
        let orbit_dir = repo_root.join(".orbit");
        std::fs::create_dir_all(&orbit_dir).expect("orbit directory");
        write_workspace_config(
            &orbit_dir,
            &WorkspaceConfig {
                schema_version: 1,
                workspace_id: format!("ws_{id}"),
            },
        )
        .expect("workspace config");
        let workspace = Workspace {
            id: id.to_string(),
            name: id.to_string(),
            owner_machine_id: None,
            git_remote: None,
            ship_mode: None,
            base_branch: "agent-main".to_string(),
            status: WorkspaceStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let checkout = WorkspaceCheckout::owner(id.to_string(), repo_root, orbit_dir);
        let runtime = crate::runtime::RemoteRuntimeFactory::open_registered_checkout(
            &global_root,
            &workspace,
            &checkout,
        )
        .expect("workspace runtime");
        let learning = runtime
            .create_learning(LearningCreateParams {
                summary: "shared merged search needle".to_string(),
                scope: LearningScope::default(),
                body: "same learning ID in each workspace".to_string(),
                evidence: Vec::new(),
                created_by: Some("codex".to_string()),
                priority: None,
            })
            .expect("seed learning");
        learning_ids.push(learning.id);
        workspaces.push(workspace);
        checkouts.push(checkout);
    }
    assert_eq!(
        learning_ids[0], learning_ids[1],
        "fixture must duplicate the learning ID"
    );
    crate::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces,
            checkouts,
            ..Default::default()
        },
        &crate::workspace_registry::registry_path_for(&global_root),
    )
    .expect("workspace registry");

    let host = BrokerMcpHost::new(global_root);
    let merged = host
        .call_tool(
            "orbit.search",
            json!({ "query": "shared merged search needle", "all": true }),
            ToolSessionContext::trusted_local(
                None,
                Some("hm_local".to_string()),
                Some("local".to_string()),
            ),
        )
        .expect("merged search");
    let rows = merged["results"].as_array().expect("merged rows");
    let matching = rows
        .iter()
        .filter(|row| row["id"].as_str() == Some(learning_ids[0].as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        matching.len(),
        2,
        "both duplicate learning IDs must be returned"
    );
    let result_workspaces = matching
        .iter()
        .map(|row| row["workspace"].as_str().expect("workspace field"))
        .collect::<BTreeSet<_>>();
    assert_eq!(result_workspaces, BTreeSet::from(["alpha", "beta"]));
    assert!(
        rows.iter().all(|row| row["workspace"].is_string()),
        "every merged result row must identify its owning workspace: {merged}"
    );
    for workspace in ["alpha", "beta"] {
        let shown = host
            .call_tool(
                "orbit.learning.show",
                json!({ "id": learning_ids[0], "workspace": workspace }),
                ToolSessionContext::trusted_local(
                    None,
                    Some("hm_local".to_string()),
                    Some("local".to_string()),
                ),
            )
            .expect("merged row remains addressable through its workspace");
        assert_eq!(shown["id"], learning_ids[0]);
    }

    let scoped = host
        .call_tool(
            "orbit.search",
            json!({
                "query": "shared merged search needle",
                "all": true,
                "workspace": "alpha"
            }),
            ToolSessionContext::trusted_local(
                None,
                Some("hm_local".to_string()),
                Some("local".to_string()),
            ),
        )
        .expect("workspace-scoped search");
    assert!(
        scoped["results"]
            .as_array()
            .expect("scoped rows")
            .iter()
            .all(|row| row.get("workspace").is_none()),
        "workspace-scoped search shape must stay unchanged: {scoped}"
    );
}

#[test]
fn broker_with_context_requires_local_checkout_before_dispatch() {
    use chrono::Utc;
    use orbit_common::types::{Workspace, WorkspaceRegistry, WorkspaceStatus};

    let root = tempfile::tempdir().expect("global root");
    crate::workspace_registry::save_registry_to(
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
        &crate::workspace_registry::registry_path_for(root.path()),
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
    use crate::{NewHostIdentity, ensure_host_identity};
    use chrono::Utc;
    use orbit_common::types::{Workspace, WorkspaceRegistry, WorkspaceStatus};

    let root = tempfile::tempdir().expect("global root");
    ensure_host_identity(root.path(), || {
        Ok(NewHostIdentity {
            host_id: "spoke".to_string(),
            task_prefix: "SP".to_string(),
        })
    })
    .expect("spoke identity");
    crate::workspace_registry::save_registry_to(
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
        &crate::workspace_registry::registry_path_for(root.path()),
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
fn broker_remote_owner_denial_writes_no_local_coordination_state() {
    use crate::{NewHostIdentity, ensure_host_identity};
    use chrono::Utc;
    use orbit_common::types::{Workspace, WorkspaceRegistry, WorkspaceStatus};

    let root = tempfile::tempdir().expect("global root");
    ensure_host_identity(root.path(), || {
        Ok(NewHostIdentity {
            host_id: "spoke".to_string(),
            task_prefix: "SP".to_string(),
        })
    })
    .expect("spoke identity");
    crate::workspace_registry::save_registry_to(
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
        &crate::workspace_registry::registry_path_for(root.path()),
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
        .expect_err("no configured route to the owner");
    let message = error.to_string();
    assert!(message.contains("hm_remote_owner"), "{message}");
    assert!(message.contains("no [[owner]] route"), "{message}");
    assert!(!root.path().join("tasks").exists());
    assert!(!root.path().join("frictions").exists());
}

#[test]
fn broker_capability_denial_precedes_coordination_store_open() {
    use chrono::Utc;
    use orbit_common::types::{McpCapability, Workspace, WorkspaceRegistry, WorkspaceStatus};

    let root = tempfile::tempdir().expect("global root");
    crate::workspace_registry::save_registry_to(
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
        &crate::workspace_registry::registry_path_for(root.path()),
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
}

impl super::learning::LearningSidecarHost for RuntimeMcpHost {
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
    // Trimmed admin/destructive tools — CLI path retains them.
    "orbit.semantic.uninstall",
    "orbit.task.delete",
    "orbit.task.lint",
    "orbit.learning.prune",
    // Agent-surface narrowing: human-decision tools — CLI-only.
    "orbit.task.reject",
    "orbit.friction.resolve",
];

// ORB-00289 + agent-surface narrowing: admin/destructive and triage tools
// (`orbit.semantic.uninstall`, `orbit.task.delete`,
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
    "orbit.task.start",
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
        || name.starts_with("orbit.semantic.")
        || name.starts_with("orbit.docs.")
        || name.starts_with("orbit.learning.")
}

fn is_remote_owned_non_runtime_tool(name: &str) -> bool {
    matches!(name, "orbit.workspace.list" | "orbit.crew.list")
}

#[test]
fn inactive_tools_are_not_in_the_mcp_safe_surface() {
    let safe_names: BTreeSet<String> = safe_mcp_tool_names().into_iter().collect();
    assert_eq!(EXPECTED_INACTIVE_TOOL_NAMES.len(), 20);

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
fn safe_surface_separates_remote_owned_and_runtime_tools() {
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
        .filter(|name| !is_remote_owned_non_runtime_tool(name))
    {
        assert!(
            names.contains(&name),
            "MCP-candidate tool missing from runtime registry: {name}"
        );
    }

    for name in ["orbit.workspace.list", "orbit.crew.list"] {
        assert!(safe_names.contains(name));
        assert!(is_mcp_tool_exposed(name));
        assert!(
            !all_names.contains(name),
            "Remote discovery must not be re-injected into Core run_tool: {name}"
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
        "orbit.graph.sync",
        "orbit.graph.search",
        "orbit.graph.show",
        "orbit.graph.refs",
        "orbit.graph.callees",
        "orbit.graph.impact",
        "orbit.graph.trace",
        "orbit.graph.overview",
        "orbit.graph.implementors",
        "orbit.graph.deps",
    ] {
        assert!(
            !names.contains(name),
            "runtime exposes removed graph MCP tool: {name}"
        );
        assert!(!safe_names.contains(name));
        assert!(!is_mcp_tool_exposed(name));
    }

    for retired in [
        "git.push",
        "github.pr.comment",
        "github.pr.comment.reply",
        "github.pr.comments",
        "github.pr.create",
        "github.pr.list",
        "github.pr.merge",
        "github.pr.review",
        "github.pr.review.comment",
        "github.pr.view",
        "orbit.state.get",
        "orbit.state.set",
    ] {
        assert!(!names.contains(retired));
        assert!(!safe_names.contains(retired));
        assert!(!is_mcp_tool_exposed(retired));
    }

    assert!(!is_mcp_tool_exposed("demo.hello"));
}

#[test]
fn runtime_mcp_host_lists_only_core_registry_backed_safe_tools() {
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
        .filter(|name| !is_remote_owned_non_runtime_tool(name))
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

    // ORB-10325: graph navigation is CLI-only and must not enter any MCP host.
    assert!(
        !listed.iter().any(|name| name.starts_with("orbit.graph.")),
        "host must expose no graph MCP tools, found: {:?}",
        listed
            .iter()
            .filter(|name| name.starts_with("orbit.graph."))
            .collect::<Vec<_>>()
    );
    for name in ["orbit.workspace.list", "orbit.crew.list"] {
        assert!(
            !listed.contains(name),
            "Core runtime host must not expose Remote discovery: {name}"
        );
    }
}

#[derive(Debug, Deserialize)]
struct McpConformanceFixture {
    capabilities: McpConformanceCapabilities,
    placements: McpConformancePlacements,
    scopes: McpConformanceScopes,
    private_connector: McpConformancePrivateConnector,
    owner_schema_digest: McpConformanceOwnerDigest,
    tools: BTreeMap<String, McpConformancePolicy>,
    withdrawn_tools: Vec<McpConformanceWithdrawnTools>,
}

#[derive(Debug, Deserialize)]
struct McpConformancePrivateConnector {
    /// Every connector-private method the v1 contract still negotiates. ADR-0357
    /// and ADR-0358 withdrew both that ever existed, so this must stay empty.
    active: Vec<String>,
    withdrawn: Vec<McpConformanceWithdrawnMethod>,
}

#[derive(Debug, Deserialize)]
struct McpConformanceWithdrawnMethod {
    method: String,
}

#[derive(Debug, Deserialize)]
struct McpConformanceOwnerDigest {
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
    withdrawn_values: Vec<McpConformanceWithdrawnCapability>,
}

#[derive(Debug, Deserialize)]
struct McpConformanceWithdrawnCapability {
    value: McpCapability,
}

#[derive(Debug, Deserialize)]
struct McpConformanceScopes {
    allowed_values: BTreeSet<McpToolScope>,
}

#[derive(Debug, Deserialize)]
struct McpConformancePlacements {
    allowed_values: BTreeSet<McpToolPlacement>,
    withdrawn_values: Vec<McpConformanceWithdrawnPlacement>,
}

#[derive(Debug, Deserialize)]
struct McpConformanceWithdrawnPlacement {
    value: String,
}

#[derive(Debug, Deserialize)]
struct McpConformanceWithdrawnTools {
    names: Vec<String>,
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
            "{ placement: owner, scope: workspace-required, allowed_capabilities: [unknown] }"
        )
        .is_err(),
        "unknown capabilities must fail typed fixture parsing"
    );
    assert!(
        serde_yaml::from_str::<McpConformancePolicy>(
            "{ placement: owner, scope: workspace-required }"
        )
        .is_err(),
        "missing capability policy must fail typed fixture parsing"
    );
    assert!(
        serde_yaml::from_str::<McpConformancePolicy>(
            "{ placement: owner, scope: unknown, allowed_capabilities: [operator] }"
        )
        .is_err(),
        "unknown scopes must fail typed fixture parsing"
    );
    assert!(
        serde_yaml::from_str::<McpConformancePolicy>(
            "{ placement: owner, allowed_capabilities: [operator] }"
        )
        .is_err(),
        "missing scope metadata must fail typed fixture parsing"
    );
    assert!(
        serde_yaml::from_str::<McpConformancePolicy>(
            "{ placement: hub, scope: workspace-required, allowed_capabilities: [operator] }"
        )
        .is_err(),
        "the withdrawn hub placement must no longer parse (ADR-0355)"
    );

    let fixture: McpConformanceFixture = serde_yaml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/design/mcp-bridge/references/conformance-v1.yaml"
    )))
    .expect("frozen MCP conformance fixture must use known typed policy values");
    assert_eq!(
        fixture.capabilities.allowed_values,
        BTreeSet::from([McpCapability::Agent, McpCapability::Operator]),
        "the v1 bridge grants agent and operator only"
    );
    assert_eq!(
        fixture
            .capabilities
            .withdrawn_values
            .iter()
            .map(|withdrawn| withdrawn.value)
            .collect::<Vec<_>>(),
        [McpCapability::Runner]
    );
    assert!(
        fixture
            .capabilities
            .allowed_values
            .iter()
            .all(|capability| capability.is_bridge_v1()),
        "the fixture and the type must agree on the v1 capability set"
    );
    assert_eq!(
        fixture.placements.allowed_values,
        BTreeSet::from([
            McpToolPlacement::Owner,
            McpToolPlacement::LocalDerived,
            McpToolPlacement::Composite,
        ])
    );
    assert_eq!(
        fixture
            .placements
            .withdrawn_values
            .iter()
            .map(|withdrawn| withdrawn.value.as_str())
            .collect::<Vec<_>>(),
        ["hub"]
    );
    assert_eq!(
        fixture.scopes.allowed_values,
        BTreeSet::from([McpToolScope::WorkspaceRequired, McpToolScope::Global])
    );

    // ADR-0357 and ADR-0358 withdrew both connector-private methods; v1
    // negotiates none, and neither may reappear as a tool.
    assert!(
        fixture.private_connector.active.is_empty(),
        "v1 negotiates no connector-private method: {:?}",
        fixture.private_connector.active
    );
    assert_eq!(
        fixture
            .private_connector
            .withdrawn
            .iter()
            .map(|entry| entry.method.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "orbit/private/register-spoke/v1",
            "orbit/private/allocate-knowledge-id/v1",
        ])
    );
    for withdrawn in &fixture.private_connector.withdrawn {
        assert!(
            !fixture.tools.contains_key(&withdrawn.method),
            "a withdrawn private method must never enter the canonical tool matrix: {}",
            withdrawn.method
        );
    }

    assert_eq!(
        fixture.owner_schema_digest.domain_tag,
        super::contract::OWNER_SCHEMA_DOMAIN
    );
    assert_eq!(
        fixture.owner_schema_digest.contract_revision,
        super::contract::MCP_CONTRACT_REVISION
    );
    assert_eq!(
        fixture.owner_schema_digest.canonical_registry_revision,
        super::contract::CANONICAL_MCP_REGISTRY_REVISION
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
        McpToolPolicy::agent_and_operator(McpToolPlacement::Owner),
    )
    .expect("golden definition");
    let vector = &fixture.owner_schema_digest.golden_vector;
    let mut expected_bytes = format!("{}\0", fixture.owner_schema_digest.domain_tag).into_bytes();
    expected_bytes.extend_from_slice(vector.canonical_json.as_bytes());
    assert_eq!(
        super::contract::canonical_owner_schema_bytes(
            std::slice::from_ref(&vector_definition),
            vector.capability
        )
        .expect("canonical golden bytes"),
        expected_bytes
    );
    assert_eq!(
        super::contract::owner_schema_digest(&[vector_definition], vector.capability)
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
        serde_yaml::from_str::<McpToolPolicy>("{ placement: owner, allowed_capabilities: [] }")
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

    // Withdrawn families must be absent from the advertised set, not merely
    // absent from the fixture's positive matrix.
    for entry in &fixture.withdrawn_tools {
        for name in &entry.names {
            assert!(
                !canonical_names.contains(name),
                "withdrawn tool is still advertised: {name}"
            );
        }
    }
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

    // ADR-0358: a withdrawn capability cannot be attached to a policy at all,
    // so the registry cannot drift back to advertising an unreachable tool.
    assert_eq!(
        McpToolPolicy::new(McpToolPlacement::Owner, [McpCapability::Runner]),
        Err(McpToolPolicyError::WithdrawnCapability(
            McpCapability::Runner
        ))
    );
    let operator_only = McpToolPolicy::operator_only(McpToolPlacement::Owner);
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
        AuditEventStatus, LearningInjectionCaps, LearningInjectionState, LearningReminder,
        LearningScope, McpCapability, McpTransport, ToolSessionContext,
    };
    use orbit_core::LearningEvidence;
    use orbit_core::{LearningCreateParams, OrbitError, OrbitRuntime};
    use orbit_mcp::McpHost;
    use serde_json::json;

    use super::super::host::{BrokerMcpHost, audited_mcp_call_with_session_context};
    use super::super::learning::LearningSidecarHost;
    use super::RuntimeMcpHost;

    fn audited_mcp_call(
        runtime: &OrbitRuntime,
        name: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, OrbitError> {
        audited_mcp_call_with_session_context(
            runtime,
            name,
            input,
            ToolSessionContext::trusted_local(None, None, None),
        )
    }

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
        let result = audited_mcp_call(&runtime, "demo.unknown", json!({}));
        assert!(
            result.is_err(),
            "preflight rejects unknown / unexposed tool"
        );

        let events = runtime
            .list_audit_events(None, Some("demo.unknown".to_string()), None, None, 16)
            .expect("list audit events");
        assert_eq!(events.len(), 1, "preflight failure produced one audit row");
        let row = &events[0];
        assert_eq!(row.command, "tool");
        assert_eq!(row.subcommand.as_deref(), Some("run-mcp"));
        assert_eq!(row.tool_name.as_deref(), Some("demo.unknown"));
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
    fn broker_executes_local_registry_discovery_without_session_workspace() {
        use chrono::Utc;
        use orbit_common::types::{Workspace, WorkspaceRegistry, WorkspaceStatus};

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
        std::fs::write(
            global_root.join("host.toml"),
            "schema_version = 2\nmachine_id = \"hm_hub\"\nhost_id = \"hub\"\ntask_prefix = \"ORB\"\n",
        )
        .expect("host identity");
        let now = Utc::now();
        crate::workspace_registry::save_registry_to(
            &WorkspaceRegistry {
                workspaces: vec![Workspace {
                    id: "ws_local".to_string(),
                    name: "local".to_string(),
                    owner_machine_id: Some("hm_hub".to_string()),
                    git_remote: None,
                    ship_mode: None,
                    base_branch: "agent-main".to_string(),
                    status: WorkspaceStatus::Active,
                    created_at: now,
                    updated_at: now,
                }],
                ..WorkspaceRegistry::default()
            },
            &crate::workspace_registry::registry_path_for(&global_root),
        )
        .expect("local workspace registry");
        let runtime = crate::runtime::RemoteRuntimeFactory::open_resolved_checkout(
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
        let host = BrokerMcpHost::new(global_root);
        let call = |tool_name: &str, call_id: &str| {
            let mut context = ToolSessionContext::trusted_local(
                None,
                Some("hm_hub".to_string()),
                Some("hub".to_string()),
            );
            context.effective_capabilities = BTreeSet::from([McpCapability::Operator]);
            context.origin_session_id = Some("mcp-session-local-discovery".to_string());
            context.mcp_call_id = Some(call_id.to_string());
            host.call_tool(tool_name, json!({}), context)
                .expect("global registry discovery succeeds")
        };

        let workspaces = call("orbit.workspace.list", "mcall-workspace-list");
        for value in [&workspaces] {
            assert_eq!(value["machine_id"], "hm_hub");
            assert!(value.get("hub_machine_id").is_none());
            assert!(value.get("registry_revision").is_none());
            let serialized = serde_json::to_string(value).expect("serialize result");
            for forbidden in ["repo_root", "orbit_dir", "path_overrides", "crews", "model"] {
                assert!(
                    !serialized.contains(forbidden),
                    "global discovery leaked {forbidden}: {serialized}"
                );
            }
        }
        let workspace_rows = workspaces["workspaces"].as_array().expect("workspace rows");
        assert_eq!(workspace_rows.len(), 1);
        assert_eq!(workspace_rows[0]["id"], "ws_local");
        assert_eq!(workspace_rows[0]["owner_machine_id"], "hm_hub");

        {
            let (tool_name, call_id) = ("orbit.workspace.list", "mcall-workspace-list");
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
    fn in_process_extension_dispatch_records_success_and_failure_audit_rows() {
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
        success_context.origin_session_id = Some("mcp-session-extension".to_string());
        success_context.mcp_call_id = Some("mcall-extension-success".to_string());
        host.call_in_process_tool(
            "orbit.search",
            json!({ "query": "dispatch", "model": "codex" }),
            success_context,
            &mut success_dispatch,
        )
        .expect("allowlisted extension call succeeds");

        let mut failure_dispatch =
            |_input: serde_json::Value, _session_context: ToolSessionContext| {
                Err(OrbitError::InvalidInput("invalid selector".to_string()))
            };
        let mut failure_context = ToolSessionContext::trusted_local(
            Some("ws_orbit".to_string()),
            Some("hm_local".to_string()),
            Some("local-host".to_string()),
        );
        failure_context.origin_session_id = Some("mcp-session-extension".to_string());
        failure_context.mcp_call_id = Some("mcall-extension-failure".to_string());
        host.call_in_process_tool(
            "orbit.task.show",
            json!({ "id": "ORB-invalid", "model": "codex" }),
            failure_context,
            &mut failure_dispatch,
        )
        .expect_err("extension implementation failure propagates");

        for (name, expected_status) in [
            ("orbit.search", AuditEventStatus::Success),
            ("orbit.task.show", AuditEventStatus::Failure),
        ] {
            let events = runtime
                .list_audit_events(None, Some(name.to_string()), None, None, 16)
                .expect("list extension audit events");
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
            assert_eq!(
                row.origin_session_id.as_deref(),
                Some("mcp-session-extension")
            );
            assert!(row.mcp_call_id.as_deref().is_some_and(|call_id| {
                call_id == "mcall-extension-success" || call_id == "mcall-extension-failure"
            }));
        }
    }

    #[test]
    fn removed_graph_tool_is_rejected_before_in_process_dispatch() {
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
                "orbit.graph.search",
                json!({ "model": "codex" }),
                Default::default(),
                &mut dispatch,
            )
            .expect_err("removed graph tool is rejected");
        assert!(matches!(error, OrbitError::NotFound { .. }));
        assert!(!called, "rejected graph implementation must not run");

        let events = runtime
            .list_audit_events(None, Some("orbit.graph.search".to_string()), None, None, 16)
            .expect("list removed graph preflight audit event");
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
                tags: Vec::new(),
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
