pub mod audit;
pub mod builder;
mod engine;
pub mod event_bus;
pub mod mutation;
pub mod pipeline;

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_policy::PolicyEngine;
use orbit_store::{
    ActivityCreateParams, ActivityUpdateParams, AuditEventFilter, AuditEventInsertParams,
    JobCreateParams, JobRunQuery, JobRunStepParams, JobUpdateParams, TaskCreateParams,
    TaskUpdateParams as StoreTaskUpdateParams,
};
use orbit_types::{
    Activity, Audit, AuditEvent, Job, JobRun, JobRunState, OrbitError, OrbitEvent, StoredTool,
    Task, TaskPriority, TaskStatus,
};
use serde::Deserialize;
use serde_json::Value;

use crate::OrbitContext;
use crate::command::init::ensure_orbit_root_initialized;
use crate::context::ActorIdentity;
use crate::paths;

#[derive(Clone)]
pub struct OrbitRuntime {
    context: OrbitContext,
    pub event_log: event_bus::EventLog,
}

impl OrbitRuntime {
    pub fn initialize() -> Result<Self, OrbitError> {
        Self::initialize_with_root_override(None)
    }

    pub fn initialize_with_root_override(root_override: Option<&Path>) -> Result<Self, OrbitError> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let data_root = resolve_initialize_data_root(&cwd, root_override)?;
        ensure_orbit_root_initialized(&data_root)?;
        Self::from_data_root(&data_root)
    }

    pub fn from_data_root(data_root: &Path) -> Result<Self, OrbitError> {
        Ok(Self {
            context: builder::build_context_from_data_root(data_root)?,
            event_log: event_bus::EventLog::default(),
        })
    }

    pub fn in_memory() -> Result<Self, OrbitError> {
        Ok(Self {
            context: builder::build_context_in_memory()?,
            event_log: event_bus::EventLog::default(),
        })
    }

    pub fn with_policy(mut self, policy: PolicyEngine) -> Self {
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

    pub fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.get_job_record(job_id)
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

    /// Returns the runtime config file at `<data_root>/config.toml`
    /// which is typically `.orbit/config.toml` in a repo-local workspace.
    pub fn config_path(&self) -> PathBuf {
        self.data_root().join("config.toml")
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

    pub fn user_name(&self) -> &str {
        self.context.user_name()
    }

    pub(crate) fn actor(&self) -> &ActorIdentity {
        self.context.actor()
    }

    pub(crate) fn actor_label(&self) -> &str {
        self.context.actor().label.as_str()
    }

    pub(crate) fn policy_engine(&self) -> &PolicyEngine {
        self.context.policy()
    }

    pub(crate) fn tool_registry(&self) -> &orbit_tools::ToolRegistry {
        self.context.registry()
    }

    pub(crate) fn skill_catalog(&self) -> &crate::skill_catalog::SkillCatalog {
        self.context.skill_catalog()
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

    pub(crate) fn create_task_record(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        self.context.task_store().create_task(params)
    }

    pub(crate) fn get_task_record(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        self.context.task_store().get_task(id)
    }

    pub(crate) fn list_task_records(&self) -> Result<Vec<Task>, OrbitError> {
        self.context.task_store().list_tasks()
    }

    pub(crate) fn list_task_records_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.context
            .task_store()
            .list_tasks_filtered(status, priority)
    }

    pub(crate) fn update_task_record(
        &self,
        id: &str,
        params: StoreTaskUpdateParams,
    ) -> Result<Task, OrbitError> {
        self.context.task_store().update_task(id, params)
    }

    pub(crate) fn delete_task_record(&self, id: &str) -> Result<bool, OrbitError> {
        self.context.task_store().delete_task(id)
    }

    pub(crate) fn search_task_records(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.context.task_store().search_tasks(query)
    }

    pub(crate) fn add_activity_record(
        &self,
        params: ActivityCreateParams,
    ) -> Result<Activity, OrbitError> {
        self.context.activity_store().add_activity(params)
    }

    pub(crate) fn list_activity_records(
        &self,
        include_inactive: bool,
    ) -> Result<Vec<Activity>, OrbitError> {
        self.context
            .activity_store()
            .list_activities(include_inactive)
    }

    pub(crate) fn get_activity_record(&self, id: &str) -> Result<Option<Activity>, OrbitError> {
        self.context.activity_store().get_activity(id)
    }

    pub(crate) fn update_activity_record(
        &self,
        id: &str,
        params: ActivityUpdateParams,
    ) -> Result<Activity, OrbitError> {
        self.context.activity_store().update_activity(id, params)
    }

    pub(crate) fn disable_activity_record(&self, id: &str) -> Result<bool, OrbitError> {
        self.context.activity_store().disable_activity(id)
    }

    pub(crate) fn add_job_record(&self, params: JobCreateParams) -> Result<Job, OrbitError> {
        self.context.job_store().add_job(params)
    }

    pub(crate) fn update_job_record(
        &self,
        job_id: &str,
        params: JobUpdateParams,
    ) -> Result<Job, OrbitError> {
        self.context.job_store().update_job(job_id, params)
    }

    pub(crate) fn mark_job_disabled_record(&self, job_id: &str) -> Result<bool, OrbitError> {
        self.context.job_store().mark_job_disabled(job_id)
    }

    pub(crate) fn list_job_records(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        self.context.job_store().list_jobs(include_disabled)
    }

    pub(crate) fn get_job_record(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        self.context.job_store().get_job(job_id)
    }

    pub(crate) fn list_job_runs_filtered_record(
        &self,
        query: &JobRunQuery,
    ) -> Result<Vec<JobRun>, OrbitError> {
        self.context.job_store().list_job_runs_filtered(query)
    }

    pub(crate) fn list_pending_or_running_job_runs_record(
        &self,
        job_id: &str,
    ) -> Result<Vec<JobRun>, OrbitError> {
        self.context
            .job_store()
            .list_pending_or_running_job_runs(job_id)
    }

    pub(crate) fn insert_job_run_record(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<JobRun, OrbitError> {
        self.context
            .job_store()
            .insert_job_run(job_id, attempt, scheduled_at)
    }

    pub(crate) fn mark_job_run_running_record(
        &self,
        run_id: &str,
        started_at: chrono::DateTime<chrono::Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.context
            .job_store()
            .mark_job_run_running(run_id, started_at, pid)
    }

    pub(crate) fn abandon_job_run_record(
        &self,
        run_id: &str,
        finished_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, OrbitError> {
        self.context
            .job_store()
            .abandon_job_run(run_id, finished_at)
    }

    pub(crate) fn complete_job_run_step_record(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.context
            .job_store()
            .complete_job_run_step(run_id, params)
    }

    pub(crate) fn finalize_job_run_record(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: chrono::DateTime<chrono::Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.context
            .job_store()
            .finalize_job_run(run_id, state, finished_at, duration_ms)
    }

    pub(crate) fn get_job_run_record(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.context.job_store().get_job_run(run_id)
    }

    pub(crate) fn list_job_run_records(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.context.job_store().list_job_runs(job_id)
    }

    pub(crate) fn archive_job_run_record(&self, run_id: &str) -> Result<String, OrbitError> {
        self.context.job_store().archive_job_run(run_id)
    }

    pub(crate) fn delete_job_run_record(&self, run_id: &str) -> Result<String, OrbitError> {
        self.context.job_store().delete_job_run(run_id)
    }

    pub(crate) fn list_tool_records(&self) -> Result<Vec<StoredTool>, OrbitError> {
        self.context.tool_store().list_tools()
    }

    pub(crate) fn get_tool_record(&self, name: &str) -> Result<Option<StoredTool>, OrbitError> {
        self.context.tool_store().get_tool(name)
    }

    pub(crate) fn insert_tool_record(&self, tool: &StoredTool) -> Result<(), OrbitError> {
        self.context.tool_store().insert_tool(tool)
    }

    pub(crate) fn delete_tool_record(&self, name: &str) -> Result<bool, OrbitError> {
        self.context.tool_store().delete_tool(name)
    }

    pub(crate) fn set_tool_enabled_record(
        &self,
        name: &str,
        enabled: bool,
    ) -> Result<bool, OrbitError> {
        self.context.tool_store().set_tool_enabled(name, enabled)
    }

    pub(crate) fn list_audit_event_records(
        &self,
        filter: &AuditEventFilter,
    ) -> Result<Vec<AuditEvent>, OrbitError> {
        self.context.audit_event_store().list_audit_events(filter)
    }

    pub(crate) fn get_audit_event_record(&self, id: i64) -> Result<Option<AuditEvent>, OrbitError> {
        self.context.audit_event_store().get_audit_event(id)
    }

    pub(crate) fn prune_audit_event_records(
        &self,
        older_than: &chrono::DateTime<chrono::Utc>,
    ) -> Result<usize, OrbitError> {
        self.context
            .audit_event_store()
            .prune_audit_events(older_than)
    }

    pub(crate) fn audit_event_stats_record(
        &self,
        since: Option<&chrono::DateTime<chrono::Utc>>,
        tool: Option<&str>,
    ) -> Result<(i64, i64, i64, i64, f64, i64), OrbitError> {
        self.context
            .audit_event_store()
            .get_audit_event_stats(since, tool)
    }

    pub(crate) fn audit_event_durations_record(
        &self,
        since: Option<&chrono::DateTime<chrono::Utc>>,
        tool: Option<&str>,
    ) -> Result<Vec<i64>, OrbitError> {
        self.context
            .audit_event_store()
            .get_audit_event_durations(since, tool)
    }

    pub(crate) fn insert_audit_event_record(
        &self,
        params: &AuditEventInsertParams,
    ) -> Result<(), OrbitError> {
        self.context
            .audit_event_store()
            .insert_audit_event_record(params)
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

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RootField {
    String(String),
    Table { path: String },
}

#[derive(Debug, Deserialize)]
struct RootOnlyConfig {
    root: Option<RootField>,
}

pub(crate) fn resolve_initialize_data_root(
    cwd: &Path,
    root_override: Option<&Path>,
) -> Result<PathBuf, OrbitError> {
    if let Some(root) = root_override {
        return resolve_root_path_value(&root.to_string_lossy(), cwd);
    }

    if let Ok(explicit) = std::env::var("ORBIT_ROOT")
        && !explicit.trim().is_empty()
    {
        return resolve_root_path_value(&explicit, cwd);
    }

    if let Some(repo_root) = find_git_repo_root(cwd) {
        let repo_orbit_root = repo_root.join(".orbit");
        let repo_config = repo_orbit_root.join("config.toml");
        if repo_config.exists()
            && let Some(configured_root) = configured_root_from_config(&repo_config)?
        {
            return Ok(configured_root);
        }
        return Ok(repo_orbit_root);
    }

    Ok(paths::cwd_orbit_root(cwd))
}

fn configured_root_from_config(config_path: &Path) -> Result<Option<PathBuf>, OrbitError> {
    let raw = fs::read_to_string(config_path).map_err(|err| {
        OrbitError::Io(format!(
            "failed to read runtime config '{}': {err}",
            config_path.display()
        ))
    })?;
    let parsed = toml::from_str::<RootOnlyConfig>(&raw).map_err(|err| {
        OrbitError::InvalidInput(format!(
            "invalid runtime config '{}': {err}",
            config_path.display()
        ))
    })?;
    let Some(root_value) = parsed.root else {
        return Ok(None);
    };
    let root_value = match root_value {
        RootField::String(value) => value,
        RootField::Table { path } => path,
    };
    let base = config_path.parent().ok_or_else(|| {
        OrbitError::InvalidInput(format!(
            "invalid config path without parent: {}",
            config_path.display()
        ))
    })?;
    Ok(Some(resolve_root_path_value(&root_value, base)?))
}

fn resolve_root_path_value(raw: &str, base_dir: &Path) -> Result<PathBuf, OrbitError> {
    paths::resolve_path_value(raw, base_dir, "root path")
}

fn find_git_repo_root(start: &Path) -> Option<PathBuf> {
    paths::find_git_repo_root(start)
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::resolve_initialize_data_root;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn cli_root_override_has_highest_precedence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        let explicit = dir.path().join("cli-root");
        let chosen = resolve_initialize_data_root(cwd, Some(explicit.as_path())).expect("resolve");
        assert_eq!(chosen, explicit);
    }

    #[test]
    fn orbit_root_env_overrides_config_roots() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path();
        let explicit = dir.path().join("env-root");

        let previous = std::env::var("ORBIT_ROOT").ok();
        // SAFETY: test runs in isolation; no other thread reads this var concurrently.
        unsafe { std::env::set_var("ORBIT_ROOT", &explicit) };
        let chosen = resolve_initialize_data_root(cwd, None).expect("resolve");
        match previous {
            Some(value) => unsafe { std::env::set_var("ORBIT_ROOT", value) },
            None => unsafe { std::env::remove_var("ORBIT_ROOT") },
        }

        assert_eq!(chosen, explicit);
    }

    #[test]
    fn repo_config_root_has_precedence_over_home_config() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let cwd = repo.join("nested");
        std::fs::create_dir_all(repo.join(".git")).expect("create git dir");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        let repo_orbit = repo.join(".orbit");
        std::fs::create_dir_all(&repo_orbit).expect("repo orbit");
        std::fs::write(
            repo_orbit.join("config.toml"),
            "root = \"./repo-root\"\n[task.approval]\nrequired_for_agent=true\n",
        )
        .expect("write repo config");

        let previous = std::env::var("ORBIT_ROOT").ok();
        match previous {
            Some(_) => unsafe { std::env::remove_var("ORBIT_ROOT") },
            None => {}
        }
        let chosen = resolve_initialize_data_root(&cwd, None).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, repo_orbit.join("repo-root"));
    }

    #[test]
    fn repo_root_used_when_inside_git_repo_without_repo_config() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let cwd = repo.join("nested");
        std::fs::create_dir_all(repo.join(".git")).expect("create git dir");
        std::fs::create_dir_all(&cwd).expect("create cwd");

        let previous = std::env::var("ORBIT_ROOT").ok();
        match previous {
            Some(_) => unsafe { std::env::remove_var("ORBIT_ROOT") },
            None => {}
        }
        let chosen = resolve_initialize_data_root(&cwd, None).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, repo.join(".orbit"));
    }

    #[test]
    fn cwd_root_used_when_outside_git_repo_without_override_or_config() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path().join("workspace");
        std::fs::create_dir_all(&cwd).expect("create cwd");

        let previous = std::env::var("ORBIT_ROOT").ok();
        match previous {
            Some(_) => unsafe { std::env::remove_var("ORBIT_ROOT") },
            None => {}
        }
        let chosen = resolve_initialize_data_root(&cwd, None).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, cwd.join(".orbit"));
    }

    #[test]
    fn repo_root_used_even_when_repo_orbit_directory_is_absent() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let cwd = repo.join("nested");
        std::fs::create_dir_all(repo.join(".git")).expect("create git dir");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        let previous = std::env::var("ORBIT_ROOT").ok();
        match previous {
            Some(_) => unsafe { std::env::remove_var("ORBIT_ROOT") },
            None => {}
        }
        let chosen = resolve_initialize_data_root(&cwd, None).expect("resolve");
        if let Some(value) = previous {
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, repo.join(".orbit"));
    }

    #[test]
    fn configured_root_normalizes_curdir_segments() {
        let _guard = env_lock().lock().expect("env lock");
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        let cwd = repo.join("nested");
        let repo_orbit = repo.join(".orbit");
        std::fs::create_dir_all(repo.join(".git")).expect("create git dir");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        std::fs::create_dir_all(&repo_orbit).expect("repo orbit");
        std::fs::write(repo_orbit.join("config.toml"), "root = \".\"\n").expect("write config");

        let previous = std::env::var("ORBIT_ROOT").ok();
        if previous.is_some() {
            // SAFETY: test runs in isolation; no other thread reads this var concurrently.
            unsafe { std::env::remove_var("ORBIT_ROOT") };
        }
        let chosen = resolve_initialize_data_root(&cwd, None).expect("resolve");
        if let Some(value) = previous {
            // SAFETY: test restores previous env var for isolation.
            unsafe { std::env::set_var("ORBIT_ROOT", value) };
        }
        assert_eq!(chosen, repo_orbit);
    }
}
