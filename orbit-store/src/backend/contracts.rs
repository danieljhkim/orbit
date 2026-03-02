use chrono::{DateTime, Utc};
use orbit_types::{
    AgentSession, AgentSessionStatus, AgentToolCall, Audit, AuditEvent, Job, OrbitError,
    OrbitEvent, Scheduler, SchedulerRetryBackoffStrategy, SchedulerRun, SchedulerRunState,
    SchedulerScheduleState, SchedulerTargetType, StoredTool, Task, TaskPriority, TaskStatus,
    TaskType, Watch,
};
use serde_json::Value;

use crate::sqlite::audit_event_store::{AuditEventFilter, AuditEventInsertParams};
use crate::sqlite::scheduler_store::DueJobsClaim;

#[derive(Debug, Clone)]
pub struct TaskCreateParams {
    pub title: String,
    pub description: String,
    pub instructions: String,
    pub context_files: Vec<String>,
    pub workspace_path: Option<String>,
    pub identity_id: Option<String>,
    pub assigned_to: Option<String>,
    pub created_by: Option<String>,
    pub approved_at: Option<DateTime<Utc>>,
    pub approved_by: Option<String>,
    pub approval_note: Option<String>,
    pub priority: TaskPriority,
    pub task_type: TaskType,
    pub owner: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct TaskUpdateParams {
    pub title: Option<String>,
    pub description: Option<String>,
    pub instructions: Option<String>,
    pub context_files: Option<Vec<String>>,
    pub workspace_path: Option<Option<String>>,
    pub identity_id: Option<Option<String>>,
    pub assigned_to: Option<Option<String>>,
    pub created_by: Option<Option<String>>,
    pub approved_at: Option<Option<DateTime<Utc>>>,
    pub approved_by: Option<Option<String>>,
    pub approval_note: Option<Option<String>>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub task_type: Option<TaskType>,
    pub owner: Option<String>,
    pub parent_id: Option<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct JobCreateParams {
    pub id: String,
    pub spec_type: String,
    pub description: String,
    pub input_schema_json: Value,
    pub output_schema_json: Value,
    pub artifact_path_template: Option<String>,
    pub skill_refs: Vec<String>,
    pub identity_id: Option<String>,
    pub assigned_to: Option<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SchedulerCreateParams {
    pub target_type: SchedulerTargetType,
    pub target_id: String,
    pub schedule: String,
    pub agent_cli: String,
    pub timeout_seconds: u64,
    pub retry_max_attempts: u32,
    pub retry_backoff_strategy: SchedulerRetryBackoffStrategy,
    pub retry_initial_delay_seconds: u64,
    pub next_run_at: DateTime<Utc>,
}

pub trait TaskStoreBackend: Send + Sync {
    fn create_task(&self, params: TaskCreateParams) -> Result<Task, OrbitError>;
    fn list_tasks(&self) -> Result<Vec<Task>, OrbitError>;
    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
    ) -> Result<Vec<Task>, OrbitError>;
    fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError>;
    fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError>;
    fn update_task(&self, id: &str, params: TaskUpdateParams) -> Result<Task, OrbitError>;
    fn delete_task(&self, id: &str) -> Result<bool, OrbitError>;
}

pub trait JobStoreBackend: Send + Sync {
    fn add_job(&self, params: JobCreateParams) -> Result<Job, OrbitError>;
    fn list_jobs(&self, include_inactive: bool) -> Result<Vec<Job>, OrbitError>;
    fn get_job(&self, id: &str) -> Result<Option<Job>, OrbitError>;
    fn disable_job(&self, id: &str) -> Result<bool, OrbitError>;
}

pub trait SchedulerStoreBackend: Send + Sync {
    fn add_scheduler(&self, params: SchedulerCreateParams) -> Result<Scheduler, OrbitError>;
    fn list_schedulers(&self, include_disabled: bool) -> Result<Vec<Scheduler>, OrbitError>;
    fn get_scheduler(&self, scheduler_id: &str) -> Result<Option<Scheduler>, OrbitError>;
    fn due_schedulers(&self, now: DateTime<Utc>) -> Result<Vec<Scheduler>, OrbitError>;
    fn list_scheduler_runs(&self, scheduler_id: &str) -> Result<Vec<SchedulerRun>, OrbitError>;
    fn get_pending_or_running_scheduler_run(
        &self,
        scheduler_id: &str,
    ) -> Result<Option<SchedulerRun>, OrbitError>;
    fn set_scheduler_state(
        &self,
        scheduler_id: &str,
        state: SchedulerScheduleState,
    ) -> Result<bool, OrbitError>;
    fn mark_scheduler_disabled(&self, scheduler_id: &str) -> Result<bool, OrbitError>;
    fn update_scheduler_next_run(
        &self,
        scheduler_id: &str,
        next_run_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError>;
    fn insert_scheduler_run(
        &self,
        scheduler_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<SchedulerRun, OrbitError>;
    fn mark_scheduler_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError>;
    #[allow(clippy::too_many_arguments)]
    fn complete_scheduler_run(
        &self,
        run_id: &str,
        state: SchedulerRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
        exit_code: Option<i32>,
        agent_response_json: Option<&Value>,
        error_code: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<bool, OrbitError>;
    fn claim_due_schedulers(&self, now: DateTime<Utc>) -> Result<DueJobsClaim, OrbitError>;
}

pub trait ToolStoreBackend: Send + Sync {
    fn list_tools(&self) -> Result<Vec<StoredTool>, OrbitError>;
    fn get_tool(&self, name: &str) -> Result<Option<StoredTool>, OrbitError>;
    fn insert_tool(&self, tool: &StoredTool) -> Result<(), OrbitError>;
    fn delete_tool(&self, name: &str) -> Result<bool, OrbitError>;
    fn set_tool_enabled(&self, name: &str, enabled: bool) -> Result<bool, OrbitError>;
}

pub trait WatchStoreBackend: Send + Sync {
    fn list_watches(&self) -> Result<Vec<Watch>, OrbitError>;
    fn get_watch(&self, id: &str) -> Result<Option<Watch>, OrbitError>;
    fn insert_watch(
        &self,
        path: &str,
        command: &str,
        debounce_ms: u64,
    ) -> Result<Watch, OrbitError>;
}

pub trait AuditStoreBackend: Send + Sync {
    fn list_audits(&self, limit: usize) -> Result<Vec<Audit>, OrbitError>;
    fn insert_audit_event(&self, event: &OrbitEvent) -> Result<(), OrbitError>;
}

pub trait AuditEventStoreBackend: Send + Sync {
    fn insert_audit_event_record(&self, params: &AuditEventInsertParams) -> Result<(), OrbitError>;
    fn list_audit_events(&self, filter: &AuditEventFilter) -> Result<Vec<AuditEvent>, OrbitError>;
    fn get_audit_event(&self, id: i64) -> Result<Option<AuditEvent>, OrbitError>;
    fn get_audit_event_stats(
        &self,
        since: Option<&DateTime<Utc>>,
        tool: Option<&str>,
    ) -> Result<(i64, i64, i64, i64, f64, i64), OrbitError>;
    fn get_audit_event_durations(
        &self,
        since: Option<&DateTime<Utc>>,
        tool: Option<&str>,
    ) -> Result<Vec<i64>, OrbitError>;
    fn prune_audit_events(&self, older_than: &DateTime<Utc>) -> Result<usize, OrbitError>;
}

pub trait AgentSessionStoreBackend: Send + Sync {
    fn get_agent_session(&self, session_id: &str) -> Result<Option<AgentSession>, OrbitError>;
    fn insert_agent_session(&self, session: &AgentSession) -> Result<(), OrbitError>;
    fn update_agent_session(
        &self,
        session_id: &str,
        tool_calls: &[AgentToolCall],
        outcome: &str,
        status: AgentSessionStatus,
    ) -> Result<bool, OrbitError>;
}

pub trait LockStoreBackend: Send + Sync {
    fn try_lock(&self, name: &str) -> Result<bool, OrbitError>;
    fn unlock(&self, name: &str) -> Result<bool, OrbitError>;
    fn global_job_lock_name(&self) -> &'static str;
}
