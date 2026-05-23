use std::path::{Path, PathBuf};

use chrono::Utc;
use clap::Args;
use orbit_common::types::{Workspace, WorkspaceStatus};
use orbit_core::command::agent_rules::{InjectionAction, inject_agent_rules};
use orbit_core::command::init::{
    InitOptions, build_initial_graph, init_workspace_at_root, seed_default_orbitignore,
};
use orbit_core::workspace_registry;
use orbit_core::{OrbitError, OrbitRuntime};

use super::support::{detect_git_remote, dir_name_or_fallback, ensure_orbit_gitignore_entry};

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
    /// Set up PreToolUse learning hooks for auto-detected agent providers.
    #[arg(long)]
    pub hooks: bool,
    /// Inject (or refresh) an Orbit workflow-rules block in CLAUDE.md and AGENTS.md at the workspace root.
    #[arg(long)]
    pub inject_agent_rules: bool,
    /// No-op (kept for backwards compatibility — defaults are always refreshed on init)
    #[arg(long, hide = true)]
    pub refresh_defaults: bool,
}

impl WorkspaceInitArgs {
    pub fn execute_without_runtime(self, root_override: Option<&Path>) -> Result<(), OrbitError> {
        let cwd = std::env::current_dir().map_err(|e| OrbitError::Io(e.to_string()))?;
        let roots = OrbitRuntime::resolve_bootstrap_roots_for_cwd(&cwd, root_override)?;
        let orbit_dir = roots.shared_root;
        let global_root = roots.global_root;
        let registry_path = workspace_registry::registry_path_for(&global_root);
        let mcp = self.mcp;
        let hooks = self.hooks;
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

        if hooks {
            let providers =
                orbit_core::command::hook_install::install_for_workspace(&init_result.root)?;
            if providers.is_empty() {
                println!("  hooks:     no providers auto-detected");
            } else {
                println!("  hooks:     {}", providers.join(", "));
            }
        } else {
            println!("  hooks:     skipped (pass --hooks to set up integrations)");
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
        cwd: &Path,
        orbit_dir: &Path,
        global_root: &Path,
        registry_path: &Path,
    ) -> Result<WorkspaceInitResult, OrbitError> {
        init_workspace_at_root(
            orbit_dir,
            InitOptions {
                refresh_defaults: true,
                global_root_override: Some(global_root.to_path_buf()),
                ..Default::default()
            },
        )?;
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
    root: PathBuf,
    orbit_dir: PathBuf,
}
