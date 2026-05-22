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
//! [`OrbitRuntime`]. The `engine`, `audit`, `mutation`, and `pipeline` sub-modules
//! provide the high-level operations exposed to command handlers.

pub mod audit;
pub mod builder;
pub mod engine;
pub mod event_bus;
pub mod mutation;
pub(crate) mod orbit_tool_host;
pub mod pipeline;
mod resolve;
pub mod run_audit;
pub(crate) mod run_input;
mod store_delegates;
mod task_reservation_cleanup;
mod v2_host;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use orbit_common::types::{Audit, OrbitError, OrbitEvent, WorkspacePaths};
use orbit_engine::ActivityExecutorRegistry;
use serde_json::Value;

use crate::OrbitContext;
use crate::command::init::ensure_orbit_root_initialized;
use crate::context::ActorIdentity;
use crate::context::OrbitStores;

pub(crate) use orbit_tool_host::build_orbit_tool_host;
pub(crate) use resolve::{
    ResolvedOrbitRoots, resolve_bootstrap_roots, resolve_global_root, resolve_initialize_roots,
    try_resolve_initialized_roots,
};
pub(crate) use store_delegates::TaskRecordUpdateParams;

#[derive(Clone)]
pub struct OrbitRuntime {
    context: OrbitContext,
    activity_executors: Arc<ActivityExecutorRegistry>,
    pub event_log: event_bus::EventLog,
    _temp_dir: Option<Arc<builder::TempDir>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrbitRuntimeRoots {
    pub global_root: PathBuf,
    pub shared_root: PathBuf,
    pub local_root: PathBuf,
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
        ensure_orbit_root_initialized(&roots.global_root, &roots.shared_root)?;
        Self::from_resolved_roots(&roots.global_root, &roots.shared_root, &roots.local_root)
    }

    /// Initialize a runtime against an already-initialized workspace, returning
    /// `Ok(None)` when no initialized workspace is discovered from cwd.
    ///
    /// Unlike [`Self::initialize_with_root_override`], this does not bootstrap
    /// a new `.orbit/` directory. Intended for long-running services like
    /// `orbit mcp serve` that may be invoked from arbitrary directories and
    /// must not silently materialize workspace state.
    pub fn try_initialize_existing(
        root_override: Option<&Path>,
    ) -> Result<Option<Self>, OrbitError> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let Some(resolved) = try_resolve_initialized_roots(&cwd, root_override)? else {
            return Ok(None);
        };
        let global_root = resolve_global_root()?;
        Ok(Some(Self::from_resolved_roots(
            &global_root,
            &resolved.shared_root,
            &resolved.local_root,
        )?))
    }

    pub fn resolve_roots_for_cwd(
        cwd: &Path,
        root_override: Option<&Path>,
    ) -> Result<OrbitRuntimeRoots, OrbitError> {
        let resolved = resolve_initialize_roots(cwd, root_override)?;
        Ok(OrbitRuntimeRoots::new(resolve_global_root()?, resolved))
    }

    pub fn resolve_bootstrap_roots_for_cwd(
        cwd: &Path,
        root_override: Option<&Path>,
    ) -> Result<OrbitRuntimeRoots, OrbitError> {
        let resolved = resolve_bootstrap_roots(cwd, root_override)?;
        Self::bootstrap_roots_from_resolved_roots(resolved, root_override)
    }

    fn bootstrap_roots_from_resolved_roots(
        resolved: ResolvedOrbitRoots,
        root_override: Option<&Path>,
    ) -> Result<OrbitRuntimeRoots, OrbitError> {
        let global_root = if has_explicit_root_override(root_override) {
            resolved.shared_root.clone()
        } else {
            resolve_global_root()?
        };
        Ok(OrbitRuntimeRoots::new(global_root, resolved))
    }

    pub fn from_data_root(data_root: &Path) -> Result<Self, OrbitError> {
        Self::from_resolved_roots(data_root, data_root, data_root)
    }

    pub fn from_roots(global_root: &Path, workspace_root: &Path) -> Result<Self, OrbitError> {
        Self::from_resolved_roots(global_root, workspace_root, workspace_root)
    }

    pub fn from_resolved_roots(
        global_root: &Path,
        shared_root: &Path,
        local_root: &Path,
    ) -> Result<Self, OrbitError> {
        let context = builder::build_context_from_roots(global_root, shared_root, local_root)?;
        Ok(Self {
            activity_executors: build_activity_executor_registry(&context)?,
            context,
            event_log: event_bus::EventLog::default(),
            _temp_dir: None,
        })
    }

    pub fn in_memory() -> Result<Self, OrbitError> {
        let (context, temp_dir) = builder::build_context_in_memory()?;
        Ok(Self {
            activity_executors: build_activity_executor_registry(&context)?,
            context,
            event_log: event_bus::EventLog::default(),
            _temp_dir: Some(Arc::new(temp_dir)),
        })
    }

    pub fn with_policy(mut self, policy: orbit_policy::PolicyEngine) -> Self {
        self.context.set_policy(policy);
        self
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

    pub fn get_job(&self, job_id: &str) -> Result<Option<orbit_common::types::Job>, OrbitError> {
        Err(OrbitError::Execution(format!(
            "v1 job lookup is retired; refusing to resolve job '{job_id}' through OrbitRuntime::get_job. Use schemaVersion: 2 job assets with `orbit job run` or `orbit run` instead."
        )))
    }

    pub fn execution_env_config(&self) -> (bool, Vec<String>) {
        (
            self.context.execution_env_policy().inherit(),
            self.context.execution_env_policy().pass().to_vec(),
        )
    }

    pub fn codex_execution_config(&self) -> (String, Option<String>) {
        (
            self.context.codex_execution_policy().sandbox().to_string(),
            self.context
                .codex_execution_policy()
                .approval_policy()
                .map(ToString::to_string),
        )
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
        self.context.pr_config()
    }

    /// Configured default for the v2 `agent_loop` execution backend (§3.1
    /// precedence step 3). Returns `None` when not set.
    pub fn v2_backend_config(&self) -> Option<&str> {
        self.context.v2_backend()
    }

    /// Default base branch for ship/duel-plan workflows. Sourced
    /// from `[workflow] base_branch` in the active `config.toml`; defaults
    /// to `"main"` when no key is present.
    pub fn workflow_base_branch(&self) -> &str {
        self.context.workflow_base_branch()
    }

    /// Returns the configured `[duel] candidates` list (e.g. ["codex", "claude", "gemini", "grok"]).
    /// Used by `orbit run duel-plan --planner-a ...` overrides to validate explicit families.
    pub fn duel_candidate_families(&self) -> Vec<String> {
        self.context.duel_config().candidates.clone()
    }

    pub(crate) fn duel_config(&self) -> &crate::config::DuelConfig {
        self.context.duel_config()
    }

    /// Build the activity catalog for `target: activity:<name>` resolution
    /// (Phase 4). Loads from the layered Orbit data dirs using §9.1
    /// `MergeByKey` semantics — global provides defaults, workspace overrides.
    ///
    /// The lookup order:
    /// 1. `ORBIT_ACTIVITY_DIR` env var (or legacy `ORBIT_V2_CATALOG_DIR`) as
    ///    a colon-separated list of dirs, highest precedence for smokes/tests.
    /// 2. `<workspace_root>/.orbit/resources/activities/` — workspace-local.
    /// 3. `<global_root>/resources/activities/` — global defaults (seeded by
    ///    `orbit init` from the YAMLs embedded in the binary).
    ///
    /// Missing directories are skipped silently. Directories are loaded from
    /// highest to lowest precedence; the first activity for each name wins.
    /// Duplicate names inside one directory tree are still a hard error
    /// (`CatalogError::DuplicateName`).
    pub fn v2_activity_catalog(
        &self,
    ) -> Result<
        orbit_common::types::activity_job::V2ActivityCatalog,
        orbit_common::types::activity_job::CatalogError,
    > {
        let mut catalog = orbit_common::types::activity_job::V2ActivityCatalog::new();
        for dir in self.v2_activity_catalog_dirs() {
            if !dir.is_dir() {
                continue;
            }
            warn_skipped_retired_activity_assets(
                &dir,
                catalog.load_dir_skipping_retired_prefer_existing(&dir)?,
            );
        }
        let registered_tools: Vec<String> = self
            .tool_registry()
            .schemas()
            .into_iter()
            .map(|schema| schema.name)
            .collect();
        catalog.validate_tool_allowlists(registered_tools.iter().map(String::as_str))?;

        Ok(catalog)
    }

    fn v2_activity_catalog_dirs(&self) -> Vec<std::path::PathBuf> {
        let mut dirs = Vec::new();
        let mut seen = std::collections::BTreeSet::new();

        let env_dirs = std::env::var("ORBIT_ACTIVITY_DIR")
            .ok()
            .or_else(|| std::env::var("ORBIT_V2_CATALOG_DIR").ok());
        if let Some(raw) = env_dirs {
            for entry in raw.split(':').filter(|value| !value.is_empty()) {
                push_unique_activity_dir(&mut dirs, &mut seen, std::path::PathBuf::from(entry));
            }
        }

        push_unique_activity_dir(
            &mut dirs,
            &mut seen,
            self.context.paths().activities_dir.clone(),
        );
        push_unique_activity_dir(
            &mut dirs,
            &mut seen,
            self.context.paths().global_dir.join("resources/activities"),
        );
        dirs
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

    pub(crate) fn paths(&self) -> &WorkspacePaths {
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
        self.stores().executors().list()
    }

    pub fn get_executor_def(
        &self,
        name: &str,
    ) -> Result<Option<orbit_common::types::ExecutorDef>, OrbitError> {
        self.stores().executors().get(name)
    }

    pub fn upsert_executor_def(
        &self,
        def: &orbit_common::types::ExecutorDef,
    ) -> Result<(), OrbitError> {
        self.stores().executors().upsert(def)
    }

    pub fn list_policy_defs(&self) -> Result<Vec<orbit_common::types::PolicyDef>, OrbitError> {
        self.stores().policies().list()
    }

    pub fn get_policy_def(
        &self,
        name: &str,
    ) -> Result<Option<orbit_common::types::PolicyDef>, OrbitError> {
        self.stores().policies().get(name)
    }

    pub fn upsert_policy_def(
        &self,
        def: &orbit_common::types::PolicyDef,
    ) -> Result<(), OrbitError> {
        self.stores().policies().upsert(def)
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
    let defs = context.stores().executors().list()?;
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

fn push_unique_activity_dir(
    dirs: &mut Vec<PathBuf>,
    seen: &mut std::collections::BTreeSet<PathBuf>,
    path: PathBuf,
) {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
    if seen.insert(canonical) {
        dirs.push(path);
    }
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
