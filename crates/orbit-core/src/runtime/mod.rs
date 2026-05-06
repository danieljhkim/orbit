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
mod engine;
pub mod event_bus;
pub mod mutation;
mod orbit_tool_host;
pub mod pipeline;
mod resolve;
pub mod run_audit;
mod store_delegates;
mod task_reservation_cleanup;
mod v2_host;

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
    resolve_bootstrap_data_root, resolve_global_root, resolve_initialize_data_root,
    try_resolve_initialized_data_root,
};
pub(crate) use store_delegates::TaskRecordUpdateParams;

#[derive(Clone)]
pub struct OrbitRuntime {
    context: OrbitContext,
    activity_executors: Arc<ActivityExecutorRegistry>,
    pub event_log: event_bus::EventLog,
    _temp_dir: Option<Arc<builder::TempDir>>,
}

impl OrbitRuntime {
    pub fn initialize() -> Result<Self, OrbitError> {
        Self::initialize_with_root_override(None)
    }

    pub fn initialize_with_root_override(root_override: Option<&Path>) -> Result<Self, OrbitError> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let (global_root, workspace_root) = Self::resolve_roots_for_cwd(&cwd, root_override)?;
        ensure_orbit_root_initialized(&global_root, &workspace_root)?;
        Self::from_roots(&global_root, &workspace_root)
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
        let Some(workspace_root) = try_resolve_initialized_data_root(&cwd, root_override)? else {
            return Ok(None);
        };
        let global_root = resolve_global_root()?;
        Ok(Some(Self::from_roots(&global_root, &workspace_root)?))
    }

    pub fn resolve_roots_for_cwd(
        cwd: &Path,
        root_override: Option<&Path>,
    ) -> Result<(PathBuf, PathBuf), OrbitError> {
        let workspace_root = resolve_initialize_data_root(cwd, root_override)?;
        Ok((resolve_global_root()?, workspace_root))
    }

    pub fn resolve_bootstrap_roots_for_cwd(
        cwd: &Path,
        root_override: Option<&Path>,
    ) -> Result<(PathBuf, PathBuf), OrbitError> {
        let workspace_root = resolve_bootstrap_data_root(cwd, root_override)?;
        Self::bootstrap_roots_from_workspace_root(workspace_root, root_override)
    }

    fn bootstrap_roots_from_workspace_root(
        workspace_root: PathBuf,
        root_override: Option<&Path>,
    ) -> Result<(PathBuf, PathBuf), OrbitError> {
        let global_root = if has_explicit_root_override(root_override) {
            workspace_root.clone()
        } else {
            resolve_global_root()?
        };
        Ok((global_root, workspace_root))
    }

    pub fn from_data_root(data_root: &Path) -> Result<Self, OrbitError> {
        let context = builder::build_context_from_data_root(data_root)?;
        Ok(Self {
            activity_executors: build_activity_executor_registry(&context)?,
            context,
            event_log: event_bus::EventLog::default(),
            _temp_dir: None,
        })
    }

    pub fn from_roots(global_root: &Path, workspace_root: &Path) -> Result<Self, OrbitError> {
        let context = builder::build_context_from_roots(global_root, workspace_root)?;
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

    pub fn get_job(&self, _job_id: &str) -> Result<Option<orbit_common::types::Job>, OrbitError> {
        Ok(None)
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

    pub fn data_root(&self) -> PathBuf {
        self.context.data_root().to_path_buf()
    }

    pub fn global_root(&self) -> PathBuf {
        self.context.global_root().to_path_buf()
    }

    /// Returns the effective config.toml path.
    /// Workspace config replaces global if present; otherwise global.
    pub fn config_path(&self) -> PathBuf {
        let ws_config = self.data_root().join("config.toml");
        if ws_config.exists() && self.data_root() != self.global_root() {
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

    /// Configured default for the v2 `agent_loop` execution backend (§3.1
    /// precedence step 3). Returns `None` when not set.
    pub fn v2_backend_config(&self) -> Option<&str> {
        self.context.v2_backend()
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
        self.context.data_root()
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::ffi::OsString;
    use std::sync::Mutex;

    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf, PathBuf) {
        let root = tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("create global root");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
        (root, runtime, global_root, workspace_root)
    }

    #[test]
    fn orbit_root_env_selects_workspace_but_not_global_root() {
        let _guard = ENV_LOCK.lock().expect("lock env");
        let home = tempdir().expect("home tempdir");
        let repo = tempdir().expect("repo tempdir");
        let workspace_root = repo.path().join(".orbit");
        seed_initialized_workspace_root(&workspace_root);
        let _home = EnvVarGuard::set("HOME", home.path().as_os_str().to_os_string());
        let _orbit_root = EnvVarGuard::set("ORBIT_ROOT", workspace_root.as_os_str().to_os_string());

        let (global_root, resolved_workspace_root) =
            OrbitRuntime::resolve_roots_for_cwd(repo.path(), None).expect("resolve roots");

        assert_eq!(global_root, home.path().join(".orbit"));
        assert_eq!(resolved_workspace_root, workspace_root);
    }

    fn seed_initialized_workspace_root(path: &Path) {
        std::fs::create_dir_all(path.join("resources")).expect("create resources dir");
        std::fs::create_dir_all(path.join("tasks")).expect("create tasks dir");
        std::fs::create_dir_all(path.join("state")).expect("create state dir");
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: OsString) -> Self {
            let previous = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => unsafe {
                    std::env::set_var(self.key, value);
                },
                None => unsafe {
                    std::env::remove_var(self.key);
                },
            }
        }
    }

    fn write_activity(path: &Path, name: &str, description: &str) {
        let yaml = format!(
            r#"schemaVersion: 2
kind: Activity
metadata:
  name: {name}
spec:
  type: deterministic
  description: {description}
  action: test_action
  config: {{}}
"#
        );
        std::fs::create_dir_all(path.parent().expect("activity path has parent"))
            .expect("create activity dir");
        std::fs::write(path, yaml).expect("write activity yaml");
    }

    #[test]
    fn workspace_activity_overrides_global_default_in_catalog() {
        let (_root, runtime, global_root, workspace_root) = test_runtime();
        write_activity(
            &global_root.join("resources/activities/pr_open.yaml"),
            "pr_open",
            "global description",
        );
        write_activity(
            &workspace_root.join("resources/activities/pr_open.yaml"),
            "pr_open",
            "workspace description",
        );

        let catalog = runtime.v2_activity_catalog().expect("activity catalog");
        let activity = catalog.get("pr_open").expect("pr_open activity");
        assert_eq!(activity.description, "workspace description");
    }

    #[test]
    fn duplicate_activities_within_one_catalog_directory_remain_invalid() {
        let (_root, runtime, _global_root, workspace_root) = test_runtime();
        let activities_dir = workspace_root.join("resources/activities");
        write_activity(
            &activities_dir.join("first.yaml"),
            "duplicate_activity",
            "first description",
        );
        write_activity(
            &activities_dir.join("nested/second.yaml"),
            "duplicate_activity",
            "second description",
        );

        let err = runtime
            .v2_activity_catalog()
            .expect_err("duplicate activity name should fail");
        assert!(err.to_string().contains("duplicate activity name"), "{err}");
    }
}
