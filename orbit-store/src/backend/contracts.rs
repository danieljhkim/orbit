use chrono::{DateTime, Utc};
use orbit_types::{
    Activity, AgentSession, AgentSessionStatus, AgentToolCall, AuditEvent, Job,
    JobRetryBackoffStrategy, JobRun, JobRunState, JobScheduleState, JobTargetType, OrbitError,
    StoredTool, Task, TaskComment, TaskPriority, TaskStatus, TaskType,
};
use serde_json::Value;

use crate::sqlite::audit_event_store::{AuditEventFilter, AuditEventInsertParams};

#[derive(Debug, Clone)]
pub struct TaskCreateParams {
    pub title: String,
    pub description: String,
    pub plan: String,
    pub execution_summary: String,
    pub context_files: Vec<String>,
    pub workspace_path: Option<String>,
    pub assigned_to: Option<String>,
    pub created_by: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub task_type: TaskType,
    pub branch: Option<String>,
    pub pr_number: Option<String>,
    pub proposed_by: Option<String>,
    pub comments: Vec<TaskComment>,
}

#[derive(Debug, Default, Clone)]
pub struct TaskUpdateParams {
    pub title: Option<String>,
    pub description: Option<String>,
    pub plan: Option<String>,
    pub execution_summary: Option<String>,
    pub context_files: Option<Vec<String>>,
    pub workspace_path: Option<Option<String>>,
    pub assigned_to: Option<Option<String>>,
    pub created_by: Option<Option<String>>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub task_type: Option<TaskType>,
    pub branch: Option<Option<String>>,
    pub pr_number: Option<Option<String>>,
    pub proposed_by: Option<Option<String>>,
    pub proposal_approved_by: Option<Option<String>>,
    pub proposal_rejected_by: Option<Option<String>>,
    pub proposal_decision_note: Option<Option<String>>,
    pub review_approved_by: Option<Option<String>>,
    pub review_rejected_by: Option<Option<String>>,
    pub review_decision_note: Option<Option<String>>,
    pub append_comments: Vec<TaskComment>,
}

#[derive(Debug, Clone)]
pub struct ActivityCreateParams {
    pub id: String,
    pub spec_type: String,
    pub description: String,
    pub instruction: String,
    pub input_schema_json: Value,
    pub output_schema_json: Value,
    pub artifact_path_template: Option<String>,
    pub skill_refs: Vec<String>,
    pub identity_id: Option<String>,
    pub assigned_to: Option<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct ActivityUpdateParams {
    pub description: Option<String>,
    pub instruction: Option<String>,
    pub input_schema_json: Option<Value>,
    pub output_schema_json: Option<Value>,
    pub artifact_path_template: Option<Option<String>>,
    pub skill_refs: Option<Vec<String>>,
    pub identity_id: Option<Option<String>>,
    pub assigned_to: Option<Option<String>>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct JobCreateParams {
    pub job_id: Option<String>,
    pub target_type: JobTargetType,
    pub target_id: String,
    pub schedule: String,
    pub agent_cli: String,
    pub timeout_seconds: u64,
    pub retry_max_attempts: u32,
    pub retry_backoff_strategy: JobRetryBackoffStrategy,
    pub retry_initial_delay_seconds: u64,
    pub next_run_at: DateTime<Utc>,
    pub initial_state: JobScheduleState,
    pub env_extra: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ClaimedJobRun {
    pub job: Job,
    pub run: JobRun,
}

#[derive(Debug, Clone, Default)]
pub struct DueJobsClaim {
    pub claimed: Vec<ClaimedJobRun>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct JobRunQuery {
    pub job_id: Option<String>,
    pub state: Option<JobRunState>,
    pub created_since: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
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

pub trait ActivityStoreBackend: Send + Sync {
    fn add_activity(&self, params: ActivityCreateParams) -> Result<Activity, OrbitError>;
    fn list_activities(&self, include_inactive: bool) -> Result<Vec<Activity>, OrbitError>;
    fn get_activity(&self, id: &str) -> Result<Option<Activity>, OrbitError>;
    fn update_activity(
        &self,
        id: &str,
        params: ActivityUpdateParams,
    ) -> Result<Activity, OrbitError>;
    fn disable_activity(&self, id: &str) -> Result<bool, OrbitError>;
}

pub trait JobStoreBackend: Send + Sync {
    fn add_job(&self, params: JobCreateParams) -> Result<Job, OrbitError>;
    fn list_jobs(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError>;
    fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError>;
    fn due_jobs(&self, now: DateTime<Utc>) -> Result<Vec<Job>, OrbitError>;
    fn next_due_job_time(&self) -> Result<Option<DateTime<Utc>>, OrbitError>;
    fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError>;
    fn list_job_runs_filtered(&self, query: &JobRunQuery) -> Result<Vec<JobRun>, OrbitError>;
    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError>;
    fn get_pending_or_running_job_run(&self, job_id: &str) -> Result<Option<JobRun>, OrbitError>;
    fn set_job_state(&self, job_id: &str, state: JobScheduleState) -> Result<bool, OrbitError>;
    fn mark_job_disabled(&self, job_id: &str) -> Result<bool, OrbitError>;
    fn update_job_next_run(
        &self,
        job_id: &str,
        next_run_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError>;
    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<JobRun, OrbitError>;
    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError>;
    fn complete_job_run(&self, params: &JobRunCompletionParams) -> Result<bool, OrbitError>;
    fn claim_due_jobs(&self, now: DateTime<Utc>) -> Result<DueJobsClaim, OrbitError>;
    fn archive_job_run(&self, run_id: &str) -> Result<String, OrbitError>;
    fn delete_job_run(&self, run_id: &str) -> Result<String, OrbitError>;
}

#[derive(Debug, Clone)]
pub struct JobRunCompletionParams<'a> {
    pub run_id: &'a str,
    pub state: JobRunState,
    pub finished_at: DateTime<Utc>,
    pub duration_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub agent_response_json: Option<&'a Value>,
    pub error_code: Option<&'a str>,
    pub error_message: Option<&'a str>,
}

pub trait ToolStoreBackend: Send + Sync {
    fn list_tools(&self) -> Result<Vec<StoredTool>, OrbitError>;
    fn get_tool(&self, name: &str) -> Result<Option<StoredTool>, OrbitError>;
    fn insert_tool(&self, tool: &StoredTool) -> Result<(), OrbitError>;
    fn delete_tool(&self, name: &str) -> Result<bool, OrbitError>;
    fn set_tool_enabled(&self, name: &str, enabled: bool) -> Result<bool, OrbitError>;
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
