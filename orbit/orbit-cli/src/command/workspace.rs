use chrono::Utc;
use clap::{Args, Subcommand};
use orbit_core::command::init::{InitOptions, init_workspace_at_root};
use orbit_core::workspace_registry;
use orbit_core::{OrbitError, OrbitRuntime};
use orbit_types::{Workspace, WorkspaceStatus};

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
        let init_result = self.execute_at_path(&cwd, &registry_path)?;

        println!("workspace '{}' initialized", init_result.name);
        println!("  id:        {}", init_result.id);
        println!("  root:      {}", init_result.root.display());
        println!("  orbit_dir: {}", init_result.orbit_dir.display());
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

        let tasks_dir = orbit_dir.join("tasks");
        std::fs::create_dir_all(&tasks_dir).map_err(|e| OrbitError::Io(e.to_string()))?;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_init_is_idempotent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("create .git dir");

        let registry_path = temp.path().join("home/.orbit/workspaces.json");

        // First init
        let init_args = WorkspaceInitArgs {
            name: None,
            base_branch: "main".to_string(),
            refresh_defaults: false,
        };
        let result1 = init_args
            .execute_at_path(&repo_root, &registry_path)
            .expect("first init should succeed");
        assert_eq!(result1.id, "ws_repo");

        let registry = workspace_registry::load_registry_from(&registry_path).expect("registry");
        assert_eq!(registry.workspaces.len(), 1);
        let first_updated_at = registry.workspaces[0].updated_at;

        // Small delay so updated_at differs
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Second init — should not error
        let init_args2 = WorkspaceInitArgs {
            name: None,
            base_branch: "main".to_string(),
            refresh_defaults: false,
        };
        let result2 = init_args2
            .execute_at_path(&repo_root, &registry_path)
            .expect("second init should succeed (idempotent)");
        assert_eq!(result2.id, "ws_repo");

        let registry = workspace_registry::load_registry_from(&registry_path).expect("registry");
        assert_eq!(
            registry.workspaces.len(),
            1,
            "should still have exactly 1 workspace"
        );
        assert!(
            registry.workspaces[0].updated_at > first_updated_at,
            "updated_at should be refreshed on re-init"
        );
    }

    #[test]
    fn teardown_requires_confirm_flag() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("create .git dir");
        let registry_path = temp.path().join("home/.orbit/workspaces.json");

        let init_args = WorkspaceInitArgs {
            name: None,
            base_branch: "main".to_string(),
            refresh_defaults: false,
        };
        init_args
            .execute_at_path(&repo_root, &registry_path)
            .expect("init");

        let orbit_dir = repo_root.join(".orbit");
        assert!(orbit_dir.is_dir());

        // Without --confirm, teardown should fail
        let runtime = OrbitRuntime::from_data_root(&orbit_dir).expect("runtime");
        let args = WorkspaceTeardownArgs { confirm: false };
        let err = args.execute(&runtime);
        assert!(err.is_err(), "teardown without --confirm should fail");
        assert!(orbit_dir.is_dir(), ".orbit should still exist");
    }

    #[test]
    fn teardown_removes_orbit_dir_and_symlinks() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("create .git dir");
        let registry_path = temp.path().join("home/.orbit/workspaces.json");

        let init_args = WorkspaceInitArgs {
            name: None,
            base_branch: "main".to_string(),
            refresh_defaults: false,
        };
        init_args
            .execute_at_path(&repo_root, &registry_path)
            .expect("init");

        let orbit_dir = repo_root.join(".orbit");
        assert!(orbit_dir.is_dir());

        // Verify symlinks were created
        assert!(repo_root.join(".agents/skills").is_dir());
        assert!(repo_root.join(".claude/skills").is_dir());

        // Execute teardown with --confirm
        let runtime = OrbitRuntime::from_data_root(&orbit_dir).expect("runtime");
        let args = WorkspaceTeardownArgs { confirm: true };
        args.execute(&runtime).expect("teardown should succeed");

        // .orbit/ should be gone
        assert!(!orbit_dir.exists(), ".orbit should be deleted");

        // Symlink dirs should be cleaned up
        assert!(
            !repo_root.join(".agents/skills").exists(),
            ".agents/skills should be gone"
        );
        assert!(
            !repo_root.join(".claude/skills").exists(),
            ".claude/skills should be gone"
        );
    }

    #[test]
    fn workspace_init_seeds_default_artifacts_and_registers_workspace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("create .git dir");

        let registry_path = temp.path().join("home/.orbit/workspaces.json");
        let init_args = WorkspaceInitArgs {
            name: None,
            base_branch: "main".to_string(),
            refresh_defaults: false,
        };

        let init_result = init_args
            .execute_at_path(&repo_root, &registry_path)
            .expect("workspace init should succeed");

        assert_eq!(init_result.id, "ws_repo");
        assert_eq!(init_result.name, "repo");
        assert_eq!(init_result.root, repo_root);
        assert_eq!(init_result.orbit_dir, repo_root.join(".orbit"));

        let orbit_dir = repo_root.join(".orbit");
        assert!(orbit_dir.join("tasks").is_dir(), "tasks dir should exist");
        assert!(orbit_dir.join("skills").is_dir(), "skills dir should exist");
        assert!(
            orbit_dir.join("activities").is_dir(),
            "activities dir should exist"
        );
        assert!(orbit_dir.join("jobs").is_dir(), "jobs dir should exist");
        assert!(
            !orbit_dir.join("config.toml").is_file(),
            "workspace init should not seed config.toml"
        );
        // Scoreboards are only seeded when scoring.enabled = true in config.
        // The default config template sets scoring.enabled = false, so
        // scoreboard files are not expected here.
        assert!(
            std::fs::read_dir(orbit_dir.join("activities"))
                .expect("read activities")
                .next()
                .is_some(),
            "activities dir should contain default files"
        );
        assert!(
            std::fs::read_dir(orbit_dir.join("jobs"))
                .expect("read jobs")
                .next()
                .is_some(),
            "jobs dir should contain default files"
        );

        let default_skill = std::fs::read_dir(orbit_dir.join("skills"))
            .expect("read skills")
            .next()
            .expect("at least one default skill")
            .expect("skill entry")
            .file_name();
        let agents_skill_link = repo_root.join(".agents/skills").join(&default_skill);
        let claude_skill_link = repo_root.join(".claude/skills").join(&default_skill);
        assert!(
            std::fs::symlink_metadata(&agents_skill_link)
                .expect("agents skill link metadata")
                .file_type()
                .is_symlink(),
            "agents skill link should be a symlink"
        );
        assert!(
            std::fs::symlink_metadata(&claude_skill_link)
                .expect("claude skill link metadata")
                .file_type()
                .is_symlink(),
            "claude skill link should be a symlink"
        );

        let registry = workspace_registry::load_registry_from(&registry_path).expect("registry");
        assert_eq!(registry.workspaces.len(), 1);
        let workspace = &registry.workspaces[0];
        assert_eq!(workspace.id, "ws_repo");
        assert_eq!(workspace.name, "repo");
        assert_eq!(workspace.root, repo_root);
        assert_eq!(workspace.orbit_dir, orbit_dir);
    }

    #[test]
    fn workspace_init_always_refreshes_default_artifacts_but_preserves_custom() {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo_root = temp.path().join("repo");
        std::fs::create_dir_all(repo_root.join(".git")).expect("create .git dir");

        let registry_path = temp.path().join("home/.orbit/workspaces.json");
        WorkspaceInitArgs {
            name: None,
            base_branch: "main".to_string(),
            refresh_defaults: false,
        }
        .execute_at_path(&repo_root, &registry_path)
        .expect("workspace init should succeed");

        let orbit_dir = repo_root.join(".orbit");
        let skill_path = orbit_dir.join("skills/orbit/SKILL.md");
        let activity_path = orbit_dir.join("activities/active/implement_change.yaml");
        let job_path = orbit_dir.join("jobs/jobs/job_parallel_task_worker.yaml");
        let custom_skill_path = orbit_dir.join("skills/custom/SKILL.md");

        let original_skill = std::fs::read_to_string(&skill_path).expect("read default skill");
        let original_activity =
            std::fs::read_to_string(&activity_path).expect("read default activity");
        let original_job = std::fs::read_to_string(&job_path).expect("read default job");
        let stale_activity = format!("# stale activity\n{original_activity}");
        let stale_job = format!("# stale job\n{original_job}");

        std::fs::write(&skill_path, "stale skill\n").expect("write stale skill");
        std::fs::write(&activity_path, &stale_activity).expect("write stale activity");
        std::fs::write(&job_path, &stale_job).expect("write stale job");
        std::fs::create_dir_all(custom_skill_path.parent().expect("custom skill parent"))
            .expect("create custom skill dir");
        std::fs::write(&custom_skill_path, "custom skill\n").expect("write custom skill");

        // Plain init (no explicit flag) should refresh stale defaults
        WorkspaceInitArgs {
            name: None,
            base_branch: "main".to_string(),
            refresh_defaults: false,
        }
        .execute_at_path(&repo_root, &registry_path)
        .expect("workspace init should succeed");

        assert_eq!(
            std::fs::read_to_string(&skill_path).expect("read skill after init"),
            original_skill,
            "default skill should be refreshed by plain init"
        );
        assert_eq!(
            std::fs::read_to_string(&activity_path).expect("read activity after init"),
            original_activity,
            "default activity should be refreshed by plain init"
        );
        assert_eq!(
            std::fs::read_to_string(&job_path).expect("read job after init"),
            original_job,
            "default job should be refreshed by plain init"
        );
        // Custom (non-default) artifact must not be touched
        assert_eq!(
            std::fs::read_to_string(&custom_skill_path).expect("read custom skill"),
            "custom skill\n",
            "custom skill should be preserved"
        );
    }
}
