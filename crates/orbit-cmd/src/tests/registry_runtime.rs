use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{
    NotFoundKind, OrbitError, Workspace, WorkspaceCheckout, WorkspaceCheckoutRole,
    WorkspaceRegistry, WorkspaceStatus,
};
use orbit_core::OrbitRuntime;
use orbit_core::runtime::OrbitRuntimeRoots;
use orbit_store::sqlite::task_registry::{WorkspaceConfig, write_workspace_config};
use serde_json::{Value, json};

use orbit_registry::workspace_registry::{registry_path_for, save_registry_to};

use crate::registry_runtime::{
    RegisteredRuntimeFactory, resolved_workspace_binding, select_workspace_for_cwd_and_roots,
    workspace_runtime_binding,
};

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

    let runtime =
        RegisteredRuntimeFactory::open_registered_checkout(&global, &workspace, &checkout)
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

#[test]
fn explicit_shared_root_selects_checkout_by_cwd_and_does_not_fall_back() {
    let root = tempfile::tempdir().expect("root");
    let shared = root.path().join("shared");
    let alpha_repo = root.path().join("alpha");
    let beta_repo = root.path().join("beta");
    let unregistered_repo = root.path().join("unregistered");
    for directory in [&shared, &alpha_repo, &beta_repo, &unregistered_repo] {
        std::fs::create_dir_all(directory).expect("fixture directory");
    }
    write_workspace_config(
        &shared,
        &WorkspaceConfig {
            schema_version: 1,
            workspace_id: "ws_shared_runtime".to_string(),
        },
    )
    .expect("workspace config");

    let mut alpha = workspace("ws_alpha", "pr");
    alpha.name = "alpha".to_string();
    let mut beta = workspace("ws_beta", "local");
    beta.name = "beta".to_string();
    let alpha_checkout = WorkspaceCheckout::owner(alpha.id.clone(), alpha_repo, shared.clone());
    let beta_checkout =
        WorkspaceCheckout::owner(beta.id.clone(), beta_repo.clone(), shared.clone());
    save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![alpha, beta],
            checkouts: vec![alpha_checkout, beta_checkout],
            ..Default::default()
        },
        &registry_path_for(&shared),
    )
    .expect("shared registry");
    let roots = OrbitRuntimeRoots {
        global_root: shared.clone(),
        shared_root: shared.clone(),
        local_root: shared,
    };

    let selected = select_workspace_for_cwd_and_roots(&beta_repo, &roots)
        .expect("select beta")
        .expect("registered beta");
    assert_eq!(selected.workspace.id, "ws_beta");
    assert_eq!(selected.checkout.repo_root, beta_repo);
    assert_eq!(
        workspace_runtime_binding(&selected.workspace, &selected.checkout)
            .expect("runtime binding")
            .workspace_id,
        "ws_shared_runtime"
    );

    assert!(
        select_workspace_for_cwd_and_roots(&unregistered_repo, &roots)
            .expect("unregistered selection")
            .is_none(),
        "a shared orbit_dir must not select the first registered checkout"
    );
}

#[test]
fn registered_checkout_task_creation_uses_host_task_prefix() {
    let root = tempfile::tempdir().expect("root");
    let global = root.path().join("global");
    let repo = root.path().join("repo");
    let orbit_dir = repo.join(".orbit");
    std::fs::create_dir_all(&global).expect("global");
    std::fs::create_dir_all(&orbit_dir).expect("orbit dir");
    std::fs::write(
        global.join("host.toml"),
        "schema_version = 2\nmachine_id = \"hm_runtime_test\"\nhost_id = \"runtime-test\"\ntask_prefix = \"DE\"\n",
    )
    .expect("host identity");
    write_workspace_config(
        &orbit_dir,
        &WorkspaceConfig {
            schema_version: 1,
            workspace_id: "ws_prefixed_runtime".to_string(),
        },
    )
    .expect("workspace config");
    let workspace = workspace("logical-prefixed", "local");
    let checkout = WorkspaceCheckout::owner(workspace.id.clone(), repo, orbit_dir);
    let runtime =
        RegisteredRuntimeFactory::open_registered_checkout(&global, &workspace, &checkout)
            .expect("bound runtime");

    let task = runtime
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Prefix-aware task",
                "description": "Mint through the normal task creation surface.",
                "workspace": "."
            }),
            Some("codex".to_string()),
            Some(orbit_common::test_fixtures::TEST_CODEX_MODEL.to_string()),
        )
        .expect("task creation");
    assert_eq!(task["id"], "DE-00000");
}

#[test]
fn replica_runtime_refuses_task_writes_and_hides_coordination_reads() {
    let root = tempfile::tempdir().expect("root");
    let global = root.path().join("global");
    let repo = root.path().join("replica");
    let orbit_dir = repo.join(".orbit");
    std::fs::create_dir_all(&global).expect("global");
    std::fs::create_dir_all(&orbit_dir).expect("orbit directory");
    write_workspace_config(
        &orbit_dir,
        &WorkspaceConfig {
            schema_version: 1,
            workspace_id: "ws_runtime".to_string(),
        },
    )
    .expect("workspace config");
    let workspace = workspace("logical-replica", "local");
    let checkout = WorkspaceCheckout {
        workspace_id: workspace.id.clone(),
        repo_root: repo,
        orbit_dir,
        role: Some(WorkspaceCheckoutRole::Replica),
        owner_machine_id: Some("hm_owner".to_string()),
        path_overrides: Vec::new(),
    };
    let runtime =
        RegisteredRuntimeFactory::open_registered_checkout(&global, &workspace, &checkout)
            .expect("replica runtime");

    let error = runtime
        .add_task(orbit_core::command::task::TaskAddParams {
            title: "must not fork".to_string(),
            ..Default::default()
        })
        .expect_err("replica task write must fail closed");
    assert!(error.to_string().contains("hm_owner"), "{error}");
    assert!(
        runtime
            .list_tasks()
            .expect("empty replica task list")
            .is_empty()
    );
}

struct DualWorkspaceFixture {
    _root: tempfile::TempDir,
    alpha: OrbitRuntime,
    beta_repo: PathBuf,
    beta_task_id: String,
}

fn execute_cli_tool(
    runtime: &OrbitRuntime,
    name: &str,
    mut input: Value,
) -> Result<Value, OrbitError> {
    let bound = RegisteredRuntimeFactory::bind_cli_tool_workspace(runtime, &mut input)?;
    bound.as_ref().unwrap_or(runtime).execute_tool_command(
        name,
        input,
        Some("codex".to_string()),
        Some(orbit_common::test_fixtures::TEST_CODEX_MODEL.to_string()),
    )
}

fn dual_workspace_fixture() -> DualWorkspaceFixture {
    use orbit_common::types::WorkspaceRegistry;

    let root = tempfile::tempdir().expect("root");
    let global = root.path().join("global");
    std::fs::create_dir_all(&global).expect("global");
    std::fs::write(
        global.join("host.toml"),
        "schema_version = 2\nmachine_id = \"hm_cli_bind\"\nhost_id = \"cli-bind\"\ntask_prefix = \"ORB\"\n",
    )
    .expect("host identity");

    let (ws_alpha, checkout_alpha) =
        registered_workspace(root.path(), "ws_alpha", "alpha", "hm_cli_bind");
    let (ws_beta, checkout_beta) =
        registered_workspace(root.path(), "ws_beta", "beta", "hm_cli_bind");
    save_registry_to(
        &WorkspaceRegistry {
            workspaces: vec![ws_alpha.clone(), ws_beta.clone()],
            checkouts: vec![checkout_alpha.clone(), checkout_beta.clone()],
            ..Default::default()
        },
        &registry_path_for(&global),
    )
    .expect("workspace registry");

    let alpha =
        RegisteredRuntimeFactory::open_registered_checkout(&global, &ws_alpha, &checkout_alpha)
            .expect("alpha runtime");
    let beta =
        RegisteredRuntimeFactory::open_registered_checkout(&global, &ws_beta, &checkout_beta)
            .expect("beta runtime");
    let created = beta
        .execute_tool_command(
            "orbit.task.add",
            json!({
                "title": "Beta-only task",
                "description": "Lives in the beta workspace.",
                "workspace": checkout_beta.repo_root
            }),
            Some("codex".to_string()),
            Some(orbit_common::test_fixtures::TEST_CODEX_MODEL.to_string()),
        )
        .expect("seed beta task");
    DualWorkspaceFixture {
        _root: root,
        alpha,
        beta_repo: checkout_beta.repo_root,
        beta_task_id: created["id"].as_str().expect("created task id").to_string(),
    }
}

fn registered_workspace(
    root: &Path,
    id: &str,
    name: &str,
    owner_machine_id: &str,
) -> (Workspace, WorkspaceCheckout) {
    let repo = root.join(name);
    let orbit_dir = repo.join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("orbit dir");
    write_workspace_config(
        &orbit_dir,
        &WorkspaceConfig {
            schema_version: 1,
            workspace_id: id.to_string(),
        },
    )
    .expect("workspace config");
    let workspace = Workspace {
        id: id.to_string(),
        name: name.to_string(),
        owner_machine_id: Some(owner_machine_id.to_string()),
        git_remote: None,
        ship_mode: Some("local".to_string()),
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let checkout = WorkspaceCheckout::owner(id.to_string(), repo, orbit_dir);
    (workspace, checkout)
}

fn unsupported_workspace_message(error: OrbitError, selector: &str) -> String {
    match error {
        OrbitError::InvalidInput(message) => {
            assert!(
                message.contains(selector),
                "error must name the rejected workspace '{selector}': {message}"
            );
            message
        }
        other => panic!("expected InvalidInput, got {other}"),
    }
}

#[test]
fn cli_tool_run_lists_the_named_workspace_not_the_cwd_runtime() {
    let fixture = dual_workspace_fixture();
    let cwd_list = execute_cli_tool(&fixture.alpha, "orbit.task.list", json!({ "limit": 10 }))
        .expect("list without workspace stays on alpha");
    assert!(
        !task_ids(&cwd_list).contains(&fixture.beta_task_id),
        "cwd-bound list must not silently return the other workspace: {cwd_list}"
    );

    let named = execute_cli_tool(
        &fixture.alpha,
        "orbit.task.list",
        json!({ "workspace": "beta", "limit": 10 }),
    )
    .expect("list should rebind to the named workspace");
    assert!(
        task_ids(&named).contains(&fixture.beta_task_id),
        "named workspace list must return beta tasks: {named}"
    );

    let by_path = execute_cli_tool(
        &fixture.alpha,
        "orbit.task.list",
        json!({
            "workspace": fixture.beta_repo,
            "limit": 10
        }),
    )
    .expect("list should rebind to the checkout path");
    assert!(
        task_ids(&by_path).contains(&fixture.beta_task_id),
        "absolute checkout path must return beta tasks: {by_path}"
    );

    let by_id = execute_cli_tool(
        &fixture.alpha,
        "orbit.task.list",
        json!({ "workspace": "ws_beta", "limit": 10 }),
    )
    .expect("list should rebind to the logical id");
    assert!(
        task_ids(&by_id).contains(&fixture.beta_task_id),
        "logical workspace id must return beta tasks: {by_id}"
    );
}

#[test]
fn cli_tool_run_fails_closed_on_unresolvable_workspace_for_read_and_write() {
    let fixture = dual_workspace_fixture();
    const BOGUS: &str = "bogus-nonexistent-xyz";

    let list_error = execute_cli_tool(
        &fixture.alpha,
        "orbit.task.list",
        json!({ "workspace": BOGUS, "limit": 2 }),
    )
    .expect_err("unresolvable workspace must not list the cwd workspace");
    unsupported_workspace_message(list_error, BOGUS);

    let add_error = execute_cli_tool(
        &fixture.alpha,
        "orbit.task.add",
        json!({
            "title": "must not land in cwd",
            "description": "unresolvable workspace must fail closed",
            "workspace": BOGUS
        }),
    )
    .expect_err("unresolvable workspace must not create a cwd task");
    unsupported_workspace_message(add_error, BOGUS);

    let after = execute_cli_tool(&fixture.alpha, "orbit.task.list", json!({ "limit": 10 }))
        .expect("cwd list after failed add");
    assert!(
        task_ids(&after).is_empty(),
        "failed write must not create a task in the cwd workspace: {after}"
    );
}

#[test]
fn cli_tool_run_write_rebounds_to_the_named_workspace() {
    let fixture = dual_workspace_fixture();
    let created = execute_cli_tool(
        &fixture.alpha,
        "orbit.task.add",
        json!({
            "title": "Filed onto beta by name",
            "description": "CLI workspace selector must rebind writes.",
            "workspace": "beta"
        }),
    )
    .expect("add should rebind to the named workspace");
    let created_id = created["id"].as_str().expect("created id").to_string();

    let alpha_list = execute_cli_tool(&fixture.alpha, "orbit.task.list", json!({ "limit": 10 }))
        .expect("alpha list");
    assert!(
        !task_ids(&alpha_list).contains(&created_id),
        "named-workspace write must not land in the cwd workspace: {alpha_list}"
    );

    let beta_list = execute_cli_tool(
        &fixture.alpha,
        "orbit.task.list",
        json!({ "workspace": "beta", "limit": 10 }),
    )
    .expect("beta list");
    assert!(
        task_ids(&beta_list).contains(&created_id),
        "named-workspace write must be visible on the target workspace: {beta_list}"
    );
}

#[test]
fn initialize_with_workspace_selector_binds_the_named_checkout() {
    let fixture = dual_workspace_fixture();
    let global = fixture.alpha.global_root();
    let runtime = RegisteredRuntimeFactory::initialize_with_overrides(Some(&global), Some("beta"))
        .expect("selector should bind beta");
    let listed = runtime
        .execute_tool_command(
            "orbit.task.list",
            json!({ "limit": 10 }),
            Some("codex".to_string()),
            Some(orbit_common::test_fixtures::TEST_CODEX_MODEL.to_string()),
        )
        .expect("list beta");
    assert!(
        task_ids(&listed).contains(&fixture.beta_task_id),
        "initialize --workspace beta must open the beta checkout: {listed}"
    );

    let unknown = match RegisteredRuntimeFactory::initialize_with_overrides(
        Some(&global),
        Some("no-such-workspace"),
    ) {
        Ok(_) => panic!("unknown selector must fail closed"),
        Err(error) => error,
    };
    unsupported_workspace_message(unknown, "no-such-workspace");
}

fn task_ids(value: &Value) -> Vec<String> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|task| {
            task.get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect()
}
