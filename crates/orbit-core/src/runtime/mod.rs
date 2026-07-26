//! Runtime bootstrap and the two-root architecture (global + workspace).
//!
//! `OrbitRuntime` is initialized by locating two roots:
//! 1. **Global root** — `~/.orbit/`: houses global config,
//!    the audit SQLite database, skills, and globally-scoped resources.
//! 2. **Workspace root** — the nearest ancestor `.orbit/` directory from cwd:
//!    houses workspace-local tasks, knowledge, optional skill overrides, and runtime state.
//!
//! The `resolve` sub-module implements root discovery. The `builder` sub-module
//! wires together stores, policy, tool registry, and event bus into a complete
//! [`OrbitRuntime`]. The `engine`, `audit`, `mutation`, and `tool_exec` sub-modules
//! provide the high-level operations exposed to command handlers.

pub mod audit;
pub mod builder;
pub mod engine;
pub mod event_bus;
pub mod mutation;
pub(crate) mod orbit_tool_host;
mod resolve;
pub mod run_audit;
pub(crate) mod run_input;
mod task_block_on_run_failure;
mod task_records;
mod task_reservation_cleanup;
pub(crate) mod tool_exec;
mod v2_host;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use orbit_common::types::activity_job::{CatalogDirectory, CatalogDirectoryList};
use orbit_common::types::{Audit, LearningInjectionState, OrbitError, OrbitEvent, WorkspacePaths};
use orbit_engine::ActivityExecutorRegistry;
use orbit_store::{Store, V2AuditEventFilter, V2AuditEventRow, workspace_id_for_orbit_dir};
use serde_json::Value;

use crate::command::activity::DEFAULT_ACTIVITY_FILES;
use crate::command::init::ensure_orbit_root_initialized;
use crate::command::workflow::ShipMode;
use crate::context::ActorIdentity;
use crate::context::OrbitContext;
use crate::context::OrbitStores;

pub use orbit_tool_host::HubCoordinationExecutor;
pub(crate) use orbit_tool_host::build_orbit_tool_host;
pub(crate) use resolve::{resolve_bootstrap_roots, resolve_initialize_roots};
// `pub` for the runtime-less `orbit migrate --dry-run` inspection that moved
// to `orbit-cmd` [ORB-10016].
pub use resolve::{
    ResolvedOrbitRoots, WorkspaceRootHint, resolve_bootstrap_roots_with_hint,
    resolve_initialize_roots_with_hint, try_resolve_initialized_roots_with_hint,
};
// `pub` for the runtime-less `orbit migrate --dry-run` inspection that moved
// to `orbit-cmd` [ORB-10016].
pub use resolve::{resolve_global_root, try_resolve_initialized_roots};
pub(crate) use task_records::TaskRecordUpdateParams;

#[derive(Clone)]
pub struct OrbitRuntime {
    context: OrbitContext,
    workspace_binding: Option<Arc<WorkspaceRuntimeBinding>>,
    activity_executors: Arc<ActivityExecutorRegistry>,
    pub event_log: event_bus::EventLog,
    /// Outcome of the [ORB-10012] workspace-layout pre-flight that ran when
    /// this runtime opened (empty `applied` when the layout was already
    /// current). Surfaced by `orbit migrate`.
    layout_report: Arc<orbit_store::layout::LayoutUpgradeReport>,
    _temp_dir: Option<Arc<builder::TempDir>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrbitRuntimeRoots {
    pub global_root: PathBuf,
    pub shared_root: PathBuf,
    pub local_root: PathBuf,
}

/// Registry-neutral metadata supplied by a higher-level workspace catalog.
///
/// `orbit-core` can construct a runtime without this binding for standalone
/// compatibility. Multi-host composition supplies it explicitly so runtime
/// path and ship-mode decisions do not have to reopen a registry owned by a
/// higher feature crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRuntimeBinding {
    pub workspace_id: String,
    pub repo_root: PathBuf,
    pub ship_mode: ShipMode,
}

impl OrbitRuntimeRoots {
    fn new(global_root: PathBuf, resolved: ResolvedOrbitRoots) -> Self {
        Self {
            global_root,
            shared_root: resolved.shared_root,
            local_root: resolved.local_root,
        }
    }
}

impl OrbitRuntime {
    pub fn initialize() -> Result<Self, OrbitError> {
        Self::initialize_with_root_override(None)
    }

    pub fn initialize_with_root_override(root_override: Option<&Path>) -> Result<Self, OrbitError> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let roots = Self::resolve_roots_for_cwd(&cwd, root_override)?;
        Self::initialize_from_resolved_roots(roots, None)
    }

    /// Initialize from roots and optional higher-level workspace metadata.
    /// This keeps bootstrap/layout behavior in Core while allowing a feature
    /// owner to resolve catalog hints and bindings externally.
    pub fn initialize_from_resolved_roots(
        roots: OrbitRuntimeRoots,
        binding: Option<WorkspaceRuntimeBinding>,
    ) -> Result<Self, OrbitError> {
        ensure_orbit_root_initialized(&roots.global_root, &roots.shared_root)?;
        match binding {
            Some(binding) => Self::from_resolved_roots_with_binding(
                &roots.global_root,
                &roots.shared_root,
                &roots.local_root,
                binding,
            ),
            None => {
                Self::from_resolved_roots(&roots.global_root, &roots.shared_root, &roots.local_root)
            }
        }
    }

    pub fn resolve_roots_for_cwd(
        cwd: &Path,
        root_override: Option<&Path>,
    ) -> Result<OrbitRuntimeRoots, OrbitError> {
        let resolved = resolve_initialize_roots(cwd, root_override)?;
        // Only the explicit `--root` flag pins the global registry root to the
        // isolated root here, so `workspace list` / `show --root <r>` read
        // `<r>/workspaces.json` — the same file `workspace init --root <r>`
        // writes — instead of `$HOME/.orbit/workspaces.json` [ORB-10218].
        // `ORBIT_ROOT` stays a workspace selector that leaves the global root at
        // `$HOME/.orbit` (see `orbit_root_env_selects_workspace_but_not_global_root`).
        Self::roots_from_resolved(resolved, root_override.is_some())
    }

    /// Resolve an initialized workspace using a registry-neutral catalog hint.
    pub fn resolve_roots_for_cwd_with_hint(
        cwd: &Path,
        root_override: Option<&Path>,
        hint: Option<&WorkspaceRootHint>,
    ) -> Result<OrbitRuntimeRoots, OrbitError> {
        let resolved = resolve_initialize_roots_with_hint(cwd, root_override, hint)?;
        Self::roots_from_resolved(resolved, root_override.is_some())
    }

    pub fn resolve_bootstrap_roots_for_cwd(
        cwd: &Path,
        root_override: Option<&Path>,
    ) -> Result<OrbitRuntimeRoots, OrbitError> {
        let resolved = resolve_bootstrap_roots(cwd, root_override)?;
        Self::roots_from_resolved(resolved, has_explicit_root_override(root_override))
    }

    /// Resolve bootstrap roots using a registry-neutral catalog hint.
    pub fn resolve_bootstrap_roots_for_cwd_with_hint(
        cwd: &Path,
        root_override: Option<&Path>,
        hint: Option<&WorkspaceRootHint>,
    ) -> Result<OrbitRuntimeRoots, OrbitError> {
        let resolved = resolve_bootstrap_roots_with_hint(cwd, root_override, hint)?;
        Self::roots_from_resolved(resolved, has_explicit_root_override(root_override))
    }

    /// Selects the global root for a resolved workspace: when
    /// `pin_global_to_shared` is set the global root is the isolated shared
    /// root (registry lookups target the custom root), otherwise it falls back
    /// to `$HOME/.orbit`.
    fn roots_from_resolved(
        resolved: ResolvedOrbitRoots,
        pin_global_to_shared: bool,
    ) -> Result<OrbitRuntimeRoots, OrbitError> {
        let global_root = if pin_global_to_shared {
            resolved.shared_root.clone()
        } else {
            resolve_global_root()?
        };
        Ok(OrbitRuntimeRoots::new(global_root, resolved))
    }

    pub fn from_roots(global_root: &Path, workspace_root: &Path) -> Result<Self, OrbitError> {
        Self::from_resolved_roots(global_root, workspace_root, workspace_root)
    }

    /// Construct a runtime with registry-neutral workspace metadata supplied
    /// by a higher-level catalog owner.
    pub fn from_roots_with_binding(
        global_root: &Path,
        workspace_root: &Path,
        binding: WorkspaceRuntimeBinding,
    ) -> Result<Self, OrbitError> {
        Self::from_resolved_roots_with_binding(global_root, workspace_root, workspace_root, binding)
    }

    pub fn from_resolved_roots(
        global_root: &Path,
        shared_root: &Path,
        local_root: &Path,
    ) -> Result<Self, OrbitError> {
        Self::from_resolved_roots_inner(global_root, shared_root, local_root, None)
    }

    /// Construct a two-root runtime with registry-neutral workspace metadata.
    pub fn from_resolved_roots_with_binding(
        global_root: &Path,
        shared_root: &Path,
        local_root: &Path,
        binding: WorkspaceRuntimeBinding,
    ) -> Result<Self, OrbitError> {
        Self::from_resolved_roots_inner(global_root, shared_root, local_root, Some(binding))
    }

    fn from_resolved_roots_inner(
        global_root: &Path,
        shared_root: &Path,
        local_root: &Path,
        binding: Option<WorkspaceRuntimeBinding>,
    ) -> Result<Self, OrbitError> {
        // [ORB-10012] Workspace-layout pre-flight: compare the `.orbit/`
        // layout marker against this binary before anything opens the store.
        // Older layouts auto-migrate here (matching how the SQLite schema
        // ledger auto-applies inside `Store::open` below); a layout newer
        // than this binary refuses with a downgrade-guard error. Runs before
        // `build_context_from_roots` because a layout migration may need to
        // restructure state the stores are about to open. Up-to-date cost:
        // one marker-file read.
        let layout_report = orbit_store::layout::upgrade_workspace_layout(shared_root)?;
        let context = builder::build_context_from_roots(
            global_root,
            shared_root,
            local_root,
            binding.as_ref(),
        )?;
        let runtime = Self {
            activity_executors: build_activity_executor_registry(&context)?,
            context,
            workspace_binding: binding.map(Arc::new),
            event_log: event_bus::EventLog::default(),
            layout_report: Arc::new(layout_report),
            _temp_dir: None,
        };
        // [ORB-10002] Workspace-open orphan scan: job runs stuck in `running`
        // whose recorded owner process is conclusively gone flip to
        // `interrupted` so dashboards and `orbit job resume` see them.
        // Best-effort — a scan failure must never block opening the runtime.
        runtime.reconcile_stale_job_runs_on_open();
        Ok(runtime)
    }

    pub fn in_memory() -> Result<Self, OrbitError> {
        let (context, temp_dir) = builder::build_context_in_memory()?;
        Ok(Self {
            activity_executors: build_activity_executor_registry(&context)?,
            context,
            workspace_binding: None,
            event_log: event_bus::EventLog::default(),
            layout_report: Arc::new(orbit_store::layout::LayoutUpgradeReport::default()),
            _temp_dir: Some(Arc::new(temp_dir)),
        })
    }

    /// Outcome of the workspace-layout pre-flight that ran when this runtime
    /// opened: which layout migrations (if any) were auto-applied.
    pub fn layout_upgrade_report(&self) -> &orbit_store::layout::LayoutUpgradeReport {
        &self.layout_report
    }

    pub fn with_actor(mut self, actor: ActorIdentity) -> Self {
        self.context.set_actor(actor);
        self
    }

    /// Returns in-process events recorded during this session only. Not persisted across process
    /// boundaries — the log is empty at startup and discarded on exit. For the persistent CLI
    /// audit log written on every invocation, see [`OrbitRuntime::list_audit_events`].
    pub fn list_session_events(&self, limit: usize) -> Result<Vec<Audit>, OrbitError> {
        let events = self.event_log.snapshot();
        let audits = events
            .into_iter()
            .enumerate()
            .map(|(idx, event)| orbit_event_to_audit((idx + 1) as i64, event))
            .rev()
            .take(limit)
            .collect();
        Ok(audits)
    }

    pub fn shared_root(&self) -> PathBuf {
        self.context.shared_root().to_path_buf()
    }

    pub fn local_root(&self) -> PathBuf {
        self.context.local_root().to_path_buf()
    }

    pub fn data_root(&self) -> PathBuf {
        self.shared_root()
    }

    pub fn global_root(&self) -> PathBuf {
        self.context.global_root().to_path_buf()
    }

    /// Higher-level workspace metadata used to construct this runtime, when
    /// the caller supplied an authoritative binding.
    pub fn workspace_runtime_binding(&self) -> Option<&WorkspaceRuntimeBinding> {
        self.workspace_binding.as_deref()
    }

    /// Returns the effective config.toml path.
    /// Workspace config replaces global if present; otherwise global.
    pub fn config_path(&self) -> PathBuf {
        let ws_config = self.shared_root().join("config.toml");
        if ws_config.exists() && self.shared_root() != self.global_root() {
            ws_config
        } else {
            self.global_root().join("config.toml")
        }
    }

    pub fn persistence_config_json(&self) -> Value {
        self.context.persistence().as_json_value()
    }

    pub fn sqlite_store(&self) -> Result<Store, OrbitError> {
        Store::open(&self.context.persistence().audit_db)
    }

    pub fn workspace_id(&self) -> Result<String, OrbitError> {
        workspace_id_for_orbit_dir(&self.context.paths().orbit_dir)
    }

    pub fn get_session_learning_state(
        &self,
        session_id: &str,
    ) -> Result<Option<LearningInjectionState>, OrbitError> {
        let workspace_id = self.workspace_id()?;
        self.sqlite_store()?
            .get_session_learning_state(&workspace_id, session_id)
    }

    pub fn upsert_session_learning_state(
        &self,
        session_id: &str,
        state: &LearningInjectionState,
    ) -> Result<(), OrbitError> {
        let workspace_id = self.workspace_id()?;
        self.sqlite_store()?
            .upsert_session_learning_state(&workspace_id, session_id, state)
    }

    pub fn list_v2_audit_events(
        &self,
        mut filter: V2AuditEventFilter,
    ) -> Result<Vec<V2AuditEventRow>, OrbitError> {
        if filter.workspace_id.trim().is_empty() {
            filter.workspace_id = self.workspace_id()?;
        }
        self.sqlite_store()?.list_v2_audit_events(&filter)
    }

    pub fn insert_v2_audit_event(
        &self,
        params: &orbit_store::V2AuditEventInsertParams,
    ) -> Result<(), OrbitError> {
        self.sqlite_store()?.insert_v2_audit_event(params)
    }

    pub fn task_approval_required_for_agent(&self) -> bool {
        self.context.task_approval_required_for_agent()
    }

    pub fn task_delegate_approval(&self) -> bool {
        self.context.task_delegate_approval()
    }

    pub fn scoring_enabled(&self) -> bool {
        self.context.scoring_enabled()
    }

    pub fn graph_editing(&self) -> bool {
        self.context.graph_editing()
    }

    pub fn pr_config(&self) -> &orbit_engine::PrConfig {
        self.context.settings().pr_config()
    }

    /// Configured default for the v2 `agent_loop` execution backend (§3.1
    /// precedence step 3). Returns `None` when not set.
    pub fn v2_backend_config(&self) -> Option<&str> {
        self.context.settings().v2_backend()
    }

    /// Default base branch for ship/duel-plan workflows. Sourced
    /// from `[workflow] base_branch` in the active `config.toml`; defaults
    /// to `"main"` when no key is present.
    pub fn workflow_base_branch(&self) -> &str {
        self.context.settings().workflow_base_branch()
    }

    /// Whether this workspace opted into unattended ship dispatch
    /// (`[workflow] auto_ship` in the active `config.toml`; defaults to
    /// `false`). Consulted by `orbit run ship-sweep` and other schedulers
    /// before dispatching ship runs nobody explicitly asked for.
    pub fn workflow_auto_ship(&self) -> bool {
        self.context.settings().workflow_auto_ship()
    }

    /// Whether this workspace declared itself a routine source
    /// (`[routines] role = "source"` in the active `config.toml`; defaults
    /// to `false`). Consulted by `orbit sweep` before loading routine
    /// definitions nobody registered explicitly.
    pub fn routines_source(&self) -> bool {
        self.context.settings().routines_source()
    }

    /// Returns the configured `[duel] candidates` list (e.g. ["codex", "claude", "gemini", "grok"]).
    /// Used by `orbit run duel-plan --planner-a ...` overrides to validate explicit families.
    pub fn duel_candidate_families(&self) -> Vec<String> {
        self.context.settings().duel_config().candidates.clone()
    }

    pub(crate) fn duel_config(&self) -> &crate::config::DuelConfig {
        self.context.settings().duel_config()
    }

    /// Build the activity catalog for `target: activity:<name>` resolution
    /// (Phase 4). Execution keeps shipped global activities authoritative:
    /// workspace-local assets can add new names, but cannot shadow binary
    /// defaults.
    ///
    /// The lookup order:
    /// 1. `ORBIT_ACTIVITY_DIR` env var (or legacy `ORBIT_V2_CATALOG_DIR`) as
    ///    a colon-separated list of dirs, highest precedence for smokes/tests.
    /// 2. `<global_root>/resources/activities/` — global defaults (seeded by
    ///    `orbit init` from the YAMLs embedded in the binary).
    /// 3. `<workspace_root>/.orbit/resources/activities/` — workspace-local
    ///    additions. Names matching shipped defaults are ignored unless an
    ///    explicit env catalog already supplied that name.
    ///
    /// Missing directories are skipped silently. Directories are loaded from
    /// highest to lowest precedence; the first activity for each name wins,
    /// and workspace-local shipped names are skipped even when the global file
    /// is missing.
    /// Duplicate names inside one directory tree are still a hard error
    /// (`CatalogError::DuplicateName`).
    pub fn v2_activity_catalog(
        &self,
    ) -> Result<
        orbit_common::types::activity_job::V2ActivityCatalog,
        orbit_common::types::activity_job::CatalogError,
    > {
        use orbit_common::types::activity_job::V2ActivityCatalog;

        let mut catalog = V2ActivityCatalog::new();
        for dir in self.v2_activity_catalog_dirs() {
            if !dir.path().is_dir() {
                continue;
            }
            // L-0060 / ORB-00356: name-based execution keeps shipped defaults
            // authoritative over workspace catalogs.
            match dir.kind() {
                V2ActivityCatalogDirKind::Explicit | V2ActivityCatalogDirKind::Global => {
                    warn_skipped_retired_activity_assets(
                        dir.path(),
                        catalog.load_dir_skipping_retired_prefer_existing(dir.path())?,
                    );
                }
                V2ActivityCatalogDirKind::WorkspaceLocal => {
                    warn_skipped_retired_activity_assets(
                        dir.path(),
                        catalog.load_dir_skipping_retired_prefer_existing_where(
                            dir.path(),
                            |name| !is_default_activity_name(name),
                        )?,
                    );
                }
            }
        }
        let registered_tools = self.allowlist_known_tool_names();
        catalog.validate_tool_allowlists(registered_tools.iter().map(String::as_str))?;

        Ok(catalog)
    }

    /// Tool names valid as activity allowlist targets.
    ///
    /// This is the registry's builtin schema set. CLI-only features such as
    /// `orbit graph` are not valid activity tool grants.
    /// `pub` for the direct v2 activity runner in `orbit-cmd` [ORB-10016].
    pub fn allowlist_known_tool_names(&self) -> Vec<String> {
        self.tool_registry()
            .schemas()
            .into_iter()
            .map(|schema| schema.name)
            .collect()
    }

    fn v2_activity_catalog_dirs(&self) -> Vec<CatalogDirectory<V2ActivityCatalogDirKind>> {
        let mut dirs = CatalogDirectoryList::default();

        let env_dirs = std::env::var("ORBIT_ACTIVITY_DIR")
            .ok()
            .or_else(|| std::env::var("ORBIT_V2_CATALOG_DIR").ok());
        if let Some(raw) = env_dirs {
            for entry in raw.split(':').filter(|value| !value.is_empty()) {
                dirs.push(
                    std::path::PathBuf::from(entry),
                    V2ActivityCatalogDirKind::Explicit,
                );
            }
        }

        dirs.push(
            self.context.paths().global_dir.join("resources/activities"),
            V2ActivityCatalogDirKind::Global,
        );
        dirs.push(
            self.context.paths().activities_dir.clone(),
            V2ActivityCatalogDirKind::WorkspaceLocal,
        );
        dirs.into_vec()
    }

    pub(crate) fn actor(&self) -> &ActorIdentity {
        self.context.actor()
    }

    pub(crate) fn actor_label(&self) -> &str {
        self.context.actor().label.as_str()
    }

    pub(crate) fn policy_engine(&self) -> &orbit_policy::PolicyEngine {
        self.context.policy()
    }

    pub(crate) fn tool_registry(&self) -> &orbit_tools::ToolRegistry {
        self.context.registry()
    }

    pub(crate) fn stores(&self) -> &OrbitStores {
        self.context.stores()
    }

    pub(crate) fn skill_catalog(&self) -> &crate::skill_catalog::SkillCatalog {
        self.context.skill_catalog()
    }

    /// Resolved workspace paths. `pub` for the command surfaces extracted to
    /// `orbit-cmd` [ORB-10016].
    pub fn paths(&self) -> &WorkspacePaths {
        self.context.paths()
    }

    pub(crate) fn data_root_path(&self) -> &Path {
        self.shared_root_path()
    }

    pub(crate) fn shared_root_path(&self) -> &Path {
        self.context.shared_root()
    }

    pub(crate) fn execution_env_policy(&self) -> &crate::config::ExecutionEnvPolicy {
        self.context.execution_env_policy()
    }

    pub(crate) fn codex_execution_policy(&self) -> &crate::config::CodexExecutionPolicy {
        self.context.codex_execution_policy()
    }

    pub(crate) fn activity_executor_registry(&self) -> &ActivityExecutorRegistry {
        self.activity_executors.as_ref()
    }

    pub fn list_executor_defs(&self) -> Result<Vec<orbit_common::types::ExecutorDef>, OrbitError> {
        self.stores().executors().list_executor_defs()
    }

    pub fn get_executor_def(
        &self,
        name: &str,
    ) -> Result<Option<orbit_common::types::ExecutorDef>, OrbitError> {
        self.stores().executors().get_executor_def(name)
    }

    pub fn upsert_executor_def(
        &self,
        def: &orbit_common::types::ExecutorDef,
    ) -> Result<(), OrbitError> {
        self.stores().executors().upsert_executor_def(def)
    }

    pub fn list_policy_defs(&self) -> Result<Vec<orbit_common::types::PolicyDef>, OrbitError> {
        self.stores().policies().list_policy_defs()
    }

    pub fn get_policy_def(
        &self,
        name: &str,
    ) -> Result<Option<orbit_common::types::PolicyDef>, OrbitError> {
        self.stores().policies().get_policy_def(name)
    }

    pub fn upsert_policy_def(
        &self,
        def: &orbit_common::types::PolicyDef,
    ) -> Result<(), OrbitError> {
        self.stores().policies().upsert_policy_def(def)
    }
}

fn has_explicit_root_override(root_override: Option<&Path>) -> bool {
    root_override.is_some()
        || std::env::var("ORBIT_ROOT").is_ok_and(|value| !value.trim().is_empty())
}

fn build_activity_executor_registry(
    context: &OrbitContext,
) -> Result<Arc<ActivityExecutorRegistry>, OrbitError> {
    let mut registry = ActivityExecutorRegistry::with_builtins();
    let defs = context.stores().executors().list_executor_defs()?;
    registry.load_from_defs(&defs);
    Ok(Arc::new(registry))
}

fn warn_skipped_retired_activity_assets(dir: &Path, skipped: Vec<PathBuf>) {
    if skipped.is_empty() {
        return;
    }
    tracing::warn!(
        target: "orbit.core.assets",
        count = skipped.len(),
        dir = %dir.display(),
        "skipped retired schemaVersion 1 activity assets while loading",
    );
}

#[derive(Clone, Copy)]
enum V2ActivityCatalogDirKind {
    Explicit,
    Global,
    WorkspaceLocal,
}

fn is_default_activity_name(name: &str) -> bool {
    DEFAULT_ACTIVITY_FILES
        .iter()
        .any(|(default_name, _)| *default_name == name)
}

fn orbit_event_to_audit(id: i64, event: OrbitEvent) -> Audit {
    let payload = serde_json::to_value(&event).unwrap_or(Value::Null);
    let event_type = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_string();

    Audit {
        id,
        event_type: event_type.clone(),
        payload,
        message: event_type,
        created_at: Utc::now(),
    }
}
