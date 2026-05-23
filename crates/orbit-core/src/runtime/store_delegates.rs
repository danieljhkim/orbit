use orbit_common::types::{
    Adr, AdrStatus, ArtifactManifestFileV2, AuditEvent, ExecutorDef, ExternalRef, JobRun,
    JobRunState, KnowledgeRunMetrics, Learning, LearningStatus, NotFoundKind, OrbitError,
    PolicyDef, ReviewThread, StoredTool, Task, TaskArtifact, TaskComment, TaskComplexity,
    TaskHistoryEntry, TaskPriority, TaskRelation, TaskStatus, TaskType,
};
use orbit_search::{EmbedWorker, VectorStore};
use orbit_store::{
    AdrCreateParams, AdrDocumentUpdateParams, AdrListEntry, AdrListFilter, AdrStoreBackend,
    AuditEventFilter, AuditEventInsertParams, AuditEventStoreBackend, ExecutorDefStoreBackend,
    JobRunQuery, JobRunStepParams, JobRunStoreBackend, LearningCommentAddParams,
    LearningCommentDeleteParams, LearningCreateParams, LearningListEntry, LearningSearchParams,
    LearningSearchResult, LearningStoreBackend, LearningUpdateParams, LearningUpvoteParams,
    PolicyDefStoreBackend, RemoteArtifactStub, TaskArtifactStoreBackend, TaskArtifactUpdateParams,
    TaskCreateParams, TaskDocumentStoreBackend, TaskDocumentUpdateParams, TaskHistoryStoreBackend,
    TaskHistoryUpdateParams, TaskReservationCheckParams, TaskReservationCheckResult,
    TaskReservationListResult, TaskReservationOwnedConflictsParams,
    TaskReservationOwnedConflictsResult, TaskReservationReleaseByOwnerParams,
    TaskReservationReleaseByOwnerResult, TaskReservationReleaseParams,
    TaskReservationReleaseResult, TaskReservationReserveParams, TaskReservationReserveResult,
    TaskReservationStoreBackend, TaskReviewStoreBackend, TaskReviewUpdateParams, TaskStoreBackend,
    ToolStoreBackend,
};

use crate::context::OrbitStores;

#[derive(Default, Clone)]
pub(crate) struct TaskRecordUpdateParams {
    pub(crate) actor: String,
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) acceptance_criteria: Option<Vec<String>>,
    pub(crate) dependencies: Option<Vec<String>>,
    pub(crate) relations: Option<Vec<TaskRelation>>,
    pub(crate) tags: Option<Vec<String>>,
    pub(crate) plan: Option<String>,
    pub(crate) execution_summary: Option<String>,
    pub(crate) context_files: Option<Vec<String>>,
    pub(crate) created_by: Option<Option<String>>,
    pub(crate) planned_by: Option<Option<String>>,
    pub(crate) implemented_by: Option<Option<String>>,
    pub(crate) status: Option<TaskStatus>,
    pub(crate) priority: Option<TaskPriority>,
    pub(crate) complexity: Option<TaskComplexity>,
    pub(crate) task_type: Option<TaskType>,
    pub(crate) external_refs: Option<Vec<ExternalRef>>,
    pub(crate) pr_status: Option<Option<String>>,
    pub(crate) source_task_id: Option<Option<String>>,
    pub(crate) job_run_id: Option<Option<String>>,
    pub(crate) crew: Option<Option<String>>,
    pub(crate) status_event: Option<String>,
    pub(crate) status_note: Option<String>,
    pub(crate) append_history: Vec<TaskHistoryEntry>,
    pub(crate) append_comments: Vec<TaskComment>,
    pub(crate) append_review_threads: Vec<ReviewThread>,
    pub(crate) replace_review_threads: Option<Vec<ReviewThread>>,
    pub(crate) upsert_artifacts: Vec<TaskArtifact>,
}

impl TaskRecordUpdateParams {
    fn has_document_changes(&self) -> bool {
        self.title.is_some()
            || self.description.is_some()
            || self.acceptance_criteria.is_some()
            || self.dependencies.is_some()
            || self.relations.is_some()
            || self.tags.is_some()
            || self.plan.is_some()
            || self.execution_summary.is_some()
            || self.context_files.is_some()
            || self.created_by.is_some()
            || self.planned_by.is_some()
            || self.implemented_by.is_some()
            || self.priority.is_some()
            || self.complexity.is_some()
            || self.task_type.is_some()
            || self.external_refs.is_some()
            || self.pr_status.is_some()
            || self.source_task_id.is_some()
            || self.job_run_id.is_some()
            || self.crew.is_some()
    }

    fn has_history_changes(&self) -> bool {
        self.status.is_some()
            || self.status_event.is_some()
            || self.status_note.is_some()
            || !self.append_history.is_empty()
            || !self.append_comments.is_empty()
    }

    fn has_review_changes(&self) -> bool {
        self.replace_review_threads.is_some() || !self.append_review_threads.is_empty()
    }

    fn has_artifact_changes(&self) -> bool {
        !self.upsert_artifacts.is_empty()
    }
}

impl OrbitStores {
    pub(crate) fn tasks(&self) -> TaskRecords<'_> {
        TaskRecords {
            store: self.task.as_ref(),
            document: self.task_document.as_ref(),
            history: self.task_history.as_ref(),
            review: self.task_review.as_ref(),
            artifact: self.task_artifact.as_ref(),
            semantic_vector: self.semantic_vector.as_ref(),
            semantic_worker: self.semantic_worker.as_ref(),
        }
    }

    pub(crate) fn adrs(&self) -> AdrRecords<'_> {
        AdrRecords {
            store: self.adr.as_ref(),
        }
    }

    pub(crate) fn learnings(&self) -> LearningRecords<'_> {
        LearningRecords {
            store: self.learning.as_ref(),
        }
    }

    pub(crate) fn task_reservations(&self) -> TaskReservationRecords<'_> {
        TaskReservationRecords {
            store: self.task_reservation.as_ref(),
        }
    }

    pub(crate) fn jobs(&self) -> JobRecords<'_> {
        JobRecords {
            run: self.job_run.as_ref(),
        }
    }

    pub(crate) fn tools(&self) -> ToolRecords<'_> {
        ToolRecords {
            store: self.tool.as_ref(),
        }
    }

    pub(crate) fn audit_events(&self) -> AuditEventRecords<'_> {
        AuditEventRecords {
            store: self.audit_event.as_ref(),
        }
    }

    pub(crate) fn executors(&self) -> ExecutorDefRecords<'_> {
        ExecutorDefRecords {
            store: self.executor_def.as_ref(),
        }
    }

    pub(crate) fn policies(&self) -> PolicyDefRecords<'_> {
        PolicyDefRecords {
            store: self.policy_def.as_ref(),
        }
    }
}

pub(crate) struct TaskRecords<'a> {
    store: &'a dyn TaskStoreBackend,
    document: &'a dyn TaskDocumentStoreBackend,
    history: &'a dyn TaskHistoryStoreBackend,
    review: &'a dyn TaskReviewStoreBackend,
    artifact: &'a dyn TaskArtifactStoreBackend,
    semantic_vector: &'a VectorStore,
    semantic_worker: &'a EmbedWorker,
}

pub(crate) struct TaskReservationRecords<'a> {
    store: &'a dyn TaskReservationStoreBackend,
}

impl TaskRecords<'_> {
    pub(crate) fn create(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        let task = self.store.create_task(params)?;
        self.semantic_worker.enqueue(task.clone());
        Ok(task)
    }

    pub(crate) fn get(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        self.store.get_task(id)
    }

    pub(crate) fn get_artifacts(&self, id: &str) -> Result<Option<Vec<TaskArtifact>>, OrbitError> {
        self.artifact.get_task_artifacts(id)
    }

    pub(crate) fn get_artifact_manifest(
        &self,
        id: &str,
    ) -> Result<Option<Vec<ArtifactManifestFileV2>>, OrbitError> {
        self.artifact.get_task_artifact_manifest(id)
    }

    pub(crate) fn get_artifact(
        &self,
        id: &str,
        path: &str,
    ) -> Result<Option<TaskArtifact>, OrbitError> {
        self.artifact.get_task_artifact(id, path)
    }

    pub(crate) fn get_comments(&self, id: &str) -> Result<Option<Vec<TaskComment>>, OrbitError> {
        self.history.get_task_comments(id)
    }

    pub(crate) fn get_history(
        &self,
        id: &str,
    ) -> Result<Option<Vec<TaskHistoryEntry>>, OrbitError> {
        self.history.get_task_history(id)
    }

    pub(crate) fn get_review_threads(
        &self,
        id: &str,
    ) -> Result<Option<Vec<ReviewThread>>, OrbitError> {
        self.review.get_task_review_threads(id)
    }

    pub(crate) fn list(&self) -> Result<Vec<Task>, OrbitError> {
        self.store.list_tasks()
    }

    pub(crate) fn list_by_tags(&self, tags: &[String]) -> Result<Vec<Task>, OrbitError> {
        self.store.list_tasks_by_tags(tags)
    }

    pub(crate) fn list_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        job_run_id: Option<&str>,
        external_ref: Option<&ExternalRef>,
        has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.store.list_tasks_filtered(
            status,
            priority,
            parent_id,
            job_run_id,
            external_ref,
            has_external_ref_system,
        )
    }

    pub(crate) fn update(
        &self,
        id: &str,
        params: TaskRecordUpdateParams,
    ) -> Result<Task, OrbitError> {
        if params.has_document_changes() {
            self.document.update_task_document(
                id,
                TaskDocumentUpdateParams {
                    actor: params.actor.clone(),
                    title: params.title.clone(),
                    description: params.description.clone(),
                    acceptance_criteria: params.acceptance_criteria.clone(),
                    dependencies: params.dependencies.clone(),
                    relations: params.relations.clone(),
                    tags: params.tags.clone(),
                    plan: params.plan.clone(),
                    execution_summary: params.execution_summary.clone(),
                    context_files: params.context_files.clone(),
                    created_by: params.created_by.clone(),
                    planned_by: params.planned_by.clone(),
                    implemented_by: params.implemented_by.clone(),
                    priority: params.priority,
                    complexity: params.complexity,
                    task_type: params.task_type,
                    external_refs: params.external_refs.clone(),
                    pr_status: params.pr_status.clone(),
                    source_task_id: params.source_task_id.clone(),
                    job_run_id: params.job_run_id.clone(),
                    crew: params.crew.clone(),
                },
            )?;
        }

        if params.has_history_changes() {
            self.history.update_task_history(
                id,
                TaskHistoryUpdateParams {
                    actor: params.actor.clone(),
                    status: params.status,
                    status_event: params.status_event.clone(),
                    status_note: params.status_note.clone(),
                    append_history: params.append_history.clone(),
                    append_comments: params.append_comments.clone(),
                },
            )?;
        }

        if params.has_review_changes() {
            self.review.update_task_reviews(
                id,
                TaskReviewUpdateParams {
                    append_review_threads: params.append_review_threads.clone(),
                    replace_review_threads: params.replace_review_threads.clone(),
                },
            )?;
        }

        if params.has_artifact_changes() {
            self.artifact.upsert_task_artifacts(
                id,
                TaskArtifactUpdateParams {
                    actor: params.actor.clone(),
                    upsert_artifacts: params.upsert_artifacts.clone(),
                },
            )?;
        }

        let task = self
            .get(id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, id.to_string()))?;
        if params.has_document_changes()
            || params.has_history_changes()
            || params.has_review_changes()
            || params.has_artifact_changes()
        {
            self.semantic_worker.enqueue(task.clone());
        }
        Ok(task)
    }

    pub(crate) fn delete(&self, id: &str) -> Result<bool, OrbitError> {
        let deleted = self.store.delete_task(id)?;
        if deleted && let Err(error) = self.semantic_vector.delete_source("task", id) {
            orbit_common::tracing::debug!(
                target: "orbit.search.indexer",
                task_id = id,
                error = %error,
                "semantic delete cascade failed after task deletion",
            );
        }
        Ok(deleted)
    }

    pub(crate) fn search(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.store.search_tasks(query)
    }

    pub(crate) fn search_filtered(
        &self,
        query: &str,
        tags: &[String],
    ) -> Result<Vec<Task>, OrbitError> {
        self.store.search_tasks_filtered(query, tags)
    }
}

impl TaskReservationRecords<'_> {
    pub(crate) fn list_active(
        &self,
        workspace_orbit_dir: &str,
        workspace_id: Option<&str>,
    ) -> Result<TaskReservationListResult, OrbitError> {
        self.store
            .list_active_task_reservations(workspace_orbit_dir, workspace_id)
    }

    pub(crate) fn check(
        &self,
        params: TaskReservationCheckParams,
    ) -> Result<TaskReservationCheckResult, OrbitError> {
        self.store.check_task_reservation_conflicts(params)
    }

    pub(crate) fn reserve(
        &self,
        params: TaskReservationReserveParams,
    ) -> Result<TaskReservationReserveResult, OrbitError> {
        self.store.reserve_task_reservation(params)
    }

    pub(crate) fn release(
        &self,
        params: TaskReservationReleaseParams,
    ) -> Result<TaskReservationReleaseResult, OrbitError> {
        self.store.release_task_reservation(params)
    }

    pub(crate) fn release_by_owner_run_id(
        &self,
        params: TaskReservationReleaseByOwnerParams,
    ) -> Result<TaskReservationReleaseByOwnerResult, OrbitError> {
        self.store.release_task_reservations_by_owner_run_id(params)
    }

    pub(crate) fn list_owned_conflicts(
        &self,
        params: TaskReservationOwnedConflictsParams,
    ) -> Result<TaskReservationOwnedConflictsResult, OrbitError> {
        self.store.list_owned_task_reservation_conflicts(params)
    }
}

pub(crate) struct JobRecords<'a> {
    run: &'a dyn JobRunStoreBackend,
}

impl JobRecords<'_> {
    pub(crate) fn list_runs_filtered(
        &self,
        query: &JobRunQuery,
    ) -> Result<Vec<JobRun>, OrbitError> {
        self.run.list_job_runs_filtered(query)
    }

    pub(crate) fn list_all_pending_or_running(&self) -> Result<Vec<JobRun>, OrbitError> {
        self.run.list_all_pending_or_running_runs()
    }

    pub(crate) fn list_pending_or_running(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.run.list_pending_or_running_job_runs(job_id)
    }

    pub(crate) fn insert_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: chrono::DateTime<chrono::Utc>,
        input: Option<serde_json::Value>,
        retry_source_run_id: Option<String>,
    ) -> Result<JobRun, OrbitError> {
        self.run
            .insert_job_run(job_id, attempt, scheduled_at, input, retry_source_run_id)
    }

    pub(crate) fn mark_run_running(
        &self,
        run_id: &str,
        started_at: chrono::DateTime<chrono::Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.run.mark_job_run_running(run_id, started_at, pid)
    }

    pub(crate) fn take_over_running_run(
        &self,
        run_id: &str,
        expected_pid: Option<u32>,
        expected_pid_start_time: Option<String>,
        started_at: chrono::DateTime<chrono::Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.run.take_over_running_job_run(
            run_id,
            expected_pid,
            expected_pid_start_time,
            started_at,
            pid,
        )
    }

    pub(crate) fn abandon_run(
        &self,
        run_id: &str,
        finished_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, OrbitError> {
        self.run.abandon_job_run(run_id, finished_at)
    }

    pub(crate) fn complete_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.run.complete_job_run_step(run_id, params)
    }

    pub(crate) fn record_run_knowledge_metrics(
        &self,
        run_id: &str,
        metrics: KnowledgeRunMetrics,
    ) -> Result<bool, OrbitError> {
        self.run.record_job_run_knowledge_metrics(run_id, metrics)
    }

    pub(crate) fn record_run_crew(
        &self,
        run_id: &str,
        crew: &orbit_common::types::Crew,
    ) -> Result<bool, OrbitError> {
        self.run.record_job_run_crew(run_id, crew)
    }

    pub(crate) fn finalize_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: chrono::DateTime<chrono::Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.run
            .finalize_job_run(run_id, state, finished_at, duration_ms)
    }

    pub(crate) fn repair_terminal_run_timing(
        &self,
        run_id: &str,
        finished_at: chrono::DateTime<chrono::Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.run
            .repair_terminal_job_run_timing(run_id, finished_at, duration_ms)
    }

    pub(crate) fn get_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.run.get_job_run(run_id)
    }

    pub(crate) fn read_run_state(
        &self,
        run_id: &str,
    ) -> Result<Option<orbit_common::types::PipelineState>, OrbitError> {
        self.run.read_run_state(run_id)
    }

    pub(crate) fn write_run_state(
        &self,
        run_id: &str,
        state: &orbit_common::types::PipelineState,
    ) -> Result<(), OrbitError> {
        self.run.write_run_state(run_id, state)
    }

    pub(crate) fn list_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.run.list_job_runs(job_id)
    }

    pub(crate) fn archive_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.run.archive_job_run(run_id)
    }

    pub(crate) fn delete_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.run.delete_job_run(run_id)
    }
}

pub(crate) struct ToolRecords<'a> {
    store: &'a dyn ToolStoreBackend,
}

impl ToolRecords<'_> {
    pub(crate) fn list(&self) -> Result<Vec<StoredTool>, OrbitError> {
        self.store.list_tools()
    }

    pub(crate) fn get(&self, name: &str) -> Result<Option<StoredTool>, OrbitError> {
        self.store.get_tool(name)
    }

    pub(crate) fn insert(&self, tool: &StoredTool) -> Result<(), OrbitError> {
        self.store.insert_tool(tool)
    }

    pub(crate) fn delete(&self, name: &str) -> Result<bool, OrbitError> {
        self.store.delete_tool(name)
    }

    pub(crate) fn set_enabled(&self, name: &str, enabled: bool) -> Result<bool, OrbitError> {
        self.store.set_tool_enabled(name, enabled)
    }
}

pub(crate) struct AuditEventRecords<'a> {
    store: &'a dyn AuditEventStoreBackend,
}

impl AuditEventRecords<'_> {
    pub(crate) fn list(&self, filter: &AuditEventFilter) -> Result<Vec<AuditEvent>, OrbitError> {
        self.store.list_audit_events(filter)
    }

    pub(crate) fn get(&self, id: i64) -> Result<Option<AuditEvent>, OrbitError> {
        self.store.get_audit_event(id)
    }

    pub(crate) fn prune(
        &self,
        older_than: &chrono::DateTime<chrono::Utc>,
    ) -> Result<usize, OrbitError> {
        self.store.prune_audit_events(older_than)
    }

    pub(crate) fn stats(
        &self,
        since: Option<&chrono::DateTime<chrono::Utc>>,
        tool: Option<&str>,
    ) -> Result<(i64, i64, i64, i64, f64, i64), OrbitError> {
        self.store.get_audit_event_stats(since, tool)
    }

    pub(crate) fn durations(
        &self,
        since: Option<&chrono::DateTime<chrono::Utc>>,
        tool: Option<&str>,
    ) -> Result<Vec<i64>, OrbitError> {
        self.store.get_audit_event_durations(since, tool)
    }

    pub(crate) fn durations_null_tool(
        &self,
        since: &chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<i64>, OrbitError> {
        self.store.get_audit_event_durations_null_tool(since)
    }

    pub(crate) fn hourly_buckets(
        &self,
        since: &chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<(String, i64)>, OrbitError> {
        self.store.get_audit_event_hourly_buckets(since)
    }

    pub(crate) fn denials_by_role(
        &self,
        since: Option<&chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<(String, i64)>, OrbitError> {
        self.store.get_audit_denials_by_role(since)
    }

    pub(crate) fn tool_call_counts_by_role(
        &self,
        since: Option<&chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<orbit_store::AuditToolCallCountsByRole>, OrbitError> {
        self.store.get_audit_tool_call_counts_by_role(since)
    }

    pub(crate) fn tool_call_counts_by_surface_and_role(
        &self,
        since: Option<&chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<orbit_store::AuditToolCallCountsBySurfaceAndRole>, OrbitError> {
        self.store
            .get_audit_tool_call_counts_by_surface_and_role(since)
    }

    pub(crate) fn top_tool_calls(
        &self,
        since: Option<&chrono::DateTime<chrono::Utc>>,
        limit: usize,
    ) -> Result<Vec<orbit_store::AuditTopToolCall>, OrbitError> {
        self.store.get_audit_top_tool_calls(since, limit)
    }

    pub(crate) fn aggregates_by_tool(
        &self,
        since: &chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<orbit_store::AuditToolAggregate>, OrbitError> {
        self.store.get_audit_event_aggregates_by_tool(since)
    }

    pub(crate) fn aggregates_by_role(
        &self,
        since: &chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<orbit_store::AuditRoleAggregate>, OrbitError> {
        self.store.get_audit_event_aggregates_by_role(since)
    }

    pub(crate) fn insert(&self, params: &AuditEventInsertParams) -> Result<(), OrbitError> {
        self.store.insert_audit_event_record(params)
    }
}

pub(crate) struct ExecutorDefRecords<'a> {
    store: &'a dyn ExecutorDefStoreBackend,
}

impl ExecutorDefRecords<'_> {
    pub(crate) fn list(&self) -> Result<Vec<ExecutorDef>, OrbitError> {
        self.store.list_executor_defs()
    }

    pub(crate) fn get(&self, name: &str) -> Result<Option<ExecutorDef>, OrbitError> {
        self.store.get_executor_def(name)
    }

    pub(crate) fn upsert(&self, def: &ExecutorDef) -> Result<(), OrbitError> {
        self.store.upsert_executor_def(def)
    }
}

pub(crate) struct PolicyDefRecords<'a> {
    store: &'a dyn PolicyDefStoreBackend,
}

impl PolicyDefRecords<'_> {
    pub(crate) fn list(&self) -> Result<Vec<PolicyDef>, OrbitError> {
        self.store.list_policy_defs()
    }

    pub(crate) fn get(&self, name: &str) -> Result<Option<PolicyDef>, OrbitError> {
        self.store.get_policy_def(name)
    }

    pub(crate) fn upsert(&self, def: &PolicyDef) -> Result<(), OrbitError> {
        self.store.upsert_policy_def(def)
    }
}

pub(crate) struct AdrRecords<'a> {
    store: &'a dyn AdrStoreBackend,
}

impl AdrRecords<'_> {
    pub(crate) fn add(&self, params: AdrCreateParams) -> Result<Adr, OrbitError> {
        self.store.add_adr(params)
    }

    pub(crate) fn get(&self, id: &str) -> Result<Option<Adr>, OrbitError> {
        self.store.get_adr(id)
    }

    pub(crate) fn get_federated(&self, id: &str) -> Result<Option<Adr>, OrbitError> {
        self.store.get_adr_federated(id)
    }

    /// Unfiltered list. The tool surface uses [`Self::list_filtered`]; this
    /// helper exists for maintenance / CLI tooling layered on top later.
    #[allow(dead_code)]
    pub(crate) fn list(&self) -> Result<Vec<Adr>, OrbitError> {
        self.store.list_adrs()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn list_filtered(
        &self,
        status: Option<AdrStatus>,
        owner: Option<&str>,
        feature: Option<&str>,
        task_id: Option<&str>,
        legacy_id: Option<&str>,
        tag: Option<&str>,
        path: Option<&str>,
        validation_warned: Option<bool>,
    ) -> Result<Vec<Adr>, OrbitError> {
        self.store.list_adrs_filtered(AdrListFilter {
            status,
            owner,
            feature,
            task_id,
            legacy_id,
            tag,
            path,
            validation_warned,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn list_entries_filtered(
        &self,
        status: Option<AdrStatus>,
        owner: Option<&str>,
        feature: Option<&str>,
        task_id: Option<&str>,
        legacy_id: Option<&str>,
        tag: Option<&str>,
        path: Option<&str>,
        validation_warned: Option<bool>,
        include_remote: bool,
    ) -> Result<Vec<AdrListEntry>, OrbitError> {
        self.store.list_adr_entries_filtered(
            AdrListFilter {
                status,
                owner,
                feature,
                task_id,
                legacy_id,
                tag,
                path,
                validation_warned,
            },
            include_remote,
        )
    }

    pub(crate) fn remote_stub(&self, id: &str) -> Result<Option<RemoteArtifactStub>, OrbitError> {
        self.store.get_adr_remote_stub(id)
    }

    pub(crate) fn update_status(&self, id: &str, new_status: AdrStatus) -> Result<(), OrbitError> {
        self.store.update_adr_status(id, new_status)
    }

    pub(crate) fn update_document(
        &self,
        id: &str,
        fields: &AdrDocumentUpdateParams,
    ) -> Result<(), OrbitError> {
        self.store.update_adr_document(id, fields)
    }

    /// Removes an ADR from disk and index. Reserved for the future
    /// migration / cleanup CLI; not exposed via the tool surface.
    #[allow(dead_code)]
    pub(crate) fn delete(&self, id: &str) -> Result<bool, OrbitError> {
        self.store.delete_adr(id)
    }

    pub(crate) fn supersede(&self, old_id: &str, new_id: &str) -> Result<(), OrbitError> {
        self.store.supersede_adr(old_id, new_id)
    }
}

pub(crate) struct LearningRecords<'a> {
    store: &'a dyn LearningStoreBackend,
}

impl LearningRecords<'_> {
    pub(crate) fn add(&self, params: LearningCreateParams) -> Result<Learning, OrbitError> {
        self.store.create_learning(params)
    }

    pub(crate) fn get(&self, id: &str) -> Result<Option<Learning>, OrbitError> {
        self.store.get_learning(id)
    }

    pub(crate) fn get_federated(&self, id: &str) -> Result<Option<Learning>, OrbitError> {
        self.store.get_learning_federated(id)
    }

    pub(crate) fn list(&self, status: Option<LearningStatus>) -> Result<Vec<Learning>, OrbitError> {
        self.store.list_learnings(status)
    }

    pub(crate) fn list_entries(
        &self,
        status: Option<LearningStatus>,
        include_remote: bool,
    ) -> Result<Vec<LearningListEntry>, OrbitError> {
        self.store.list_learning_entries(status, include_remote)
    }

    pub(crate) fn remote_stub(&self, id: &str) -> Result<Option<RemoteArtifactStub>, OrbitError> {
        self.store.get_learning_remote_stub(id)
    }

    pub(crate) fn search(
        &self,
        params: LearningSearchParams,
    ) -> Result<Vec<LearningSearchResult>, OrbitError> {
        self.store.search_learnings(params)
    }

    pub(crate) fn upvote(
        &self,
        params: LearningUpvoteParams,
    ) -> Result<orbit_common::types::LearningVoteSummary, OrbitError> {
        self.store.upvote_learning(params)
    }

    pub(crate) fn vote_summary(
        &self,
        id: &str,
    ) -> Result<orbit_common::types::LearningVoteSummary, OrbitError> {
        self.store.learning_vote_summary(id)
    }

    pub(crate) fn add_comment(
        &self,
        params: LearningCommentAddParams,
    ) -> Result<orbit_common::types::LearningComment, OrbitError> {
        self.store.add_learning_comment(params)
    }

    pub(crate) fn list_comments(
        &self,
        learning_id: &str,
        include_deleted: bool,
    ) -> Result<Vec<orbit_common::types::LearningComment>, OrbitError> {
        self.store
            .list_learning_comments(learning_id, include_deleted)
    }

    pub(crate) fn delete_comment(
        &self,
        params: LearningCommentDeleteParams,
    ) -> Result<(), OrbitError> {
        self.store.delete_learning_comment(params)
    }

    pub(crate) fn update(
        &self,
        id: &str,
        params: LearningUpdateParams,
    ) -> Result<Learning, OrbitError> {
        self.store.update_learning(id, params)
    }

    pub(crate) fn supersede(&self, old_id: &str, new_id: &str) -> Result<(), OrbitError> {
        self.store.supersede_learning(old_id, new_id)
    }

    pub(crate) fn archive(&self, id: &str) -> Result<bool, OrbitError> {
        self.store.archive_learning(id)
    }

    /// Hard-deletes a learning's YAML file and index row. Reserved for
    /// maintenance / migration tooling; the tool surface uses `archive`
    /// for stale-record cleanup per §7.3.
    #[allow(dead_code)]
    pub(crate) fn delete(&self, id: &str) -> Result<bool, OrbitError> {
        self.store.delete_learning(id)
    }

    pub(crate) fn sync(&self) -> Result<(), OrbitError> {
        self.store.sync_learnings()
    }
}
