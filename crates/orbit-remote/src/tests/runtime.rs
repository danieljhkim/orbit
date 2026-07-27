use chrono::Utc;
use orbit_common::types::{
    NotFoundKind, OrbitError, Workspace, WorkspaceCheckout, WorkspaceStatus,
};
use orbit_store::sqlite::task_registry::{WorkspaceConfig, write_workspace_config};
use serde_json::json;

use crate::runtime::{RemoteRuntimeFactory, resolved_workspace_binding, workspace_runtime_binding};

fn workspace(id: &str, ship_mode: &str) -> Workspace {
    Workspace {
        id: id.to_string(),
        name: "orbit".to_string(),
        owner_machine_id: Some("hm_owner".to_string()),
        git_remote: None,
        ship_mode: Some(ship_mode.to_string()),
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn binding_preserves_logical_and_runtime_ids_and_ship_mode() {
    let root = tempfile::tempdir().expect("root");
    let repo = root.path().join("repo");
    let orbit_dir = repo.join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("orbit dir");
    write_workspace_config(
        &orbit_dir,
        &WorkspaceConfig {
            schema_version: 1,
            workspace_id: "ws_runtime_config".to_string(),
        },
    )
    .expect("workspace config");
    let workspace = workspace("logical-abc123", "pr");
    let checkout = WorkspaceCheckout::owner(workspace.id.clone(), repo.clone(), orbit_dir.clone());

    let resolved = resolved_workspace_binding(&workspace, &checkout).expect("resolved binding");
    assert_eq!(resolved.logical_workspace_id, "logical-abc123");
    assert_eq!(resolved.runtime.workspace_id, "ws_runtime_config");
    assert_eq!(resolved.runtime.repo_root, repo);
    assert_eq!(resolved.runtime.ship_mode.as_input_value(), "pr");

    let direct = workspace_runtime_binding(&workspace, &checkout).expect("core binding");
    assert_eq!(direct, resolved.runtime);
}

#[test]
fn registered_checkout_opens_a_bound_runtime() {
    let root = tempfile::tempdir().expect("root");
    let global = root.path().join("global");
    let repo = root.path().join("repo");
    let orbit_dir = repo.join(".orbit");
    std::fs::create_dir_all(&global).expect("global");
    std::fs::create_dir_all(&orbit_dir).expect("orbit dir");
    write_workspace_config(
        &orbit_dir,
        &WorkspaceConfig {
            schema_version: 1,
            workspace_id: "ws_runtime".to_string(),
        },
    )
    .expect("workspace config");
    let workspace = workspace("logical-abc123", "local");
    let checkout = WorkspaceCheckout::owner(workspace.id.clone(), repo.clone(), orbit_dir);

    let runtime = RemoteRuntimeFactory::open_registered_checkout(&global, &workspace, &checkout)
        .expect("bound runtime");
    let binding = runtime
        .workspace_runtime_binding()
        .expect("runtime binding");
    assert_eq!(binding.workspace_id, "ws_runtime");
    assert_eq!(binding.repo_root, repo);
    assert_eq!(binding.ship_mode.as_input_value(), "local");

    assert!(matches!(
        runtime.run_tool("orbit.workspace.list", json!({})),
        Err(OrbitError::NotFound {
            kind: NotFoundKind::Tool,
            ..
        })
    ));
}
