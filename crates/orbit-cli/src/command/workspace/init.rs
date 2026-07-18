use std::path::{Path, PathBuf};

use chrono::Utc;
use clap::Args;
use orbit_cmd::agent_rules::{InjectionAction, inject_agent_rules};
use orbit_common::types::{Workspace, WorkspaceCheckout, WorkspaceStatus};
use orbit_core::command::init::{InitOptions, init_workspace_at_root, seed_default_orbitignore};
use orbit_core::workspace_registry;
use orbit_core::{OrbitError, OrbitRuntime};

use super::support::{detect_git_remote, dir_name_or_fallback, ensure_orbit_gitignore_entry};

#[derive(Args)]
pub struct WorkspaceInitArgs {
    /// Workspace name (defaults to directory name)
    #[arg(long)]
    pub name: Option<String>,
    /// Base branch for this workspace (default: main)
    ///
    /// Kept optional so re-initializing an existing workspace can distinguish
    /// an omitted value from an explicit request to reset it to `main`.
    #[arg(long)]
    pub base_branch: Option<String>,
    /// Ship-pipeline mode for this workspace: `pr` or `local`. When omitted, the
    /// effective mode defaults to `local` (only PR-gated workspaces set `pr`).
    #[arg(long, value_name = "MODE")]
    pub ship_mode: Option<String>,
    /// Seed the local task-id allocator so the next task id is N (e.g. hand this
    /// machine a disjoint id range like 10000+). The counter only moves forward;
    /// a value below the current position is refused.
    #[arg(long, value_name = "N")]
    pub task_id_start: Option<u32>,
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
        let task_id_start = self.task_id_start;
        let init_result = self.execute_at_path(&cwd, &orbit_dir, &global_root, &registry_path)?;

        println!("workspace '{}' initialized", init_result.name);
        println!("  id:        {}", init_result.id);
        println!("  root:      {}", init_result.root.display());
        println!("  orbit_dir: {}", init_result.orbit_dir.display());

        if let Some(start) = task_id_start {
            let outcome =
                orbit_core::command::task_migration::seed_task_id_start(&global_root, start)?;
            if outcome.changed {
                println!("  id_start:  allocator seeded to ORB-{:05}", outcome.next);
            } else {
                println!(
                    "  id_start:  allocator already at ORB-{:05} (unchanged)",
                    outcome.next
                );
            }
        }

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
            let providers = orbit_cmd::hook_install::install_for_workspace(&init_result.root)?;
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

        // The code graph is served by orbit-graph (v2), which indexes lazily on
        // first query — no build step at init time (ORB-00391).
        println!("  graph:     indexed on demand by orbit-graph");

        Ok(())
    }

    fn execute_at_path(
        self,
        cwd: &Path,
        orbit_dir: &Path,
        global_root: &Path,
        registry_path: &Path,
    ) -> Result<WorkspaceInitResult, OrbitError> {
        // Validate before bootstrapping any workspace state so invalid modes
        // fail closed at the command boundary.
        if let Some(mode) = self.ship_mode.as_deref() {
            orbit_core::ShipMode::parse(mode)?;
        }

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

        let mut registry = workspace_registry::load_registry_from(registry_path)?;
        if let Some(existing) = registry.workspaces.iter_mut().find(|w| w.id == id) {
            if let Some(ship_mode) = self.ship_mode {
                existing.ship_mode = Some(ship_mode);
            }
            if let Some(base_branch) = self.base_branch {
                existing.base_branch = base_branch;
            }
            existing.updated_at = Utc::now();
            if let Some(checkout) = registry
                .checkouts
                .iter_mut()
                .find(|checkout| checkout.workspace_id == id)
            {
                checkout.repo_root = cwd.to_path_buf();
                checkout.orbit_dir = orbit_dir.to_path_buf();
            } else {
                workspace_registry::register_checkout(
                    &mut registry,
                    WorkspaceCheckout::owner(
                        id.clone(),
                        cwd.to_path_buf(),
                        orbit_dir.to_path_buf(),
                    ),
                )?;
            }
        } else {
            let now = Utc::now();
            let ws = Workspace {
                id: id.clone(),
                name: name.clone(),
                owner_machine_id: None,
                git_remote,
                ship_mode: self.ship_mode,
                base_branch: self.base_branch.unwrap_or_else(|| "main".to_string()),
                status: WorkspaceStatus::Active,
                created_at: now,
                updated_at: now,
            };
            workspace_registry::register_workspace(&mut registry, ws)?;
            workspace_registry::register_checkout(
                &mut registry,
                WorkspaceCheckout::owner(id.clone(), cwd.to_path_buf(), orbit_dir.to_path_buf()),
            )?;
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
