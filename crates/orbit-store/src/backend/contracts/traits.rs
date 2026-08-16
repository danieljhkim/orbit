use chrono::{DateTime, Utc};
use orbit_common::types::{
    ArtifactManifestFileV2, AuditEvent, Crew, ExecutorDef, ExternalRef, JobRun, JobRunState,
    KnowledgeRunMetrics, OrbitError, OrbitId, PipelineState, PolicyDef, StoredTool, Task,
    TaskArtifact, TaskComment, TaskHistoryEntry, TaskPriority, TaskStatus, normalize_task_tags,
    task_matches_tags,
};
use serde_json::Value;
use std::collections::BTreeMap;

use super::params::*;

use crate::sqlite::audit_event_store::incident::{FailureIncidentQuery, FailureIncidentReport};
use crate::sqlite::audit_event_store::{
    AuditEventFilter, AuditEventInsertParams, AuditRoleAggregate, AuditToolAggregate,
    AuditToolCallCountsByRole, AuditToolCallCountsBySurfaceAndRole, AuditTopToolCall,
};

pub trait TaskStoreBackend: Send + Sync {
    fn create_task(&self, params: TaskCreateParams) -> Result<Task, OrbitError>;
    fn list_tasks(&self) -> Result<Vec<Task>, OrbitError>;
    fn task_status_index(&self) -> Result<BTreeMap<OrbitId, TaskStatus>, OrbitError> {
        Ok(self
            .list_tasks()?
            .into_iter()
            .map(|task| (task.id, task.status))
            .collect())
    }
    fn list_tasks_by_tags(&self, tags: &[String]) -> Result<Vec<Task>, OrbitError> {
        let required_tags = normalize_task_tags(tags.to_vec());
        let mut tasks = self.list_tasks()?;
        if !required_tags.is_empty() {
            tasks.retain(|task| task_matches_tags(task, &required_tags));
        }
        Ok(tasks)
    }
    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        job_run_id: Option<&str>,
        external_ref: Option<&ExternalRef>,
        has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError>;
    fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError>;
    fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError>;
    fn search_tasks_filtered(&self, query: &str, tags: &[String]) -> Result<Vec<Task>, OrbitError> {
        let required_tags = normalize_task_tags(tags.to_vec());
        let mut tasks = self.search_tasks(query)?;
        if !required_tags.is_empty() {
            tasks.retain(|task| task_matches_tags(task, &required_tags));
        }
        Ok(tasks)
    }
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
    fn get_task_comments(&self, id: &str) -> Result<Option<Vec<TaskComment>>, OrbitError>;
    fn get_task_history(&self, id: &str) -> Result<Option<Vec<TaskHistoryEntry>>, OrbitError>;
    fn update_task_history(
        &self,
        id: &str,
        params: TaskHistoryUpdateParams,
    ) -> Result<(), OrbitError>;
}

pub trait TaskArtifactStoreBackend: Send + Sync {
    fn get_task_artifact_manifest(
        &self,
        _id: &str,
    ) -> Result<Option<Vec<ArtifactManifestFileV2>>, OrbitError> {
        Err(OrbitError::Store(
            "task artifact manifest read is not supported by this backend".to_string(),
        ))
    }
    fn get_task_artifacts(&self, id: &str) -> Result<Option<Vec<TaskArtifact>>, OrbitError>;
    fn get_task_artifact(
        &self,
        _id: &str,
        _path: &str,
    ) -> Result<Option<TaskArtifact>, OrbitError> {
        Err(OrbitError::Store(
            "task artifact read is not supported by this backend".to_string(),
        ))
    }
    fn upsert_task_artifacts(
        &self,
        id: &str,
        params: TaskArtifactUpdateParams,
    ) -> Result<(), OrbitError>;
}

pub trait TaskReservationStoreBackend: Send + Sync {
    /// Read active reservations without expiring or otherwise mutating rows.
    fn inspect_active_task_reservations(
        &self,
        workspace_orbit_dir: &str,
        workspace_id: Option<&str>,
    ) -> Result<Vec<ActiveTaskReservation>, OrbitError>;

    fn list_active_task_reservations(
        &self,
        workspace_orbit_dir: &str,
        workspace_id: Option<&str>,
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

    fn release_task_reservations_by_owner_run_id(
        &self,
        params: TaskReservationReleaseByOwnerParams,
    ) -> Result<TaskReservationReleaseByOwnerResult, OrbitError>;

    fn list_owned_task_reservation_conflicts(
        &self,
        params: TaskReservationOwnedConflictsParams,
    ) -> Result<TaskReservationOwnedConflictsResult, OrbitError>;

    /// Take the exclusive workspace claim [ADR-0352, ORB-10709], or report the
    /// incumbent that refused it.
    fn acquire_workspace_claim(
        &self,
        params: WorkspaceClaimAcquireParams,
    ) -> Result<WorkspaceClaimAcquireResult, OrbitError>;

    /// Release the claim with its token, or force-release it.
    fn release_workspace_claim(
        &self,
        params: WorkspaceClaimReleaseParams,
    ) -> Result<WorkspaceClaimReleaseResult, OrbitError>;

    /// The active claim after lazy expiry, or `None` when unclaimed.
    fn show_workspace_claim(
        &self,
        workspace_orbit_dir: &str,
        workspace_id: Option<&str>,
    ) -> Result<WorkspaceClaimStatusResult, OrbitError>;

    /// Whether a presented token satisfies the active claim. The comparison
    /// stays inside the store so a refusal never has to carry the incumbent's
    /// token back out.
    fn check_workspace_claim(
        &self,
        params: WorkspaceClaimCheckParams,
    ) -> Result<WorkspaceClaimCheckResult, OrbitError>;
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
    /// [ORB-10070] Record `pid` (+ its start-time identity token) as the owner
    /// of a still-`pending` run so orphan reconciliation can distinguish a
    /// queued run with a live worker from one whose worker died. Returns
    /// `false` without writing when the run is missing or no longer pending.
    fn claim_pending_job_run_owner(&self, run_id: &str, pid: u32) -> Result<bool, OrbitError>;
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
    fn record_job_run_crew(&self, run_id: &str, crew: &Crew) -> Result<bool, OrbitError>;
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
    fn get_audit_event_durations_null_tool(
        &self,
        since: &DateTime<Utc>,
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
    fn get_audit_tool_call_counts_by_surface_and_role(
        &self,
        since: Option<&DateTime<Utc>>,
    ) -> Result<Vec<AuditToolCallCountsBySurfaceAndRole>, OrbitError>;
    fn get_audit_top_tool_calls(
        &self,
        since: Option<&DateTime<Utc>>,
        limit: usize,
    ) -> Result<Vec<AuditTopToolCall>, OrbitError>;
    fn get_audit_event_aggregates_by_tool(
        &self,
        since: &DateTime<Utc>,
    ) -> Result<Vec<AuditToolAggregate>, OrbitError>;
    fn get_audit_event_aggregates_by_role(
        &self,
        since: &DateTime<Utc>,
    ) -> Result<Vec<AuditRoleAggregate>, OrbitError>;
    /// Failure incidents grouped from the raw failed/denied rows in `query`'s
    /// window [ORB-10871]. A derived view: it neither mutates nor withholds
    /// any row that `list_audit_events` would return.
    fn get_failure_incidents(
        &self,
        query: &FailureIncidentQuery,
    ) -> Result<FailureIncidentReport, OrbitError>;
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
