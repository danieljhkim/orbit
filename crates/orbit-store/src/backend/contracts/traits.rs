use chrono::{DateTime, Utc};
use orbit_common::types::{
    Adr, AdrStatus, ArtifactManifestFileV2, AuditEvent, Crew, ExecutorDef, ExternalRef, JobRun,
    JobRunState, KnowledgeRunMetrics, Learning, LearningEvidence, LearningScope, OrbitError,
    OrbitId, PipelineState, PolicyDef, StoredTool, Task, TaskArtifact, TaskComment,
    TaskHistoryEntry, TaskPriority, TaskStatus, normalize_task_tags, task_matches_tags,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

use super::params::*;

use crate::sqlite::id_allocator::IdAllocationRecord;

use crate::sqlite::audit_event_store::{
    AuditEventFilter, AuditEventInsertParams, AuditRoleAggregate, AuditToolAggregate,
    AuditToolCallCountsByRole, AuditToolCallCountsBySurfaceAndRole, AuditTopToolCall,
    LearningUsageStat,
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

pub trait AdrStoreBackend: Send + Sync {
    fn add_adr(&self, params: AdrCreateParams) -> Result<Adr, OrbitError>;

    /// [ORB-10330] Finalize a hub-preallocated ADR at the caller-supplied
    /// canonical `id`. Unlike [`Self::add_adr`], the id is chosen upstream by
    /// the hub sequence, so this never allocates, abandons, retries, or selects
    /// a second id; a pre-existing artifact at `id` fails deterministically.
    fn finalize_preallocated_adr(
        &self,
        id: &str,
        params: AdrCreateParams,
    ) -> Result<Adr, OrbitError>;

    /// [ORB-10538] Restore an ADR at an existing allocation whose local and
    /// canonical artifacts are unreadable. Never allocates or overwrites.
    fn restore_allocated_adr(&self, id: &str, params: AdrCreateParams) -> Result<Adr, OrbitError>;

    /// [ORB-10545] Copy a complete bundle from a registered sibling worktree
    /// into the current checkout without reallocating or changing lifecycle
    /// metadata.
    fn reconcile_federated_adr(&self, id: &str, source_worktree: &Path) -> Result<Adr, OrbitError>;
    fn get_adr(&self, id: &str) -> Result<Option<Adr>, OrbitError>;
    fn resolve_adr_artifact(&self, id: &str) -> Result<AdrArtifactResolution, OrbitError>;
    fn list_adrs(&self) -> Result<Vec<Adr>, OrbitError>;
    fn list_adrs_filtered(&self, filter: AdrListFilter<'_>) -> Result<Vec<Adr>, OrbitError>;
    fn list_adr_entries_filtered(
        &self,
        filter: AdrListFilter<'_>,
        include_remote: bool,
    ) -> Result<Vec<AdrListEntry>, OrbitError>;
    fn get_adr_remote_stub(&self, id: &str) -> Result<Option<RemoteArtifactStub>, OrbitError>;

    /// [ORB-10501] Allocations pinned to a worktree that no longer exists and
    /// whose bundle is not readable anywhere locally — permanently orphaned
    /// index rows, reported by `orbit doctor`.
    fn list_orphaned_adr_allocations(&self) -> Result<Vec<IdAllocationRecord>, OrbitError>;

    /// [ORB-10501] Abandon one orphaned allocation row. `false` when the id
    /// has no live allocation; an error when it is still recoverable.
    fn abandon_orphaned_adr_allocation(&self, id: &str) -> Result<bool, OrbitError>;

    fn update_adr_status(&self, id: &str, new_status: AdrStatus) -> Result<(), OrbitError>;
    fn update_adr_document(
        &self,
        id: &str,
        fields: &AdrDocumentUpdateParams,
    ) -> Result<(), OrbitError>;
    fn delete_adr(&self, id: &str) -> Result<bool, OrbitError>;
    fn rebuild_index(&self) -> Result<(), OrbitError>;

    /// Writes the bidirectional supersession edge between two ADRs.
    ///
    /// On success: `old.status = Superseded`, `old.superseded_by = Some(new)`,
    /// `new.supersedes` contains `old`. The implementation acquires per-ADR
    /// locks for the duration so concurrent writers serialize.
    ///
    /// **Atomicity caveat:** the filesystem writes that update both ADR
    /// documents are sequential, not transactional. A crash between writes
    /// leaves the filesystem source-of-truth in a recoverable state — both ADR
    /// bundles survive, and `rebuild_index` reconstructs the SQLite index from
    /// disk.
    fn supersede_adr(&self, old_id: &str, new_id: &str) -> Result<(), OrbitError>;
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
    fn get_learning_usage_stats(
        &self,
        since: Option<&DateTime<Utc>>,
    ) -> Result<Vec<LearningUsageStat>, OrbitError>;
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

/// Parameters for creating a new [`Learning`] record.
#[derive(Debug, Clone)]
pub struct LearningCreateParams {
    pub summary: String,
    pub scope: LearningScope,
    pub body: String,
    pub evidence: Vec<LearningEvidence>,
    pub created_by: Option<String>,
    /// Optional explicit priority. Used as a secondary key in `search`
    /// ranking; `None` ranks below any `Some(_)`.
    pub priority: Option<u8>,
}

/// Partial update to an existing learning. Fields that are `None` are left
/// unchanged. Mirrors the `*UpdateParams` convention used for tasks.
#[derive(Debug, Clone, Default)]
pub struct LearningUpdateParams {
    pub summary: Option<String>,
    pub scope: Option<LearningScope>,
    pub body: Option<String>,
    pub evidence: Option<Vec<LearningEvidence>>,
    /// `Some(Some(N))` sets the priority; `Some(None)` clears it; `None`
    /// leaves it unchanged.
    pub priority: Option<Option<u8>>,
}

/// Search query for [`LearningStoreBackend::search_learnings`]. All fields
/// are optional; an empty query returns the active set unfiltered (capped
/// by `limit`).
#[derive(Debug, Clone, Default)]
pub struct LearningSearchParams {
    pub path: Option<String>,
    pub tag: Option<String>,
    pub query: Option<String>,
    pub limit: Option<usize>,
}

/// Result row from [`LearningStoreBackend::search_learnings`]. Carries
/// `matched_by` so callers can attribute matches to their scope axis (path
/// vs. tag vs. query) per the design's §5.3 result shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningSearchResult {
    pub learning: Learning,
    pub matched_by: Vec<String>,
}

pub trait LearningStoreBackend: Send + Sync {
    fn create_learning(&self, params: LearningCreateParams) -> Result<Learning, OrbitError>;

    /// [ORB-10330] Finalize a hub-preallocated learning at the caller-supplied
    /// canonical `id`. Unlike [`Self::create_learning`], there is no allocation
    /// loop and the id is never selected, abandoned, retried, or replaced; a
    /// path collision fails deterministically and preserves the existing
    /// artifact.
    fn finalize_preallocated_learning(
        &self,
        id: &str,
        params: LearningCreateParams,
    ) -> Result<Learning, OrbitError>;
    fn get_learning(&self, id: &str) -> Result<Option<Learning>, OrbitError>;
    fn get_learning_federated(&self, id: &str) -> Result<Option<Learning>, OrbitError>;
    fn list_learnings(
        &self,
        status: Option<orbit_common::types::LearningStatus>,
    ) -> Result<Vec<Learning>, OrbitError>;
    fn list_learning_entries(
        &self,
        status: Option<orbit_common::types::LearningStatus>,
        include_remote: bool,
    ) -> Result<Vec<LearningListEntry>, OrbitError>;
    fn get_learning_remote_stub(&self, id: &str) -> Result<Option<RemoteArtifactStub>, OrbitError>;

    /// [ORB-10501] Allocations pinned to a worktree that no longer exists and
    /// whose body is not readable anywhere locally — permanently orphaned
    /// index rows, reported by `orbit doctor`.
    fn list_orphaned_learning_allocations(&self) -> Result<Vec<IdAllocationRecord>, OrbitError>;

    /// [ORB-10501] Abandon one orphaned allocation row. `false` when the id
    /// has no live allocation; an error when it is still recoverable.
    fn abandon_orphaned_learning_allocation(&self, id: &str) -> Result<bool, OrbitError>;

    fn search_learnings(
        &self,
        params: LearningSearchParams,
    ) -> Result<Vec<LearningSearchResult>, OrbitError>;
    fn update_learning(
        &self,
        id: &str,
        params: LearningUpdateParams,
    ) -> Result<Learning, OrbitError>;
    fn supersede_learning(&self, old_id: &str, new_id: &str) -> Result<(), OrbitError>;
    /// Archive a learning without a replacement record. Flips
    /// `status = superseded` and sets `superseded_by = None`. Returns `false` when the record does not
    /// exist. Used by `prune --delete` (§7.3).
    fn archive_learning(&self, id: &str) -> Result<bool, OrbitError>;
    fn delete_learning(&self, id: &str) -> Result<bool, OrbitError>;
    fn sync_learnings(&self) -> Result<(), OrbitError>;
}
