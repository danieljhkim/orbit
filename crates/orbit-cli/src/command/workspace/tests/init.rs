use std::sync::Mutex;

use tempfile::tempdir;

use orbit_core::command::init::default_orbitignore_template;
use orbit_core::workspace_registry;

use super::super::init::WorkspaceInitArgs;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn workspace_init_seeds_auto_detected_mcp_configs() {
    let _guard = ENV_LOCK.lock().expect("lock env");
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

    let previous_home = std::env::var_os("HOME");
    let previous_cwd = std::env::current_dir().expect("capture cwd");
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    std::env::set_current_dir(workspace.path()).expect("enter workspace");

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: "main".to_string(),
        mcp: true,
        hooks: false,
        inject_agent_rules: false,
        refresh_defaults: false,
    }
    .execute_without_runtime(None);

    std::env::set_current_dir(previous_cwd).expect("restore cwd");

    match previous_home {
        Some(value) => unsafe {
            std::env::set_var("HOME", value);
        },
        None => unsafe {
            std::env::remove_var("HOME");
        },
    }

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
    let _guard = ENV_LOCK.lock().expect("lock env");
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

    let previous_home = std::env::var_os("HOME");
    let previous_cwd = std::env::current_dir().expect("capture cwd");
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    std::env::set_current_dir(workspace.path()).expect("enter workspace");

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: "main".to_string(),
        mcp: false,
        hooks: false,
        inject_agent_rules: false,
        refresh_defaults: false,
    }
    .execute_without_runtime(None);

    std::env::set_current_dir(previous_cwd).expect("restore cwd");

    match previous_home {
        Some(value) => unsafe {
            std::env::set_var("HOME", value);
        },
        None => unsafe {
            std::env::remove_var("HOME");
        },
    }

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
    let _guard = ENV_LOCK.lock().expect("lock env");
    let home = tempdir().expect("home tempdir");
    let workspace = home.path().join("work").join("repo");
    std::fs::create_dir_all(workspace.join(".git")).expect("create workspace repo");
    std::fs::create_dir_all(home.path().join(".orbit")).expect("create global orbit root");

    let previous_home = std::env::var_os("HOME");
    let previous_cwd = std::env::current_dir().expect("capture cwd");
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    std::env::set_current_dir(&workspace).expect("enter workspace");

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: "main".to_string(),
        mcp: false,
        hooks: false,
        inject_agent_rules: false,
        refresh_defaults: false,
    }
    .execute_without_runtime(None);

    std::env::set_current_dir(previous_cwd).expect("restore cwd");

    match previous_home {
        Some(value) => unsafe {
            std::env::set_var("HOME", value);
        },
        None => unsafe {
            std::env::remove_var("HOME");
        },
    }

    result.expect("workspace init");
    assert!(workspace.join(".orbit").join("state").is_dir());
    assert!(workspace.join(".orbit").join("knowledge").is_dir());
    assert!(!home.path().join(".orbit").join("state").exists());
    assert!(!home.path().join(".orbit").join("knowledge").exists());
    assert_eq!(
        std::fs::read_to_string(workspace.join(".gitignore")).expect("read .gitignore"),
        ".orbit\n"
    );
}

#[test]
fn workspace_init_appends_orbit_to_existing_gitignore() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(workspace.path().join(".git")).expect("create .git");
    std::fs::write(workspace.path().join(".gitignore"), "target/\n.DS_Store")
        .expect("write .gitignore");

    let previous_home = std::env::var_os("HOME");
    let previous_cwd = std::env::current_dir().expect("capture cwd");
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    std::env::set_current_dir(workspace.path()).expect("enter workspace");

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: "main".to_string(),
        mcp: false,
        hooks: false,
        inject_agent_rules: false,
        refresh_defaults: false,
    }
    .execute_without_runtime(None);

    std::env::set_current_dir(previous_cwd).expect("restore cwd");

    match previous_home {
        Some(value) => unsafe {
            std::env::set_var("HOME", value);
        },
        None => unsafe {
            std::env::remove_var("HOME");
        },
    }

    result.expect("workspace init");
    assert_eq!(
        std::fs::read_to_string(workspace.path().join(".gitignore")).expect("read .gitignore"),
        "target/\n.DS_Store\n.orbit\n"
    );
}

#[test]
fn workspace_init_does_not_duplicate_existing_orbit_gitignore_entry() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::create_dir_all(workspace.path().join(".git")).expect("create .git");
    std::fs::write(workspace.path().join(".gitignore"), "target/\n/.orbit/\n")
        .expect("write .gitignore");

    let previous_home = std::env::var_os("HOME");
    let previous_cwd = std::env::current_dir().expect("capture cwd");
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    std::env::set_current_dir(workspace.path()).expect("enter workspace");

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: "main".to_string(),
        mcp: false,
        hooks: false,
        inject_agent_rules: false,
        refresh_defaults: false,
    }
    .execute_without_runtime(None);

    std::env::set_current_dir(previous_cwd).expect("restore cwd");

    match previous_home {
        Some(value) => unsafe {
            std::env::set_var("HOME", value);
        },
        None => unsafe {
            std::env::remove_var("HOME");
        },
    }

    result.expect("workspace init");
    assert_eq!(
        std::fs::read_to_string(workspace.path().join(".gitignore")).expect("read .gitignore"),
        "target/\n/.orbit/\n"
    );
}

#[test]
fn workspace_init_from_git_subdir_gitignores_repo_orbit_dir() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let repo = tempdir().expect("repo tempdir");
    let home = tempdir().expect("home tempdir");
    let nested = repo.path().join("packages").join("demo");
    std::fs::create_dir_all(repo.path().join(".git")).expect("create .git");
    std::fs::create_dir_all(&nested).expect("create nested workspace");

    let previous_home = std::env::var_os("HOME");
    let previous_cwd = std::env::current_dir().expect("capture cwd");
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    std::env::set_current_dir(&nested).expect("enter nested workspace");

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: "main".to_string(),
        mcp: false,
        hooks: false,
        inject_agent_rules: false,
        refresh_defaults: false,
    }
    .execute_without_runtime(None);

    std::env::set_current_dir(previous_cwd).expect("restore cwd");

    match previous_home {
        Some(value) => unsafe {
            std::env::set_var("HOME", value);
        },
        None => unsafe {
            std::env::remove_var("HOME");
        },
    }

    result.expect("workspace init");
    assert_eq!(
        std::fs::read_to_string(repo.path().join(".gitignore")).expect("read repo .gitignore"),
        ".orbit\n"
    );
    assert!(!nested.join(".gitignore").exists());
}

#[test]
fn workspace_init_with_root_override_uses_custom_registry() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    let custom_root_parent = tempdir().expect("custom root parent");
    let custom_root = custom_root_parent.path().join("custom-orbit");

    let previous_home = std::env::var_os("HOME");
    let previous_cwd = std::env::current_dir().expect("capture cwd");
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    std::env::set_current_dir(workspace.path()).expect("enter workspace");

    let result = WorkspaceInitArgs {
        name: Some("custom-root".to_string()),
        base_branch: "main".to_string(),
        mcp: false,
        hooks: false,
        inject_agent_rules: false,
        refresh_defaults: false,
    }
    .execute_without_runtime(Some(custom_root.as_path()));

    std::env::set_current_dir(previous_cwd).expect("restore cwd");

    match previous_home {
        Some(value) => unsafe {
            std::env::set_var("HOME", value);
        },
        None => unsafe {
            std::env::remove_var("HOME");
        },
    }

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
    assert_eq!(
        std::fs::canonicalize(&workspace_record.root).expect("canonical registered root"),
        std::fs::canonicalize(workspace.path()).expect("canonical workspace")
    );
    assert_eq!(
        std::fs::canonicalize(&workspace_record.orbit_dir).expect("canonical registered root"),
        std::fs::canonicalize(&custom_root).expect("canonical custom root")
    );
    assert_eq!(
        std::fs::read_to_string(workspace.path().join(".orbitignore"))
            .expect("read workspace .orbitignore"),
        default_orbitignore_template()
    );
    assert!(
        !custom_root_parent.path().join(".orbitignore").exists(),
        ".orbitignore belongs in the workspace root, not beside a custom Orbit root"
    );
}

#[test]
fn workspace_init_seeds_default_orbitignore_when_missing() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");

    let previous_home = std::env::var_os("HOME");
    let previous_cwd = std::env::current_dir().expect("capture cwd");
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    std::env::set_current_dir(workspace.path()).expect("enter workspace");

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: "main".to_string(),
        mcp: false,
        hooks: false,
        inject_agent_rules: false,
        refresh_defaults: false,
    }
    .execute_without_runtime(None);

    std::env::set_current_dir(previous_cwd).expect("restore cwd");

    match previous_home {
        Some(value) => unsafe {
            std::env::set_var("HOME", value);
        },
        None => unsafe {
            std::env::remove_var("HOME");
        },
    }

    result.expect("workspace init");
    assert_eq!(
        std::fs::read_to_string(workspace.path().join(".orbitignore")).expect("read .orbitignore"),
        default_orbitignore_template()
    );
}

#[test]
fn workspace_init_preserves_existing_orbitignore() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    std::fs::write(
        workspace.path().join(".orbitignore"),
        "custom-output/\n!custom-output/keep.txt\n",
    )
    .expect("seed existing .orbitignore");

    let previous_home = std::env::var_os("HOME");
    let previous_cwd = std::env::current_dir().expect("capture cwd");
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    std::env::set_current_dir(workspace.path()).expect("enter workspace");

    let result = WorkspaceInitArgs {
        name: None,
        base_branch: "main".to_string(),
        mcp: false,
        hooks: false,
        inject_agent_rules: false,
        refresh_defaults: false,
    }
    .execute_without_runtime(None);

    std::env::set_current_dir(previous_cwd).expect("restore cwd");

    match previous_home {
        Some(value) => unsafe {
            std::env::set_var("HOME", value);
        },
        None => unsafe {
            std::env::remove_var("HOME");
        },
    }

    result.expect("workspace init");
    assert_eq!(
        std::fs::read_to_string(workspace.path().join(".orbitignore")).expect("read .orbitignore"),
        "custom-output/\n!custom-output/keep.txt\n"
    );
}

#[test]
fn workspace_init_with_root_override_does_not_modify_repo_gitignore() {
    let _guard = ENV_LOCK.lock().expect("lock env");
    let workspace = tempdir().expect("workspace tempdir");
    let home = tempdir().expect("home tempdir");
    let custom_root_parent = tempdir().expect("custom root parent");
    let custom_root = custom_root_parent.path().join("custom-orbit");

    // Seed the workspace as a git repo so the pre-fix code would have
    // appended `.orbit` to <workspace>/.gitignore.
    std::fs::create_dir_all(workspace.path().join(".git")).expect("seed git dir");

    let previous_home = std::env::var_os("HOME");
    let previous_cwd = std::env::current_dir().expect("capture cwd");
    unsafe {
        std::env::set_var("HOME", home.path());
    }
    std::env::set_current_dir(workspace.path()).expect("enter workspace");

    let result = WorkspaceInitArgs {
        name: Some("custom-root-git".to_string()),
        base_branch: "main".to_string(),
        mcp: false,
        hooks: false,
        inject_agent_rules: false,
        refresh_defaults: false,
    }
    .execute_without_runtime(Some(custom_root.as_path()));

    std::env::set_current_dir(previous_cwd).expect("restore cwd");

    match previous_home {
        Some(value) => unsafe {
            std::env::set_var("HOME", value);
        },
        None => unsafe {
            std::env::remove_var("HOME");
        },
    }

    result.expect("workspace init with root override in a git repo");

    let gitignore = workspace.path().join(".gitignore");
    assert!(
        !gitignore.exists(),
        "`--root` outside the workspace must not create <workspace>/.gitignore",
    );
}
