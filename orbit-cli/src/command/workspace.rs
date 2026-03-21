use chrono::Utc;
use clap::{Args, Subcommand};
use orbit_core::workspace_registry;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_types::{Workspace, WorkspaceStatus};

use crate::command::Execute;

#[derive(Args)]
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
    /// Bind the current directory to a workspace
    Use(WorkspaceUseArgs),
    /// Show the current workspace
    Show(WorkspaceShowArgs),
    /// Remove a workspace from the registry (does not delete .orbit)
    Remove(WorkspaceRemoveArgs),
}

#[derive(Args)]
pub struct WorkspaceInitArgs {
    /// Workspace name (defaults to directory name)
    #[arg(long)]
    pub name: Option<String>,
    /// Base branch for this workspace (default: main)
    #[arg(long, default_value = "main")]
    pub base_branch: String,
}

#[derive(Args)]
pub struct WorkspaceListArgs {}

#[derive(Args)]
pub struct WorkspaceUseArgs {
    /// Workspace name or id
    pub workspace: String,
}

#[derive(Args)]
pub struct WorkspaceShowArgs {}

#[derive(Args)]
pub struct WorkspaceRemoveArgs {
    /// Workspace name or id
    pub workspace: String,
}

impl Execute for WorkspaceCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self.command {
            WorkspaceSubcommand::Init(_) => {
                // Init is handled without runtime in main.rs
                unreachable!("workspace init should be handled before runtime initialization")
            }
            WorkspaceSubcommand::List(args) => args.execute(runtime),
            WorkspaceSubcommand::Use(args) => args.execute(runtime),
            WorkspaceSubcommand::Show(args) => args.execute(runtime),
            WorkspaceSubcommand::Remove(args) => args.execute(runtime),
        }
    }
}

impl WorkspaceInitArgs {
    pub fn execute_without_runtime(self) -> Result<(), OrbitError> {
        let cwd = std::env::current_dir().map_err(|e| OrbitError::Io(e.to_string()))?;
        let orbit_dir = cwd.join(".orbit");
        // Workspace .orbit/ only contains tasks/ by default
        let tasks_dir = orbit_dir.join("tasks");
        std::fs::create_dir_all(&tasks_dir).map_err(|e| OrbitError::Io(e.to_string()))?;

        let name = self.name.unwrap_or_else(|| dir_name_or_fallback(&cwd));

        let id = format!("ws_{name}");
        let git_remote = detect_git_remote(&cwd);

        let ws = Workspace {
            id: id.clone(),
            name: name.clone(),
            root: cwd.clone(),
            orbit_dir: orbit_dir.clone(),
            git_remote,
            base_branch: self.base_branch,
            status: WorkspaceStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let registry_path = workspace_registry::registry_path()?;
        let mut registry = workspace_registry::load_registry_from(&registry_path)?;
        workspace_registry::register_workspace(&mut registry, ws)?;
        workspace_registry::save_registry_to(&registry, &registry_path)?;

        println!("workspace '{}' initialized", name);
        println!("  id:        {}", id);
        println!("  root:      {}", cwd.display());
        println!("  orbit_dir: {}", orbit_dir.display());
        Ok(())
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

        println!("{:<20} {:<12} {:<8} {}", "NAME", "ID", "STATUS", "ROOT");
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

impl Execute for WorkspaceUseArgs {
    fn execute(self, _runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let cwd = std::env::current_dir().map_err(|e| OrbitError::Io(e.to_string()))?;
        let registry_path = workspace_registry::registry_path()?;
        let mut registry = workspace_registry::load_registry_from(&registry_path)?;

        let ws = workspace_registry::find_workspace(&registry, &self.workspace)
            .ok_or_else(|| OrbitError::WorkspaceNotFound(self.workspace.clone()))?;
        let ws_id = ws.id.clone();
        let ws_name = ws.name.clone();

        workspace_registry::set_path_override(&mut registry, cwd.clone(), &ws_id)?;
        workspace_registry::save_registry_to(&registry, &registry_path)?;

        println!("workspace '{}' bound to {}", ws_name, cwd.display());
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
