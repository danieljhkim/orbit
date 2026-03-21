use chrono::{DateTime, Utc};
use orbit_types::{
    Activity, AuditEvent, Job, JobRun, JobRunState, JobScheduleState, JobStep, OrbitError,
    StoredTool, Task, TaskComment, TaskComplexity, TaskHistoryEntry, TaskPriority, TaskStatus,
    TaskType,
};
use serde_json::Value;

use crate::sqlite::audit_event_store::{AuditEventFilter, AuditEventInsertParams};

#[derive(Debug, Clone)]
pub struct TaskCreateParams {
    pub actor: String,
    pub title: String,
    pub description: String,
    pub plan: String,
    pub execution_summary: String,
    pub context_files: Vec<String>,
    pub workspace_path: Option<String>,
    pub created_by: Option<String>,
    pub assigned_to: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub complexity: Option<TaskComplexity>,
    pub task_type: TaskType,
    pub pr_number: Option<String>,
    pub proposed_by: Option<String>,
    pub comments: Vec<TaskComment>,
}

#[derive(Debug, Default, Clone)]
pub struct TaskUpdateParams {
    pub actor: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub plan: Option<String>,
    pub execution_summary: Option<String>,
    pub context_files: Option<Vec<String>>,
    pub workspace_path: Option<Option<String>>,
    pub repo_root: Option<Option<String>>,
    pub assigned_to: Option<Option<String>>,
    pub created_by: Option<Option<String>>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub complexity: Option<TaskComplexity>,
    pub task_type: Option<TaskType>,
    pub pr_number: Option<Option<String>>,
    pub proposed_by: Option<Option<String>>,
    pub status_event: Option<String>,
    pub status_note: Option<String>,
    pub append_history: Vec<TaskHistoryEntry>,
    pub append_comments: Vec<TaskComment>,
}

#[derive(Debug, Clone)]
pub struct ActivityCreateParams {
    pub id: String,
    pub spec_type: String,
    pub description: String,
    pub input_schema_json: Value,
    pub output_schema_json: Value,
    pub spec_config: Value,
    pub workspace_path: Option<String>,
    pub created_by: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct ActivityUpdateParams {
    pub description: Option<String>,
    pub input_schema_json: Option<Value>,
    pub output_schema_json: Option<Value>,
    pub spec_config: Option<Value>,
    pub workspace_path: Option<Option<String>>,
    pub created_by: Option<Option<String>>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct JobCreateParams {
    pub job_id: Option<String>,
    pub default_input: Option<Value>,
    pub max_active_runs: u32,
    pub steps: Vec<JobStep>,
    pub initial_state: JobScheduleState,
}

#[derive(Debug, Default, Clone)]
pub struct JobUpdateParams {
    pub default_input: Option<Option<Value>>,
    pub max_active_runs: Option<u32>,
    pub steps: Option<Vec<JobStep>>,
    pub state: Option<JobScheduleState>,
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
    fn update_job(&self, job_id: &str, params: JobUpdateParams) -> Result<Job, OrbitError>;
    fn list_jobs(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError>;
    fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError>;
    fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError>;
    fn list_job_runs_filtered(&self, query: &JobRunQuery) -> Result<Vec<JobRun>, OrbitError>;
    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError>;
    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError>;
    fn set_job_state(&self, job_id: &str, state: JobScheduleState) -> Result<bool, OrbitError>;
    fn mark_job_disabled(&self, job_id: &str) -> Result<bool, OrbitError>;
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
        pid: u32,
    ) -> Result<bool, OrbitError>;
    fn abandon_job_run(&self, run_id: &str, finished_at: DateTime<Utc>)
    -> Result<bool, OrbitError>;
    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError>;
    fn finalize_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError>;
    fn archive_job_run(&self, run_id: &str) -> Result<String, OrbitError>;
    fn delete_job_run(&self, run_id: &str) -> Result<String, OrbitError>;
}

#[derive(Debug, Clone)]
pub struct JobRunStepParams {
    pub step_index: usize,
    pub target_type: orbit_types::JobTargetType,
    pub target_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub duration_ms: Option<u64>,
    pub exit_code: Option<i32>,
    pub agent_response_json: Option<Value>,
    pub state: JobRunState,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
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
