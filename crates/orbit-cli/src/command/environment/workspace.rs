use chrono::Utc;
use clap::{Args, Subcommand};
use orbit_common::types::{Workspace, WorkspaceStatus};
use orbit_core::command::agent_rules::{InjectionAction, inject_agent_rules};
use orbit_core::command::design::seed_design_conventions;
use orbit_core::command::init::{
    InitOptions, build_initial_graph, init_workspace_at_root, seed_default_orbitignore,
};
use orbit_core::workspace_registry;
use orbit_core::{OrbitError, OrbitRuntime};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Initialize and manage workspaces")]
pub struct WorkspaceCommand {
    #[command(subcommand)]
    pub command: WorkspaceSubcommand,
}

#[derive(Subcommand)]
pub enum WorkspaceSubcommand {
    /// Initialize a new workspace in the current directory
    Init(WorkspaceInitArgs),
    /// List all registered workspaces
    List(WorkspaceListArgs),
    /// Show the current workspace
    Show(WorkspaceShowArgs),
    /// Remove a workspace from the registry (does not delete .orbit)
    Remove(WorkspaceRemoveArgs),
    /// Remove all Orbit artifacts from this workspace
    Teardown(WorkspaceTeardownArgs),
}

#[derive(Args)]
pub struct WorkspaceInitArgs {
    /// Workspace name (defaults to directory name)
    #[arg(long)]
    pub name: Option<String>,
    /// Base branch for this workspace (default: main)
    #[arg(long, default_value = "main")]
    pub base_branch: String,
    /// Set up MCP client integrations for auto-detected providers.
    #[arg(long)]
    pub mcp: bool,
    /// Seed docs/design/CONVENTIONS.md when it does not already exist.
    #[arg(long)]
    pub design: bool,
    /// Inject (or refresh) an Orbit workflow-rules block in CLAUDE.md and AGENTS.md at the workspace root.
    #[arg(long)]
    pub inject_agent_rules: bool,
    /// No-op (kept for backwards compatibility — defaults are always refreshed on init)
    #[arg(long, hide = true)]
    pub refresh_defaults: bool,
}

#[derive(Args)]
pub struct WorkspaceListArgs {}

#[derive(Args)]
pub struct WorkspaceShowArgs {}

#[derive(Args)]
pub struct WorkspaceRemoveArgs {
    /// Workspace name or id
    pub workspace: String,
}

#[derive(Args)]
pub struct WorkspaceTeardownArgs {
    /// Required flag to confirm destructive operation
    #[arg(long)]
    pub confirm: bool,
}

impl Execute for WorkspaceCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self.command {
            WorkspaceSubcommand::Init(_) => {
                // Init is handled without runtime in main.rs
                unreachable!("workspace init should be handled before runtime initialization")
            }
            WorkspaceSubcommand::List(args) => args.execute(runtime),
            WorkspaceSubcommand::Show(args) => args.execute(runtime),
            WorkspaceSubcommand::Remove(args) => args.execute(runtime),
            WorkspaceSubcommand::Teardown(args) => args.execute(runtime),
        }
    }
}

impl WorkspaceInitArgs {
    pub fn execute_without_runtime(
        self,
        root_override: Option<&std::path::Path>,
    ) -> Result<(), OrbitError> {
        let cwd = std::env::current_dir().map_err(|e| OrbitError::Io(e.to_string()))?;
        let (global_root, orbit_dir) =
            OrbitRuntime::resolve_bootstrap_roots_for_cwd(&cwd, root_override)?;
        let registry_path = workspace_registry::registry_path_for(&global_root);
        let mcp = self.mcp;
        let inject_rules = self.inject_agent_rules;
        let init_result = self.execute_at_path(&cwd, &orbit_dir, &global_root, &registry_path)?;

        println!("workspace '{}' initialized", init_result.name);
        println!("  id:        {}", init_result.id);
        println!("  root:      {}", init_result.root.display());
        println!("  orbit_dir: {}", init_result.orbit_dir.display());

        if mcp {
            let providers = crate::command::mcp::init_auto_for_workspace(
                &init_result.root,
                &init_result.orbit_dir,
            )?;
            if providers.is_empty() {
                println!("  mcp:       no providers auto-detected");
            } else {
                println!("  mcp:       {}", providers.join(", "));
            }
        } else {
            println!("  mcp:       skipped (pass --mcp to set up integrations)");
        }

        if inject_rules {
            let outcome = inject_agent_rules(&init_result.root)?;
            for entry in &outcome.outcomes {
                let label = entry
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| entry.path.display().to_string());
                let verb = match entry.action {
                    InjectionAction::Created => "created with Orbit rules block",
                    InjectionAction::AppendedBlock => "Orbit rules block appended",
                    InjectionAction::ReplacedBlock => "Orbit rules block refreshed",
                };
                println!("  rules:     {label}: {verb}");
            }
        }

        eprintln!("graph build: scanning {}", init_result.root.display());
        match build_initial_graph(&init_result.root, &init_result.orbit_dir) {
            Ok(summary) => {
                eprintln!(
                    "graph build: {} dirs, {} files, {} symbols",
                    summary.dirs, summary.files, summary.leaves,
                );
            }
            Err(e) => {
                eprintln!("graph build: failed ({e}), run `orbit graph build` manually");
            }
        }

        Ok(())
    }

    fn execute_at_path(
        self,
        cwd: &std::path::Path,
        orbit_dir: &std::path::Path,
        global_root: &std::path::Path,
        registry_path: &std::path::Path,
    ) -> Result<WorkspaceInitResult, OrbitError> {
        init_workspace_at_root(
            orbit_dir,
            InitOptions {
                refresh_defaults: true,
                global_root_override: Some(global_root.to_path_buf()),
                ..Default::default()
            },
        )?;
        if self.design {
            seed_design_conventions(cwd, "human")?;
        }
        seed_default_orbitignore(cwd)?;
        ensure_orbit_gitignore_entry(cwd, orbit_dir)?;

        let name = self.name.unwrap_or_else(|| dir_name_or_fallback(cwd));

        let id = format!("ws_{name}");
        let git_remote = detect_git_remote(cwd);

        let ws = Workspace {
            id: id.clone(),
            name: name.clone(),
            root: cwd.to_path_buf(),
            orbit_dir: orbit_dir.to_path_buf(),
            git_remote,
            base_branch: self.base_branch,
            status: WorkspaceStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let mut registry = workspace_registry::load_registry_from(registry_path)?;
        if let Some(existing) = registry.workspaces.iter_mut().find(|w| w.id == id) {
            existing.updated_at = Utc::now();
        } else {
            workspace_registry::register_workspace(&mut registry, ws)?;
        }
        workspace_registry::save_registry_to(&registry, registry_path)?;

        Ok(WorkspaceInitResult {
            id,
            name,
            root: cwd.to_path_buf(),
            orbit_dir: orbit_dir.to_path_buf(),
        })
    }
}

struct WorkspaceInitResult {
    id: String,
    name: String,
    root: std::path::PathBuf,
    orbit_dir: std::path::PathBuf,
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use std::sync::Mutex;

    use tempfile::tempdir;

    use orbit_core::command::graph::default_orbitignore_template;
    use orbit_core::workspace_registry;

    use super::WorkspaceInitArgs;

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
            design: false,
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
            design: false,
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
            design: false,
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
            design: false,
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
            design: false,
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
            design: false,
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
            design: false,
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
            design: false,
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
            std::fs::read_to_string(workspace.path().join(".orbitignore"))
                .expect("read .orbitignore"),
            default_orbitignore_template()
        );
    }

    #[test]
    fn workspace_init_design_flag_seeds_conventions() {
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
            design: true,
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
        let conventions = std::fs::read_to_string(
            workspace
                .path()
                .join("docs")
                .join("design")
                .join("CONVENTIONS.md"),
        )
        .expect("read conventions");
        assert!(conventions.contains("owner: human"));
        assert!(!conventions.contains("owner: daniel"));
    }

    #[test]
    fn workspace_init_without_design_flag_leaves_conventions_absent() {
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
            design: false,
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
                .join("docs")
                .join("design")
                .join("CONVENTIONS.md")
                .exists()
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
            design: false,
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
            std::fs::read_to_string(workspace.path().join(".orbitignore"))
                .expect("read .orbitignore"),
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
            design: false,
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
}

impl Execute for WorkspaceListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let global_root = runtime.global_root();
        let registry_path = workspace_registry::registry_path_for(&global_root);
        let mut registry = workspace_registry::load_registry_from(&registry_path)?;
        workspace_registry::validate_workspaces(&mut registry);

        if registry.workspaces.is_empty() {
            println!("no workspaces registered");
            return Ok(());
        }

        // Save back if staleness changed any status
        workspace_registry::save_registry_to(&registry, &registry_path)?;

        println!("{:<20} {:<12} {:<8} ROOT", "NAME", "ID", "STATUS");
        for ws in &registry.workspaces {
            println!(
                "{:<20} {:<12} {:<8} {}",
                ws.name,
                ws.id,
                ws.status,
                ws.root.display()
            );
        }
        Ok(())
    }
}

impl Execute for WorkspaceShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let data_root = runtime.data_root();
        let data_root_canonical = std::fs::canonicalize(&data_root).unwrap_or(data_root.clone());
        let global_root = runtime.global_root();
        let registry_path = workspace_registry::registry_path_for(&global_root);
        let registry = workspace_registry::load_registry_from(&registry_path)?;

        // Find workspace whose orbit_dir matches the current runtime's data root
        let ws = registry.workspaces.iter().find(|w| {
            let ws_canonical =
                std::fs::canonicalize(&w.orbit_dir).unwrap_or_else(|_| w.orbit_dir.clone());
            ws_canonical == data_root_canonical
        });

        match ws {
            Some(ws) => {
                println!("name:        {}", ws.name);
                println!("id:          {}", ws.id);
                println!("root:        {}", ws.root.display());
                println!("orbit_dir:   {}", ws.orbit_dir.display());
                println!("base_branch: {}", ws.base_branch);
                println!("status:      {}", ws.status);
                if let Some(ref remote) = ws.git_remote {
                    println!("git_remote:  {}", remote);
                }
                println!("created_at:  {}", ws.created_at);
                println!("updated_at:  {}", ws.updated_at);
            }
            None => {
                println!("current orbit root: {}", data_root.display());
                println!("(not registered as a workspace)");
            }
        }
        Ok(())
    }
}

impl Execute for WorkspaceRemoveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let global_root = runtime.global_root();
        let registry_path = workspace_registry::registry_path_for(&global_root);
        let mut registry = workspace_registry::load_registry_from(&registry_path)?;
        let removed = workspace_registry::remove_workspace(&mut registry, &self.workspace)?;
        workspace_registry::save_registry_to(&registry, &registry_path)?;
        println!("workspace '{}' removed from registry", removed.name);
        Ok(())
    }
}

impl Execute for WorkspaceTeardownArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        if !self.confirm {
            return Err(OrbitError::InvalidInput(
                "teardown is destructive. Pass --confirm to proceed.".to_string(),
            ));
        }

        let orbit_dir = runtime.data_root();
        let global_dir = runtime.global_root();

        // Safety: never delete the global ~/.orbit/ directory
        let orbit_canonical =
            std::fs::canonicalize(&orbit_dir).unwrap_or_else(|_| orbit_dir.clone());
        let global_canonical =
            std::fs::canonicalize(&global_dir).unwrap_or_else(|_| global_dir.clone());
        if orbit_canonical == global_canonical {
            return Err(OrbitError::InvalidInput(
                "refusing to teardown the global ~/.orbit/ directory".to_string(),
            ));
        }

        // Safety: orbit_dir must end with ".orbit"
        if orbit_dir.file_name().and_then(|n| n.to_str()) != Some(".orbit") {
            return Err(OrbitError::InvalidInput(format!(
                "data root '{}' does not end with .orbit — aborting teardown",
                orbit_dir.display()
            )));
        }

        let repo_root = orbit_dir
            .parent()
            .ok_or_else(|| OrbitError::InvalidInput("cannot determine repo root".to_string()))?;

        let mut removed: Vec<String> = Vec::new();

        // 1. Deregister from workspace registry (before deleting .orbit/)
        let global_root = runtime.global_root();
        let registry_path = workspace_registry::registry_path_for(&global_root);
        if registry_path.exists() {
            let mut registry = workspace_registry::load_registry_from(&registry_path)?;
            let ws = registry.workspaces.iter().find(|w| {
                let ws_canonical =
                    std::fs::canonicalize(&w.orbit_dir).unwrap_or_else(|_| w.orbit_dir.clone());
                ws_canonical == orbit_canonical
            });
            if let Some(ws_id) = ws.map(|w| w.id.clone()) {
                let ws = workspace_registry::remove_workspace(&mut registry, &ws_id)?;
                workspace_registry::save_registry_to(&registry, &registry_path)?;
                removed.push(format!(
                    "deregistered workspace '{}' from registry",
                    ws.name
                ));
            }
        }

        // 2. Remove legacy repo-local skill symlinks from .agents/skills/ and .claude/skills/
        for dir_name in &[".agents", ".claude"] {
            let skills_dir = repo_root.join(dir_name).join("skills");
            if skills_dir.is_dir() {
                remove_symlinks_in(&skills_dir)?;
                removed.push(format!("removed symlinks from {}/skills/", dir_name));

                // Remove skills dir if empty
                if is_dir_empty(&skills_dir) {
                    std::fs::remove_dir(&skills_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
                }
                // Remove parent dir if empty
                let parent = repo_root.join(dir_name);
                if parent.is_dir() && is_dir_empty(&parent) {
                    std::fs::remove_dir(&parent).map_err(|e| OrbitError::Io(e.to_string()))?;
                    removed.push(format!("removed empty {}/", dir_name));
                }
            }
        }

        // 3. Delete .orbit/ directory
        if orbit_dir.is_dir() {
            std::fs::remove_dir_all(&orbit_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
            removed.push(format!("deleted {}", orbit_dir.display()));
        }

        // 4. Print summary
        println!("teardown complete:");
        for item in &removed {
            println!("  - {item}");
        }
        if removed.is_empty() {
            println!("  (nothing to remove)");
        }

        Ok(())
    }
}

/// Remove all symlinks in a directory (non-recursive).
fn remove_symlinks_in(dir: &std::path::Path) -> Result<(), OrbitError> {
    let entries = std::fs::read_dir(dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
        let meta =
            std::fs::symlink_metadata(entry.path()).map_err(|e| OrbitError::Io(e.to_string()))?;
        if meta.file_type().is_symlink() {
            std::fs::remove_file(entry.path()).map_err(|e| OrbitError::Io(e.to_string()))?;
        }
    }
    Ok(())
}

/// Check if a directory is empty.
fn is_dir_empty(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false)
}

fn dir_name_or_fallback(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default")
        .to_string()
}

fn detect_git_remote(cwd: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn ensure_orbit_gitignore_entry(
    workspace_root: &std::path::Path,
    orbit_dir: &std::path::Path,
) -> Result<(), OrbitError> {
    let Some(gitignore_root) = orbit_gitignore_root(workspace_root, orbit_dir) else {
        return Ok(());
    };
    let gitignore_path = gitignore_root.join(".gitignore");
    write_orbit_gitignore_entry(&gitignore_path)
}

fn orbit_gitignore_root<'a>(
    workspace_root: &'a std::path::Path,
    orbit_dir: &'a std::path::Path,
) -> Option<&'a std::path::Path> {
    // Legacy: walking up from a subdir, orbit_dir is `<repo>/.orbit` whose
    // parent is a git repo root.
    if orbit_dir.file_name().and_then(|name| name.to_str()) == Some(".orbit")
        && let Some(repo_root) = orbit_dir.parent()
        && is_git_repo_root(repo_root)
    {
        return Some(repo_root);
    }

    // Default: orbit_dir lives directly inside workspace_root as `.orbit`.
    // If the user passed `--root` to relocate Orbit data outside the workspace
    // (or to a non-`.orbit` basename), skip the gitignore write — there is no
    // `<workspace>/.orbit` directory to ignore.
    if is_git_repo_root(workspace_root) && orbit_dir == workspace_root.join(".orbit") {
        return Some(workspace_root);
    }

    None
}

fn is_git_repo_root(path: &std::path::Path) -> bool {
    path.join(".git").exists()
}

fn write_orbit_gitignore_entry(gitignore_path: &std::path::Path) -> Result<(), OrbitError> {
    let content = match std::fs::read_to_string(gitignore_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(OrbitError::Io(error.to_string())),
    };

    if gitignore_has_orbit_entry(&content) {
        return Ok(());
    }

    let mut next = content;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(".orbit\n");
    std::fs::write(gitignore_path, next).map_err(|error| OrbitError::Io(error.to_string()))
}

fn gitignore_has_orbit_entry(content: &str) -> bool {
    content.lines().any(|line| {
        let line = line.trim();
        matches!(line, ".orbit" | ".orbit/" | "/.orbit" | "/.orbit/")
    })
}
