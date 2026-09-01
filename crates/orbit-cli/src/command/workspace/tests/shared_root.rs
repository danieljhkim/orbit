use orbit_registry::workspace_registry;
use tempfile::tempdir;

use crate::tests::env_isolation::EnvGuard;

use super::super::init::WorkspaceInitArgs;
use super::super::show::registered_workspace_for_repo_root;

fn init_args(name: &str) -> WorkspaceInitArgs {
    WorkspaceInitArgs {
        name: Some(name.to_string()),
        base_branch: None,
        ship_mode: None,
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force: false,
    }
}

#[test]
fn shared_root_registers_each_checkout_and_show_resolves_by_repo_root() {
    let fixture = tempdir().expect("fixture");
    // The registry records checkout paths as the process resolves them, and
    // `set_current_dir` below resolves symlinks. On macOS a temp dir arrives as
    // `/var/...` but resolves to `/private/var/...`, so fixtures built from the
    // unresolved path would never equal what the registry stored.
    let fixture_root = std::fs::canonicalize(fixture.path()).expect("canonical fixture root");
    let home = fixture_root.join("home");
    let shared_root = fixture_root.join("shared-orbit");
    let repo_alpha = fixture_root.join("repo-alpha");
    let repo_beta = fixture_root.join("repo-beta");
    let unregistered = fixture_root.join("repo-unregistered");
    for directory in [&home, &repo_alpha, &repo_beta, &unregistered] {
        std::fs::create_dir_all(directory).expect("fixture directory");
    }

    let _env = EnvGuard::acquire().home(&home).cwd(&repo_alpha);
    init_args("alpha")
        .execute_without_runtime(Some(&shared_root))
        .expect("first shared-root workspace init");

    std::env::set_current_dir(&repo_beta).expect("enter second repo");
    init_args("beta")
        .execute_without_runtime(Some(&shared_root))
        .expect("second shared-root workspace init");
    let mut beta_reinit = init_args("beta");
    beta_reinit.force = true;
    beta_reinit
        .execute_without_runtime(Some(&shared_root))
        .expect("shared-root workspace reconciliation");

    let registry = workspace_registry::load_registry_from(&workspace_registry::registry_path_for(
        &shared_root,
    ))
    .expect("shared registry");
    assert_eq!(registry.workspaces.len(), 2);
    assert_eq!(registry.checkouts.len(), 2);
    assert!(registry.checkouts.iter().all(|checkout| {
        checkout.orbit_dir == shared_root
            && (checkout.repo_root == repo_alpha || checkout.repo_root == repo_beta)
    }));

    let (workspace, checkout) = registered_workspace_for_repo_root(&registry, Some(&repo_beta))
        .expect("beta checkout resolves");
    assert_eq!(workspace.id, "ws_beta");
    assert_eq!(checkout.repo_root, repo_beta);
    assert!(registered_workspace_for_repo_root(&registry, Some(&unregistered)).is_none());

    let identity =
        std::fs::read_to_string(shared_root.join("config.yaml")).expect("shared runtime identity");
    assert!(identity.contains("workspace_id: ws_alpha"), "{identity}");
    assert!(!identity.contains("workspace_id: ws_beta"), "{identity}");
}
