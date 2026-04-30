use chrono::{DateTime, Utc};
use orbit_common::types::{
    AuditEvent, ExecutorDef, JobRun, JobRunState, KnowledgeRunMetrics, OrbitError, OrbitId,
    PipelineState, PolicyDef, ReviewThread, StoredTool, Task, TaskArtifact, TaskComment,
    TaskComplexity, TaskHistoryEntry, TaskPriority, TaskStatus, TaskType,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::sqlite::audit_event_store::{
    AuditEventFilter, AuditEventInsertParams, AuditToolCallCountsByRole,
};

#[derive(Debug, Clone)]
pub struct TaskCreateParams {
    pub actor: String,
    pub parent_id: Option<OrbitId>,
    pub title: String,
    pub description: String,
    pub acceptance_criteria: Vec<String>,
    pub dependencies: Vec<OrbitId>,
    pub plan: String,
    pub execution_summary: String,
    pub context_files: Vec<String>,
    /// The working directory the agent should use when executing this task.
    /// Typically the root of the repository being modified. Used to set `cwd`
    /// for tool calls and to resolve relative `context_files` paths.
    pub workspace_path: Option<String>,
    /// The git repository root for this task, when it differs from
    /// `workspace_path`. Most tasks leave this `None` (the repo root is the
    /// same as the workspace). Set explicitly when the task targets a
    /// sub-directory of a monorepo and git operations must run from the root.
    pub repo_root: Option<String>,
    pub created_by: Option<String>,
    pub planned_by: Option<String>,
    pub implemented_by: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub complexity: Option<TaskComplexity>,
    pub task_type: TaskType,
    pub pr_number: Option<String>,
    pub source_task_id: Option<String>,
    pub comments: Vec<TaskComment>,
}

/// Parameters for a partial update to an existing task.
///
/// Fields that are `None` are left unchanged. Fields of type `Option<Option<T>>`
/// follow the "outer = whether to update, inner = new value" convention:
/// - `None`           → leave the field untouched
/// - `Some(Some(v))`  → set the field to `v`
/// - `Some(None)`     → explicitly clear the field (set it to null/absent)
#[derive(Debug, Default, Clone)]
pub struct TaskDocumentUpdateParams {
    pub actor: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub acceptance_criteria: Option<Vec<String>>,
    pub dependencies: Option<Vec<OrbitId>>,
    pub plan: Option<String>,
    pub execution_summary: Option<String>,
    pub context_files: Option<Vec<String>>,
    pub workspace_path: Option<Option<String>>,
    pub repo_root: Option<Option<String>>,
    pub created_by: Option<Option<String>>,
    pub planned_by: Option<Option<String>>,
    pub implemented_by: Option<Option<String>>,
    pub agent: Option<Option<String>>,
    pub model: Option<Option<String>>,
    pub priority: Option<TaskPriority>,
    pub complexity: Option<TaskComplexity>,
    pub task_type: Option<TaskType>,
    pub pr_number: Option<Option<String>>,
    pub pr_status: Option<Option<String>>,
    pub source_task_id: Option<Option<String>>,
    pub batch_id: Option<Option<String>>,
}

#[derive(Debug, Default, Clone)]
pub struct TaskHistoryUpdateParams {
    pub actor: String,
    pub status: Option<TaskStatus>,
    pub status_event: Option<String>,
    pub status_note: Option<String>,
    pub append_history: Vec<TaskHistoryEntry>,
    pub append_comments: Vec<TaskComment>,
}

#[derive(Debug, Default, Clone)]
pub struct TaskReviewUpdateParams {
    /// Review threads to append or merge. Threads whose `thread_id` matches
    /// an existing thread have their messages appended; new threads are added.
    pub append_review_threads: Vec<ReviewThread>,
    /// When set, replaces the entire review_threads collection (used by sync).
    pub replace_review_threads: Option<Vec<ReviewThread>>,
}

#[derive(Debug, Default, Clone)]
pub struct TaskArtifactUpdateParams {
    /// Artifact files to write under the task bundle `artifacts/` directory.
    /// Existing files at the same relative path are overwritten.
    pub upsert_artifacts: Vec<TaskArtifact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskLockHolder {
    Task,
    Reservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLockConflict {
    pub file: String,
    pub held_by: TaskLockHolder,
    pub held_by_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpiredTaskReservation {
    pub reservation_id: String,
    pub expired_at: String,
}

#[derive(Debug, Clone)]
pub struct TaskReservationCheckParams {
    pub workspace_orbit_dir: String,
    pub requested_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskReservationCheckResult {
    pub conflicts: Vec<TaskLockConflict>,
    pub expired_reservations: Vec<ExpiredTaskReservation>,
}

#[derive(Debug, Clone)]
pub struct TaskReservationReserveParams {
    pub workspace_orbit_dir: String,
    pub task_ids: Vec<String>,
    pub requested_files: Vec<String>,
    pub actor: String,
    pub ttl_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskReservationReserveResult {
    pub reserved: bool,
    pub reservation_id: Option<String>,
    pub expires_at: Option<String>,
    pub reserved_files: Vec<String>,
    pub conflicts: Vec<TaskLockConflict>,
    pub expired_reservations: Vec<ExpiredTaskReservation>,
}

#[derive(Debug, Clone)]
pub struct TaskReservationReleaseParams {
    pub workspace_orbit_dir: String,
    pub reservation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskReservationReleaseResult {
    pub released: bool,
    pub released_at: Option<String>,
    pub expired_reservations: Vec<ExpiredTaskReservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveTaskReservation {
    pub reservation_id: String,
    pub task_ids: Vec<String>,
    pub files: Vec<String>,
    pub actor: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskReservationListResult {
    pub reservations: Vec<ActiveTaskReservation>,
    pub expired_reservations: Vec<ExpiredTaskReservation>,
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
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError>;
    fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError>;
    fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError>;
    fn delete_task(&self, id: &str) -> Result<bool, OrbitError>;
}

pub trait TaskDocumentStoreBackend: Send + Sync {
    fn update_task_document(
        &self,
        id: &str,
        params: TaskDocumentUpdateParams,
    ) -> Result<(), OrbitError>;
}

pub trait TaskHistoryStoreBackend: Send + Sync {
    fn update_task_history(
        &self,
        id: &str,
        params: TaskHistoryUpdateParams,
    ) -> Result<(), OrbitError>;
}

pub trait TaskReviewStoreBackend: Send + Sync {
    fn update_task_reviews(
        &self,
        id: &str,
        params: TaskReviewUpdateParams,
    ) -> Result<(), OrbitError>;
}

pub trait TaskArtifactStoreBackend: Send + Sync {
    fn get_task_artifacts(&self, id: &str) -> Result<Option<Vec<TaskArtifact>>, OrbitError>;
    fn upsert_task_artifacts(
        &self,
        id: &str,
        params: TaskArtifactUpdateParams,
    ) -> Result<(), OrbitError>;
}

pub trait TaskReservationStoreBackend: Send + Sync {
    fn list_active_task_reservations(
        &self,
        workspace_orbit_dir: &str,
    ) -> Result<TaskReservationListResult, OrbitError>;

    fn check_task_reservation_conflicts(
        &self,
        params: TaskReservationCheckParams,
    ) -> Result<TaskReservationCheckResult, OrbitError>;

    fn reserve_task_reservation(
        &self,
        params: TaskReservationReserveParams,
    ) -> Result<TaskReservationReserveResult, OrbitError>;

    fn release_task_reservation(
        &self,
        params: TaskReservationReleaseParams,
    ) -> Result<TaskReservationReleaseResult, OrbitError>;
}

pub trait JobRunStoreBackend: Send + Sync {
    fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError>;
    fn list_job_runs_filtered(&self, query: &JobRunQuery) -> Result<Vec<JobRun>, OrbitError>;
    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError>;
    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError>;
    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
        input: Option<serde_json::Value>,
        retry_source_run_id: Option<String>,
    ) -> Result<JobRun, OrbitError>;
    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError>;
    fn take_over_running_job_run(
        &self,
        run_id: &str,
        expected_pid: Option<u32>,
        expected_pid_start_time: Option<String>,
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
    fn record_job_run_knowledge_metrics(
        &self,
        run_id: &str,
        metrics: KnowledgeRunMetrics,
    ) -> Result<bool, OrbitError>;
    fn finalize_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError>;
    fn repair_terminal_job_run_timing(
        &self,
        run_id: &str,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError>;
    fn list_all_pending_or_running_runs(&self) -> Result<Vec<JobRun>, OrbitError>;
    fn archive_job_run(&self, run_id: &str) -> Result<String, OrbitError>;
    fn delete_job_run(&self, run_id: &str) -> Result<String, OrbitError>;
    fn read_run_state(&self, run_id: &str) -> Result<Option<PipelineState>, OrbitError>;
    fn write_run_state(&self, run_id: &str, state: &PipelineState) -> Result<(), OrbitError>;
}

#[derive(Debug, Clone)]
pub struct JobRunStepParams {
    pub step_index: usize,
    pub target_type: orbit_common::types::JobTargetType,
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
    fn get_audit_event_hourly_buckets(
        &self,
        since: &DateTime<Utc>,
    ) -> Result<Vec<(String, i64)>, OrbitError>;
    fn get_audit_denials_by_role(
        &self,
        since: Option<&DateTime<Utc>>,
    ) -> Result<Vec<(String, i64)>, OrbitError>;
    fn get_audit_tool_call_counts_by_role(
        &self,
        since: Option<&DateTime<Utc>>,
    ) -> Result<Vec<AuditToolCallCountsByRole>, OrbitError>;
    fn prune_audit_events(&self, older_than: &DateTime<Utc>) -> Result<usize, OrbitError>;
}

pub trait ExecutorDefStoreBackend: Send + Sync {
    fn list_executor_defs(&self) -> Result<Vec<ExecutorDef>, OrbitError>;
    fn get_executor_def(&self, name: &str) -> Result<Option<ExecutorDef>, OrbitError>;
    fn upsert_executor_def(&self, def: &ExecutorDef) -> Result<(), OrbitError>;
}

pub trait PolicyDefStoreBackend: Send + Sync {
    fn list_policy_defs(&self) -> Result<Vec<PolicyDef>, OrbitError>;
    fn get_policy_def(&self, name: &str) -> Result<Option<PolicyDef>, OrbitError>;
    fn upsert_policy_def(&self, def: &PolicyDef) -> Result<(), OrbitError>;
}
