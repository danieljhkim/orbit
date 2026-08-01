use tempfile::tempdir;

use chrono::Utc;
use orbit_common::types::{
    OverlapPolicy, RoutineTarget, Workspace, WorkspaceCheckout, WorkspaceRegistry, WorkspaceStatus,
    parse_routine_yaml,
};
use orbit_remote::workspace_registry;

use crate::tests::env_isolation::EnvGuard;

use super::super::init::{WorkspaceInitArgs, canonical_workspace_id};
use super::super::list::format_workspace_list;
use super::super::role::CliCheckoutRole;
use super::super::show::format_workspace_show;
use super::super::support::orbit_gitignore_block;

#[test]
fn workspace_reinit_requires_force_and_force_reconciles_matching_registration() {
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    let global = home.path().join(".orbit");
    std::fs::create_dir_all(&global).expect("create global orbit");
    // A host identity must exist for workspace init to seed default routines
    // (which creates `.orbit/routines/`); `orbit init` owns its creation.
    std::fs::write(
        global.join("host.toml"),
        "schema_version = 1\nmachine_id = \"hm_reinit\"\nhost_id = \"reinit-host\"\nmode = \"standalone\"\n",
    )
    .expect("write host identity");

    let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());

    let init = |base_branch: Option<&str>, ship_mode: Option<&str>, force| WorkspaceInitArgs {
        name: Some("reinit-merge".to_string()),
        base_branch: base_branch.map(str::to_string),
        ship_mode: ship_mode.map(str::to_string),
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force,
    };

    init(Some("agent-main"), None, false)
        .execute_without_runtime(None)
        .expect("initial workspace init");

    let registry_path = global.join("workspaces.json");
    let original = workspace_registry::load_registry_from(&registry_path)
        .expect("load initial registry")
        .workspaces
        .into_iter()
        .next()
        .expect("registered workspace");
    let authored_routine = r#"schemaVersion: 1
name: custom-ship-sweep
description: operator-authored routine
enabled: true
hosts: [custom-host]
trigger:
  cron: "7 3 * * *"
  missed_run: catch_up_once
target: job:workspace_ship_pipeline
policy:
  timeout_minutes: 77
  overlap: allow
"#;
    let routine_path = workspace.path().join(".orbit/routines/ship_sweep.yaml");
    std::fs::write(&routine_path, authored_routine).expect("author routine");
    let auto_task_path = workspace
        .path()
        .join(".orbit/auto_tasks/friction-curation.yaml");
    let seeded_auto_task =
        std::fs::read_to_string(&auto_task_path).expect("read seeded friction-curation definition");
    assert!(
        seeded_auto_task.contains("enabled: false"),
        "workspace initialization must not enable a default auto-task"
    );
    let authored_auto_task = "operator-authored auto-task definition\n";
    std::fs::write(&auto_task_path, authored_auto_task).expect("author auto-task definition");

    let registry_bytes = std::fs::read_to_string(&registry_path).expect("read protected registry");
    let identity_path = workspace.path().join(".orbit/config.yaml");
    let identity_bytes = std::fs::read_to_string(&identity_path).expect("read protected identity");
    let error = init(None, Some("pr"), false)
        .execute_without_runtime(None)
        .expect_err("existing checkout must require force")
        .to_string();
    assert!(error.contains("already exists"), "unexpected: {error}");
    assert_eq!(
        std::fs::read_to_string(&registry_path).expect("read registry"),
        registry_bytes
    );
    assert_eq!(
        std::fs::read_to_string(&identity_path).expect("read identity"),
        identity_bytes
    );

    init(None, Some("pr"), true)
        .execute_without_runtime(None)
        .expect("re-init with explicit PR mode");
    let after_ship_mode = workspace_registry::load_registry_from(&registry_path)
        .expect("load registry after ship mode update")
        .workspaces
        .into_iter()
        .next()
        .expect("registered workspace");
    assert_eq!(after_ship_mode.id, original.id);
    assert_eq!(after_ship_mode.created_at, original.created_at);
    assert_eq!(after_ship_mode.base_branch, "agent-main");
    assert_eq!(after_ship_mode.ship_mode.as_deref(), Some("pr"));
    assert_eq!(
        orbit_core::resolved_ship_mode(&after_ship_mode).as_input_value(),
        "pr",
        "workspace_ship_pipeline must receive the persisted PR mode"
    );
    assert_eq!(
        std::fs::read_to_string(&routine_path).expect("read authored routine"),
        authored_routine
    );
    assert_eq!(
        std::fs::read_to_string(&auto_task_path).expect("read authored auto-task definition"),
        authored_auto_task,
        "workspace --force reconciliation must preserve an authored auto-task definition"
    );

    init(None, None, true)
        .execute_without_runtime(None)
        .expect("re-init with omitted registration options");
    let after_omitted = workspace_registry::load_registry_from(&registry_path)
        .expect("load registry after omitted options")
        .workspaces
        .into_iter()
        .next()
        .expect("registered workspace");
    assert_eq!(after_omitted.id, original.id);
    assert_eq!(after_omitted.created_at, original.created_at);
    assert_eq!(after_omitted.base_branch, "agent-main");
    assert_eq!(after_omitted.ship_mode.as_deref(), Some("pr"));
    assert_eq!(
        std::fs::read_to_string(&routine_path).expect("read authored routine"),
        authored_routine
    );

    init(Some("release"), None, true)
        .execute_without_runtime(None)
        .expect("re-init with explicit base branch");
    let after_base_branch = workspace_registry::load_registry_from(&registry_path)
        .expect("load registry after base branch update")
        .workspaces
        .into_iter()
        .next()
        .expect("registered workspace");
    assert_eq!(after_base_branch.base_branch, "release");
    assert_eq!(after_base_branch.ship_mode.as_deref(), Some("pr"));

    let before_invalid = after_base_branch.clone();
    let error = init(None, Some("invalid"), true)
        .execute_without_runtime(None)
        .expect_err("invalid ship mode must fail closed");
    assert!(error.to_string().contains("unknown ship mode 'invalid'"));
    let after_invalid = workspace_registry::load_registry_from(&registry_path)
        .expect("load registry after invalid mode")
        .workspaces
        .into_iter()
        .next()
        .expect("registered workspace");
    assert_eq!(after_invalid, before_invalid);
}

#[test]
fn workspace_init_rejects_existing_checkout_path_with_different_id_without_force() {
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    let global = home.path().join(".orbit");
    std::fs::create_dir_all(&global).expect("create global orbit");
    std::fs::write(
        global.join("host.toml"),
        "schema_version = 1\nmachine_id = \"hm_path_collision\"\nhost_id = \"path-collision\"\nmode = \"standalone\"\n",
    )
    .expect("write host identity");
    let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());
    let args = |name: &str| WorkspaceInitArgs {
        name: Some(name.to_string()),
        base_branch: Some("agent-main".to_string()),
        ship_mode: None,
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force: false,
    };
    args("path-owner")
        .execute_without_runtime(None)
        .expect("initial workspace init");
    let registry_path = global.join("workspaces.json");
    let identity_path = workspace.path().join(".orbit/config.yaml");
    let registry_bytes = std::fs::read_to_string(&registry_path).expect("read protected registry");
    let identity_bytes = std::fs::read_to_string(&identity_path).expect("read protected identity");

    let error = args("different-id")
        .execute_without_runtime(None)
        .expect_err("existing checkout path must require force")
        .to_string();
    assert!(error.contains("already exists"), "unexpected: {error}");
    assert_eq!(
        std::fs::read_to_string(&registry_path).expect("read registry"),
        registry_bytes
    );
    assert_eq!(
        std::fs::read_to_string(&identity_path).expect("read identity"),
        identity_bytes
    );
}

#[test]
fn workspace_init_rejects_existing_durable_id_without_force() {
    let first = tempdir().expect("first workspace tempdir");
    let second = tempdir().expect("second workspace tempdir");
    let home = tempdir().expect("home tempdir");
    let global = home.path().join(".orbit");
    std::fs::create_dir_all(&global).expect("create global orbit");
    std::fs::write(
        global.join("host.toml"),
        "schema_version = 1\nmachine_id = \"hm_id_collision\"\nhost_id = \"id-collision\"\nmode = \"standalone\"\n",
    )
    .expect("write host identity");
    let _env = EnvGuard::acquire().home(home.path()).cwd(first.path());
    let args = |force| WorkspaceInitArgs {
        name: Some("shared-id".to_string()),
        base_branch: Some("agent-main".to_string()),
        ship_mode: None,
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force,
    };
    args(false)
        .execute_without_runtime(None)
        .expect("initial workspace init");
    let registry_path = global.join("workspaces.json");
    let registry_bytes = std::fs::read_to_string(&registry_path).expect("read protected registry");

    std::env::set_current_dir(second.path()).expect("switch to second workspace");
    let error = args(false)
        .execute_without_runtime(None)
        .expect_err("existing durable ID must require force")
        .to_string();
    assert!(error.contains("already exists"), "unexpected: {error}");
    assert_eq!(
        std::fs::read_to_string(&registry_path).expect("read registry"),
        registry_bytes
    );
    assert!(!second.path().join(".orbit").exists());
}

#[test]
fn forced_workspace_reconciliation_preserves_registry_and_identity_on_validation_failure() {
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    let global = home.path().join(".orbit");
    std::fs::create_dir_all(&global).expect("create global orbit");
    std::fs::write(
        global.join("host.toml"),
        "schema_version = 1\nmachine_id = \"hm_force_failure\"\nhost_id = \"force-failure\"\nmode = \"standalone\"\n",
    )
    .expect("write host identity");
    let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());
    let args = |force| WorkspaceInitArgs {
        name: Some("force-failure".to_string()),
        base_branch: Some("agent-main".to_string()),
        ship_mode: Some("pr".to_string()),
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force,
    };
    args(false)
        .execute_without_runtime(None)
        .expect("initial workspace init");
    let registry_path = global.join("workspaces.json");
    let identity_path = workspace.path().join(".orbit/config.yaml");
    std::fs::write(
        &identity_path,
        "schema_version: 1\nworkspace_id: ws_other\n",
    )
    .expect("corrupt identity for validation test");
    let registry_bytes = std::fs::read_to_string(&registry_path).expect("read protected registry");
    let identity_bytes = std::fs::read_to_string(&identity_path).expect("read protected identity");

    let error = args(true)
        .execute_without_runtime(None)
        .expect_err("mismatched identity must reject forced reconciliation")
        .to_string();
    assert!(error.contains("checkout identity"), "unexpected: {error}");
    assert_eq!(
        std::fs::read_to_string(&registry_path).expect("read registry"),
        registry_bytes
    );
    assert_eq!(
        std::fs::read_to_string(&identity_path).expect("read identity"),
        identity_bytes
    );
}

#[test]
fn multi_host_workspace_init_persists_an_explicit_local_owner() {
    for mode in ["hub", "spoke"] {
        let workspace = tempdir().expect("workspace tempdir");
        let home = tempdir().expect("home tempdir");
        let global = home.path().join(".orbit");
        std::fs::create_dir_all(&global).expect("create global orbit");
        std::fs::write(
            global.join("host.toml"),
            format!(
                "schema_version = 1\nmachine_id = \"hm_local\"\nhost_id = \"local\"\nmode = \"{mode}\"\n"
            ),
        )
        .expect("write host identity");

        let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());
        WorkspaceInitArgs {
            name: Some(format!("{mode}-owner")),
            base_branch: Some("agent-main".to_string()),
            ship_mode: Some("pr".to_string()),
            role: None,
            owner: None,
            task_id_start: None,
            mcp: false,
            inject_agent_rules: false,
            refresh_defaults: false,
            force: false,
        }
        .execute_without_runtime(None)
        .expect("explicit multi-host workspace init");

        let registry = workspace_registry::load_registry_from(&global.join("workspaces.json"))
            .expect("reload multi-host registry");
        assert_eq!(
            registry.workspaces[0].owner_machine_id.as_deref(),
            Some("hm_local")
        );
        assert_eq!(
            registry.checkouts[0].role,
            Some(orbit_common::types::WorkspaceCheckoutRole::Owner)
        );
    }
}

#[test]
fn spoke_workspace_init_can_atomically_declare_a_remote_owner_replica() {
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    let global = home.path().join(".orbit");
    std::fs::create_dir_all(&global).expect("create global orbit");
    std::fs::write(
        global.join("host.toml"),
        "schema_version = 1\nmachine_id = \"hm_spoke\"\nhost_id = \"spoke\"\nmode = \"spoke\"\n",
    )
    .expect("write host identity");

    let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());
    WorkspaceInitArgs {
        name: Some("replica".to_string()),
        base_branch: Some("agent-main".to_string()),
        ship_mode: Some("pr".to_string()),
        role: Some(CliCheckoutRole::Replica),
        owner: Some("hm_owner".to_string()),
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force: false,
    }
    .execute_without_runtime(None)
    .expect("atomic replica workspace init");

    let registry = workspace_registry::load_registry_from(&global.join("workspaces.json"))
        .expect("reload replica registry");
    assert_eq!(
        registry.workspaces[0].owner_machine_id.as_deref(),
        Some("hm_owner")
    );
    assert_eq!(
        registry.checkouts[0].role,
        Some(CliCheckoutRole::Replica.into())
    );
    assert_eq!(
        registry.checkouts[0].owner_machine_id.as_deref(),
        Some("hm_owner")
    );
}

#[test]
fn invalid_replica_init_fails_before_workspace_artifacts_or_registry_mutation() {
    for (mode, rejected_owner, expected) in [
        ("spoke", "hm_spoke", "local machine"),
        ("spoke", "ssh:hub", "machine_id"),
        ("standalone", "hm_owner", "standalone mode"),
    ] {
        let workspace = tempdir().expect("workspace tempdir");
        let home = tempdir().expect("home tempdir");
        let global = home.path().join(".orbit");
        std::fs::create_dir_all(&global).expect("create global orbit");
        std::fs::write(
            global.join("host.toml"),
            format!(
                "schema_version = 1\nmachine_id = \"hm_spoke\"\nhost_id = \"spoke\"\nmode = \"{mode}\"\n"
            ),
        )
        .expect("write host identity");

        let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());
        let error = WorkspaceInitArgs {
            name: Some("invalid-replica".to_string()),
            base_branch: Some("agent-main".to_string()),
            ship_mode: Some("pr".to_string()),
            role: Some(CliCheckoutRole::Replica),
            owner: Some(rejected_owner.to_string()),
            task_id_start: None,
            mcp: false,
            inject_agent_rules: false,
            refresh_defaults: false,
            force: false,
        }
        .execute_without_runtime(None)
        .expect_err("invalid replica declaration must fail before bootstrap")
        .to_string();
        assert!(error.contains(expected), "unexpected: {error}");
        assert!(!workspace.path().join(".orbit").exists());
        assert!(!workspace.path().join(".gitignore").exists());
        assert!(!global.join("workspaces.json").exists());
    }
}

#[test]
fn workspace_list_and_show_report_effective_ship_mode() {
    let now = Utc::now();
    let workspace = Workspace {
        id: "ws_constellation".to_string(),
        name: "pr-gated".to_string(),
        owner_machine_id: None,
        git_remote: None,
        ship_mode: Some("pr".to_string()),
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: now,
        updated_at: now,
    };
    let registry = WorkspaceRegistry {
        workspaces: vec![workspace.clone()],
        checkouts: vec![WorkspaceCheckout::owner(
            workspace.id.clone(),
            "/work/pr-gated".into(),
            "/work/pr-gated/.orbit".into(),
        )],
        ..Default::default()
    };

    let list = format_workspace_list(&registry);
    assert!(list.contains("SHIP MODE"), "{list}");
    assert!(list.contains("pr"), "{list}");
    let mut lines = list.lines();
    let header = lines.next().expect("workspace list header");
    let row = lines.next().expect("workspace list row");
    let status_column = header.find("STATUS").expect("status column");
    let ship_mode_column = header.find("SHIP MODE").expect("ship mode column");
    assert!(row[status_column..].starts_with("active"), "{list}");
    assert!(row[ship_mode_column..].starts_with("pr"), "{list}");

    let show = format_workspace_show(&workspace, &registry.checkouts[0]);
    assert!(show.contains("ship_mode:   pr"), "{show}");
}

#[test]
fn workspace_init_seeds_disabled_routines_and_reinit_preserves_authored_files() {
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    let global = home.path().join(".orbit");
    std::fs::create_dir_all(&global).expect("create global orbit");
    std::fs::write(
        global.join("host.toml"),
        "schema_version = 1\nmachine_id = \"hm_inithost\"\nhost_id = \"init-host\"\nmode = \"standalone\"\n",
    )
    .expect("write host identity");

    let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());

    let init = |force| WorkspaceInitArgs {
        name: Some("routine-seed-test".to_string()),
        base_branch: Some("agent-main".to_string()),
        ship_mode: Some("pr".to_string()),
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force,
    };
    init(false)
        .execute_without_runtime(None)
        .expect("first workspace init");

    let routines_dir = workspace.path().join(".orbit/routines");
    let workspace_slug = workspace
        .path()
        .file_name()
        .expect("workspace directory name")
        .to_string_lossy()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    for (stem, target) in [
        ("auto_task_scheduler", "auto_task_scheduler_pipeline"),
        ("task_triage", "task_triage_pipeline"),
        ("ship_sweep", "workspace_ship_pipeline"),
    ] {
        let yaml = std::fs::read_to_string(routines_dir.join(format!("{stem}.yaml")))
            .expect("read seeded routine");
        let definition = parse_routine_yaml(&yaml).expect("parse seeded routine");
        assert_eq!(
            definition.name,
            format!("{}-{workspace_slug}", stem.replace('_', "-"))
        );
        assert_eq!(definition.hosts, ["init-host"]);
        assert_eq!(definition.target, RoutineTarget::Job(target.to_string()));
        assert_eq!(definition.policy.overlap, OverlapPolicy::Forbid);
        assert!(!definition.enabled);
    }

    let authored_ship = r#"schemaVersion: 1
name: custom-ship-sweep
description: authored values must survive re-init
enabled: true
hosts: [custom-host]
trigger:
  cron: "7 3 * * *"
  missed_run: catch_up_once
target: job:workspace_ship_pipeline
policy:
  timeout_minutes: 77
  overlap: allow
"#;
    std::fs::write(routines_dir.join("ship_sweep.yaml"), authored_ship)
        .expect("author ship routine");
    std::fs::remove_file(routines_dir.join("task_triage.yaml"))
        .expect("remove one default routine");

    init(true)
        .execute_without_runtime(None)
        .expect("second workspace init");

    assert_eq!(
        std::fs::read_to_string(routines_dir.join("ship_sweep.yaml"))
            .expect("read authored ship routine"),
        authored_ship,
        "plain re-init must preserve workspace-authored routine bytes"
    );
    let recreated = std::fs::read_to_string(routines_dir.join("task_triage.yaml"))
        .expect("missing default recreated");
    assert!(
        !parse_routine_yaml(&recreated)
            .expect("parse recreated routine")
            .enabled
    );
}

#[test]
fn workspace_init_seeds_auto_detected_mcp_configs() {
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");

    std::fs::create_dir_all(workspace.path().join(".claude")).expect("create .claude");
    std::fs::create_dir_all(workspace.path().join(".gemini")).expect("create .gemini");
    std::fs::create_dir_all(workspace.path().join(".grok")).expect("create .grok");
    std::fs::create_dir_all(home.path().join(".codex")).expect("create global .codex");
    std::fs::write(
        home.path().join(".codex").join("config.toml"),
        "model = \"gpt-5.4\"\n",
    )
    .expect("write global codex config");

    let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: Some("main".to_string()),
        ship_mode: None,
        role: None,
        owner: None,
        task_id_start: None,
        mcp: true,
        inject_agent_rules: false,
        refresh_defaults: false,
        force: false,
    }
    .execute_without_runtime(None);

    result.expect("workspace init");
    assert!(
        workspace
            .path()
            .join(".claude")
            .join("settings.json")
            .exists()
    );
    assert!(workspace.path().join(".codex").join("config.toml").exists());
    assert!(
        workspace
            .path()
            .join(".gemini")
            .join("settings.json")
            .exists()
    );
    assert!(workspace.path().join(".grok").join("config.toml").exists());
}

#[test]
fn workspace_init_skips_mcp_by_default() {
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");

    std::fs::create_dir_all(workspace.path().join(".claude")).expect("create .claude");
    std::fs::create_dir_all(workspace.path().join(".gemini")).expect("create .gemini");
    std::fs::create_dir_all(workspace.path().join(".grok")).expect("create .grok");
    std::fs::create_dir_all(home.path().join(".codex")).expect("create global .codex");
    std::fs::write(
        home.path().join(".codex").join("config.toml"),
        "model = \"gpt-5.4\"\n",
    )
    .expect("write global codex config");

    let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: Some("main".to_string()),
        ship_mode: None,
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force: false,
    }
    .execute_without_runtime(None);

    result.expect("workspace init");
    assert!(
        !workspace
            .path()
            .join(".claude")
            .join("settings.json")
            .exists()
    );
    assert!(!workspace.path().join(".codex").join("config.toml").exists());
    assert!(
        !workspace
            .path()
            .join(".gemini")
            .join("settings.json")
            .exists()
    );
    assert!(!workspace.path().join(".grok").join("config.toml").exists());
}

#[test]
fn workspace_init_under_home_with_global_orbit_creates_repo_orbit() {
    let home = tempdir().expect("home tempdir");
    let workspace = home.path().join("work").join("repo");
    std::fs::create_dir_all(workspace.join(".git")).expect("create workspace repo");
    std::fs::create_dir_all(home.path().join(".orbit")).expect("create global orbit root");

    let _env = EnvGuard::acquire().home(home.path()).cwd(&workspace);

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: Some("main".to_string()),
        ship_mode: None,
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force: false,
    }
    .execute_without_runtime(None);

    result.expect("workspace init");
    assert!(workspace.join(".orbit").join("state").is_dir());
    assert!(workspace.join(".orbit").join("knowledge").is_dir());
    assert!(!home.path().join(".orbit").join("state").exists());
    assert!(!home.path().join(".orbit").join("knowledge").exists());
    assert_eq!(
        std::fs::read_to_string(workspace.join(".gitignore")).expect("read .gitignore"),
        orbit_gitignore_block()
    );
}

#[test]
fn workspace_init_appends_orbit_to_existing_gitignore() {
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(workspace.path().join(".git")).expect("create .git");
    std::fs::write(workspace.path().join(".gitignore"), "target/\n.DS_Store")
        .expect("write .gitignore");

    let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: Some("main".to_string()),
        ship_mode: None,
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force: false,
    }
    .execute_without_runtime(None);

    result.expect("workspace init");
    assert_eq!(
        std::fs::read_to_string(workspace.path().join(".gitignore")).expect("read .gitignore"),
        format!("target/\n.DS_Store\n{}", orbit_gitignore_block())
    );
}

#[test]
fn workspace_init_replaces_legacy_bare_orbit_gitignore_line_with_managed_block() {
    // A bare `.orbit` line (written by earlier init versions) ignores the whole
    // directory, so the ADR / learning re-includes can never apply. Init must
    // replace the legacy line with the managed block, not merely append.
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(workspace.path().join(".git")).expect("create .git");
    std::fs::write(workspace.path().join(".gitignore"), "target/\n/.orbit/\n")
        .expect("write .gitignore");

    let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());

    let init = |force| WorkspaceInitArgs {
        name: None,
        base_branch: Some("main".to_string()),
        ship_mode: None,
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force,
    };

    init(false)
        .execute_without_runtime(None)
        .expect("workspace init");
    let expected = format!("target/\n{}", orbit_gitignore_block());
    assert_eq!(
        std::fs::read_to_string(workspace.path().join(".gitignore")).expect("read .gitignore"),
        expected,
        "legacy bare `.orbit` must be replaced by the managed block"
    );

    // Re-init is idempotent: the block is not duplicated or reordered.
    init(true)
        .execute_without_runtime(None)
        .expect("workspace re-init");
    assert_eq!(
        std::fs::read_to_string(workspace.path().join(".gitignore")).expect("read .gitignore"),
        expected,
        "re-init must be idempotent once the managed block is present"
    );
}

#[test]
fn workspace_init_from_git_subdir_gitignores_repo_orbit_dir() {
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let nested = repo.path().join("packages").join("demo");
    std::fs::create_dir_all(repo.path().join(".git")).expect("create .git");
    std::fs::create_dir_all(&nested).expect("create nested workspace");

    let _env = EnvGuard::acquire().home(home.path()).cwd(&nested);

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: Some("main".to_string()),
        ship_mode: None,
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force: false,
    }
    .execute_without_runtime(None);

    result.expect("workspace init");
    assert_eq!(
        std::fs::read_to_string(repo.path().join(".gitignore")).expect("read repo .gitignore"),
        orbit_gitignore_block()
    );
    assert!(!nested.join(".gitignore").exists());
}

#[test]
fn workspace_init_with_root_override_uses_custom_registry() {
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    let custom_root_parent = tempdir().expect("custom root parent");
    let custom_root = custom_root_parent.path().join("custom-orbit");

    let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());

    let result = WorkspaceInitArgs {
        name: Some("custom-root".to_string()),
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
    .execute_without_runtime(Some(custom_root.as_path()));

    result.expect("workspace init with root override");

    let custom_registry_path = custom_root.join("workspaces.json");
    assert!(custom_registry_path.exists());
    assert!(!home.path().join(".orbit").join("workspaces.json").exists());

    let registry = workspace_registry::load_registry_from(&custom_registry_path)
        .expect("load custom registry");
    let workspace_record = registry
        .workspaces
        .iter()
        .find(|workspace| workspace.name == "custom-root")
        .expect("registered workspace");
    let checkout = workspace_registry::find_checkout(&registry, &workspace_record.id)
        .expect("registered checkout");
    assert_eq!(
        std::fs::canonicalize(&checkout.repo_root).expect("canonical registered root"),
        std::fs::canonicalize(workspace.path()).expect("canonical workspace")
    );
    assert_eq!(
        std::fs::canonicalize(&checkout.orbit_dir).expect("canonical registered root"),
        std::fs::canonicalize(&custom_root).expect("canonical custom root")
    );
    assert_eq!(workspace_record.base_branch, "main");
    assert!(
        !workspace.path().join(".orbitignore").exists(),
        "workspace init must not create the retired graph ignore file"
    );
}

#[test]
fn workspace_init_does_not_create_orbitignore() {
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");

    let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: Some("main".to_string()),
        ship_mode: None,
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force: false,
    }
    .execute_without_runtime(None);

    result.expect("workspace init");
    assert!(
        !workspace.path().join(".orbitignore").exists(),
        "workspace init must not create the retired graph ignore file"
    );
}

#[test]
fn workspace_init_preserves_existing_orbitignore() {
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::write(
        workspace.path().join(".orbitignore"),
        "custom-output/\n!custom-output/keep.txt\n",
    )
    .expect("seed existing .orbitignore");

    let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: Some("main".to_string()),
        ship_mode: None,
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force: false,
    }
    .execute_without_runtime(None);

    result.expect("workspace init");
    assert_eq!(
        std::fs::read_to_string(workspace.path().join(".orbitignore")).expect("read .orbitignore"),
        "custom-output/\n!custom-output/keep.txt\n"
    );
}

#[test]
fn workspace_init_with_root_override_does_not_modify_repo_gitignore() {
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    let custom_root_parent = tempdir().expect("custom root parent");
    let custom_root = custom_root_parent.path().join("custom-orbit");

    // Seed the workspace as a git repo so the pre-fix code would have
    // appended `.orbit` to <workspace>/.gitignore.
    std::fs::create_dir_all(workspace.path().join(".git")).expect("seed git dir");

    let _env = EnvGuard::acquire().home(home.path()).cwd(workspace.path());

    let result = WorkspaceInitArgs {
        name: Some("custom-root-git".to_string()),
        base_branch: Some("main".to_string()),
        ship_mode: None,
        role: None,
        owner: None,
        task_id_start: None,
        mcp: false,
        inject_agent_rules: false,
        refresh_defaults: false,
        force: false,
    }
    .execute_without_runtime(Some(custom_root.as_path()));

    result.expect("workspace init with root override in a git repo");

    let gitignore = workspace.path().join(".gitignore");
    assert!(
        !gitignore.exists(),
        "`--root` outside the workspace must not create <workspace>/.gitignore",
    );
}

/// Regression (ORB-10293): a nameless workspace whose default name is derived
/// from a `.tmpXXXXXX` cwd must register only in the isolated fixture registry
/// and never touch a synthetic "outer" HOME registry standing in for the
/// operator's real `~/.orbit/workspaces.json`. This reproduces the exact shape
/// (`ws_.tmpXXXXXX`) that leaked into the operator's registry before the shared
/// env guard serialized these tests.
#[test]
fn nameless_tmp_workspace_registers_only_in_isolated_registry() {
    // Synthetic operator HOME with a sentinel registry that must never change.
    let outer_home = tempdir().expect("outer home tempdir");
    let outer_registry = outer_home.path().join(".orbit").join("workspaces.json");
    std::fs::create_dir_all(outer_registry.parent().expect("outer .orbit parent"))
        .expect("create outer .orbit");
    let sentinel = "{\"sentinel\":\"operator-registry\"}\n";
    std::fs::write(&outer_registry, sentinel).expect("seed sentinel registry");

    // Isolated fixture HOME plus a nameless workspace directory. `tempdir()`
    // yields `/tmp/.tmpXXXXXX`, so the default workspace name is `.tmpXXXXXX`.
    let fixture_home = tempdir().expect("fixture home tempdir");
    let workspace = tempdir().expect("nameless workspace tempdir");
    let workspace_name = workspace
        .path()
        .file_name()
        .expect("workspace dir name")
        .to_string_lossy()
        .into_owned();
    assert!(
        workspace_name.starts_with(".tmp"),
        "fixture must reproduce the nameless `.tmpXXXXXX` shape, got {workspace_name}"
    );

    {
        let _env = EnvGuard::acquire()
            .home(fixture_home.path())
            .cwd(workspace.path());
        WorkspaceInitArgs {
            name: None,
            base_branch: Some("main".to_string()),
            ship_mode: None,
            role: None,
            owner: None,
            task_id_start: None,
            mcp: false,
            inject_agent_rules: false,
            refresh_defaults: false,
            force: false,
        }
        .execute_without_runtime(None)
        .expect("nameless workspace init");
    }

    // The nameless workspace registered only in the isolated fixture registry.
    let fixture_registry = fixture_home.path().join(".orbit").join("workspaces.json");
    let registry =
        workspace_registry::load_registry_from(&fixture_registry).expect("load fixture registry");
    assert!(
        registry
            .workspaces
            .iter()
            .any(|w| w.id == canonical_workspace_id(&workspace_name)),
        "nameless workspace must register in the isolated fixture registry"
    );

    // The synthetic outer registry is byte-for-byte unchanged: workspace init
    // never touched the operator's real machine-global registry.
    assert_eq!(
        std::fs::read_to_string(&outer_registry).expect("read outer registry"),
        sentinel,
        "workspace init must never mutate the operator's real registry"
    );
}
