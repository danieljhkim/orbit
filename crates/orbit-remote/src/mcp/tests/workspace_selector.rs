use std::collections::BTreeSet;
use std::path::Path;

use chrono::Utc;
use orbit_common::types::{
    McpCapability, ToolSessionContext, Workspace, WorkspaceRegistry, WorkspaceStatus,
};
use orbit_core::runtime::HubCoordinationExecutor;
use orbit_mcp::McpHost;
use serde_json::json;

use super::super::host::BrokerMcpHost;

fn write_identity(root: &Path) {
    std::fs::write(
        root.join("host.toml"),
        "schema_version = 2\nmachine_id = \"hm_hub\"\nhost_id = \"hub\"\ntask_prefix = \"ORB\"\n",
    )
    .expect("host identity");
}

fn save_named_workspaces(root: &Path, names: &[(&str, &str)]) {
    let now = Utc::now();
    let workspaces = names
        .iter()
        .map(|(id, name)| Workspace {
            id: (*id).to_string(),
            name: (*name).to_string(),
            owner_machine_id: Some("hm_hub".to_string()),
            git_remote: None,
            ship_mode: None,
            base_branch: "agent-main".to_string(),
            status: WorkspaceStatus::Active,
            created_at: now,
            updated_at: now,
        })
        .collect();
    crate::workspace_registry::save_registry_to(
        &WorkspaceRegistry {
            workspaces,
            ..Default::default()
        },
        &crate::workspace_registry::registry_path_for(root),
    )
    .expect("workspace registry");
    for (id, name) in names {
        HubCoordinationExecutor::register_workspace(root, *id, *name).expect("task workspace");
    }
}

fn operator_context(workspace: Option<&str>, call_id: &str) -> ToolSessionContext {
    let mut context = ToolSessionContext::trusted_local(
        None,
        Some("hm_hub".to_string()),
        Some("hub".to_string()),
    );
    context.workspace = workspace.map(ToOwned::to_owned);
    context.effective_capabilities = BTreeSet::from([McpCapability::Operator]);
    context.mcp_call_id = Some(call_id.to_string());
    context
}

fn audit_workspace_id(root: &Path, call_id: &str) -> Option<String> {
    let audit_path = orbit_core::config::resolved_audit_db_path(root, root).expect("audit path");
    let connection = rusqlite::Connection::open(audit_path).expect("audit store");
    connection
        .query_row(
            "SELECT workspace_id FROM audit_events WHERE mcp_call_id = ?1",
            [call_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .expect("audit row")
}

#[test]
fn mcp_accepts_registered_name_and_logical_id_and_audits_resolved_id() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(root.path());
    save_named_workspaces(root.path(), &[("ws_orbit", "orbit"), ("ws_other", "other")]);
    let host = BrokerMcpHost::new(root.path().to_path_buf());

    let by_name = host
        .call_tool(
            "orbit.task.add",
            json!({
                "workspace": "orbit",
                "title": "Named",
                "description": "Created via registered name",
                "model": "codex"
            }),
            operator_context(None, "mcall-name"),
        )
        .expect("name selector");
    assert_eq!(by_name["title"], "Named");
    assert_eq!(
        audit_workspace_id(root.path(), "mcall-name").as_deref(),
        Some("ws_orbit")
    );

    let listed = host
        .call_tool(
            "orbit.task.list",
            json!({ "workspace": "ws_orbit", "limit": 5 }),
            operator_context(None, "mcall-id"),
        )
        .expect("logical id selector");
    let ids = listed
        .as_array()
        .expect("task list")
        .iter()
        .filter_map(|task| task.get("id").and_then(|id| id.as_str()))
        .collect::<Vec<_>>();
    assert!(
        ids.contains(&by_name["id"].as_str().expect("created id")),
        "logical id must route to the same workspace: {listed}"
    );
    assert_eq!(
        audit_workspace_id(root.path(), "mcall-id").as_deref(),
        Some("ws_orbit")
    );
}

#[test]
fn mcp_unknown_and_ambiguous_selectors_fail_closed_naming_the_value() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(root.path());
    save_named_workspaces(
        root.path(),
        &[("ws_shared", "alpha"), ("ws_other", "ws_shared")],
    );
    let host = BrokerMcpHost::new(root.path().to_path_buf());

    let unknown = host
        .call_tool(
            "orbit.task.list",
            json!({ "workspace": "no-such-ws", "limit": 1 }),
            operator_context(None, "mcall-unknown"),
        )
        .expect_err("unknown selector");
    let unknown_message = unknown.to_string();
    assert!(unknown_message.contains("no-such-ws"), "{unknown_message}");

    let ambiguous = host
        .call_tool(
            "orbit.task.list",
            json!({ "workspace": "ws_shared", "limit": 1 }),
            operator_context(None, "mcall-ambiguous"),
        )
        .expect_err("ambiguous selector");
    let ambiguous_message = ambiguous.to_string();
    assert!(
        ambiguous_message.contains("ws_shared"),
        "{ambiguous_message}"
    );
    assert!(
        ambiguous_message.contains("ambiguous"),
        "{ambiguous_message}"
    );
}

#[test]
fn mcp_explicit_selector_wins_over_session_and_never_uses_process_cwd() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(root.path());
    save_named_workspaces(root.path(), &[("ws_alpha", "alpha"), ("ws_beta", "beta")]);
    let host = BrokerMcpHost::new(root.path().to_path_buf());

    host.call_tool(
        "orbit.task.add",
        json!({
            "workspace": "alpha",
            "title": "Alpha only",
            "description": "Lives in alpha",
            "model": "codex"
        }),
        operator_context(None, "mcall-seed-alpha"),
    )
    .expect("seed alpha");

    let listed = host
        .call_tool(
            "orbit.task.list",
            json!({ "workspace": "alpha", "limit": 10 }),
            operator_context(Some("beta"), "mcall-explicit-wins"),
        )
        .expect("explicit selector wins");
    let titles: Vec<_> = listed
        .as_array()
        .expect("list")
        .iter()
        .filter_map(|task| task.get("title").and_then(|title| title.as_str()))
        .collect();
    assert!(
        titles.contains(&"Alpha only"),
        "explicit alpha must beat session beta: {listed}"
    );

    let session_only = host
        .call_tool(
            "orbit.task.list",
            json!({ "limit": 10 }),
            operator_context(Some("alpha"), "mcall-session"),
        )
        .expect("session selector");
    let session_titles: Vec<_> = session_only
        .as_array()
        .expect("list")
        .iter()
        .filter_map(|task| task.get("title").and_then(|title| title.as_str()))
        .collect();
    assert!(
        session_titles.contains(&"Alpha only"),
        "session _meta must bind when the call omits workspace: {session_only}"
    );

    let missing = host
        .call_tool(
            "orbit.task.list",
            json!({ "limit": 1 }),
            operator_context(None, "mcall-no-cwd"),
        )
        .expect_err("MCP must not fall back to process cwd");
    let message = missing.to_string();
    assert!(
        message.contains("requires a workspace selector"),
        "{message}"
    );
}

#[test]
fn mcp_relative_path_selector_is_rejected() {
    let root = tempfile::tempdir().expect("global root");
    write_identity(root.path());
    save_named_workspaces(root.path(), &[("ws_orbit", "orbit")]);
    let host = BrokerMcpHost::new(root.path().to_path_buf());

    let error = host
        .call_tool(
            "orbit.task.list",
            json!({ "workspace": "./relative", "limit": 1 }),
            operator_context(None, "mcall-relative"),
        )
        .expect_err("relative path");
    let message = error.to_string();
    assert!(message.contains("./relative"), "{message}");
    assert!(message.contains("must be absolute"), "{message}");
}
