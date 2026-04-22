use chrono::Utc;
use clap::{Args, Subcommand};
use orbit_common::types::{Workspace, WorkspaceStatus};
use orbit_core::command::init::{InitOptions, init_workspace_at_root};
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
    /// Skip automatic MCP client integration setup.
    #[arg(long)]
    pub no_mcp: bool,
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
    pub fn execute_without_runtime(self) -> Result<(), OrbitError> {
        let cwd = std::env::current_dir().map_err(|e| OrbitError::Io(e.to_string()))?;
        let registry_path = workspace_registry::registry_path()?;
        let no_mcp = self.no_mcp;
        let init_result = self.execute_at_path(&cwd, &registry_path)?;

        println!("workspace '{}' initialized", init_result.name);
        println!("  id:        {}", init_result.id);
        println!("  root:      {}", init_result.root.display());
        println!("  orbit_dir: {}", init_result.orbit_dir.display());

        if no_mcp {
            println!("  mcp:       skipped (--no-mcp)");
        } else {
            let providers = crate::command::mcp::init_auto_for_workspace(
                &init_result.root,
                &init_result.orbit_dir,
            )?;
            if providers.is_empty() {
                println!("  mcp:       no providers auto-detected");
            } else {
                println!("  mcp:       {}", providers.join(", "));
            }
        }

        // Build the knowledge graph
        eprintln!("graph build: scanning {}", init_result.root.display());
        let config = orbit_knowledge::pipeline::context::BuildConfig {
            repo_path: init_result.root.clone(),
            output_dir: init_result.orbit_dir.join("knowledge"),
            incremental: false,
            ref_name: None,
        };
        match orbit_knowledge::pipeline::run_build(config) {
            Ok(ctx) => {
                eprintln!(
                    "graph build: {} dirs, {} files, {} symbols",
                    ctx.graph.dirs.len(),
                    ctx.graph.files.len(),
                    ctx.graph.leaves.len(),
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
        registry_path: &std::path::Path,
    ) -> Result<WorkspaceInitResult, OrbitError> {
        let orbit_dir = cwd.join(".orbit");
        init_workspace_at_root(
            &orbit_dir,
            InitOptions {
                refresh_defaults: true,
                ..Default::default()
            },
        )?;

        let name = self.name.unwrap_or_else(|| dir_name_or_fallback(cwd));

        let id = format!("ws_{name}");
        let git_remote = detect_git_remote(cwd);

        let ws = Workspace {
            id: id.clone(),
            name: name.clone(),
            root: cwd.to_path_buf(),
            orbit_dir: orbit_dir.clone(),
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
            orbit_dir,
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
mod tests {
    use std::sync::Mutex;

    use tempfile::tempdir;

    use super::WorkspaceInitArgs;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn workspace_init_seeds_auto_detected_mcp_configs() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let workspace = tempdir().expect("workspace tempdir");
        let home = tempdir().expect("home tempdir");

        std::fs::create_dir_all(workspace.path().join(".claude")).expect("create .claude");
        std::fs::create_dir_all(workspace.path().join(".gemini")).expect("create .gemini");
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
            no_mcp: false,
            refresh_defaults: false,
        }
        .execute_without_runtime();

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
    }

    #[test]
    fn workspace_init_respects_no_mcp_flag() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let workspace = tempdir().expect("workspace tempdir");
        let home = tempdir().expect("home tempdir");

        std::fs::create_dir_all(workspace.path().join(".claude")).expect("create .claude");
        std::fs::create_dir_all(workspace.path().join(".gemini")).expect("create .gemini");
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
            no_mcp: true,
            refresh_defaults: false,
        }
        .execute_without_runtime();

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
    }
}

impl Execute for WorkspaceListArgs {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let registry_path = workspace_registry::registry_path()?;
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
        let registry_path = workspace_registry::registry_path()?;
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
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let registry_path = workspace_registry::registry_path()?;
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
        let global_dir = workspace_registry::global_orbit_dir()?;

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
        let registry_path = workspace_registry::registry_path()?;
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

        // 2. Remove skill symlinks from .agents/skills/ and .claude/skills/
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
