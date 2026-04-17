//! Runtime bootstrap and the two-root architecture (global + workspace).
//!
//! `OrbitRuntime` is initialized by locating two roots:
//! 1. **Global root** — `~/.orbit/` (or `ORBIT_ROOT`): houses global config,
//!    the audit SQLite database, and globally-scoped resources.
//! 2. **Workspace root** — the nearest ancestor `.orbit/` directory from cwd:
//!    houses workspace-local tasks, knowledge, skills, and runtime state.
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
mod store_delegates;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use orbit_engine::ActivityExecutorRegistry;
use orbit_types::{Audit, OrbitError, OrbitEvent, WorkspacePaths};
use serde_json::Value;

use crate::OrbitContext;
use crate::command::init::ensure_orbit_root_initialized;
use crate::context::ActorIdentity;
use crate::context::OrbitStores;

pub(crate) use orbit_tool_host::build_orbit_tool_host;
pub(crate) use resolve::{resolve_global_root, resolve_initialize_data_root};
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
        let workspace_root = resolve_initialize_data_root(&cwd, root_override)?;
        let global_root = resolve_global_root()?;
        ensure_orbit_root_initialized(&global_root, &workspace_root)?;
        Self::from_roots(&global_root, &workspace_root)
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

    pub fn get_job(&self, job_id: &str) -> Result<Option<orbit_types::Job>, OrbitError> {
        self.stores().jobs().get(job_id)
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

    pub fn list_executor_defs(&self) -> Result<Vec<orbit_types::ExecutorDef>, OrbitError> {
        self.stores().executors().list()
    }

    pub fn get_executor_def(
        &self,
        name: &str,
    ) -> Result<Option<orbit_types::ExecutorDef>, OrbitError> {
        self.stores().executors().get(name)
    }

    pub fn upsert_executor_def(&self, def: &orbit_types::ExecutorDef) -> Result<(), OrbitError> {
        self.stores().executors().upsert(def)
    }

    pub fn list_policy_defs(&self) -> Result<Vec<orbit_types::PolicyDef>, OrbitError> {
        self.stores().policies().list()
    }

    pub fn get_policy_def(&self, name: &str) -> Result<Option<orbit_types::PolicyDef>, OrbitError> {
        self.stores().policies().get(name)
    }

    pub fn upsert_policy_def(&self, def: &orbit_types::PolicyDef) -> Result<(), OrbitError> {
        self.stores().policies().upsert(def)
    }
}

fn build_activity_executor_registry(
    context: &OrbitContext,
) -> Result<Arc<ActivityExecutorRegistry>, OrbitError> {
    let mut registry = ActivityExecutorRegistry::with_builtins();
    let defs = context.stores().executors().list()?;
    registry.load_from_defs(&defs);
    Ok(Arc::new(registry))
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
