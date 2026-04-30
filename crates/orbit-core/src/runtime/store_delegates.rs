use orbit_common::types::{
    AuditEvent, ExecutorDef, JobRun, JobRunState, KnowledgeRunMetrics, OrbitError, PolicyDef,
    ReviewThread, StoredTool, Task, TaskArtifact, TaskComment, TaskComplexity, TaskHistoryEntry,
    TaskPriority, TaskStatus, TaskType,
};
use orbit_store::{
    AuditEventFilter, AuditEventInsertParams, AuditEventStoreBackend, ExecutorDefStoreBackend,
    JobRunQuery, JobRunStepParams, JobRunStoreBackend, PolicyDefStoreBackend,
    TaskArtifactStoreBackend, TaskArtifactUpdateParams, TaskCreateParams, TaskDocumentStoreBackend,
    TaskDocumentUpdateParams, TaskHistoryStoreBackend, TaskHistoryUpdateParams,
    TaskReservationCheckParams, TaskReservationCheckResult, TaskReservationListResult,
    TaskReservationReleaseParams, TaskReservationReleaseResult, TaskReservationReserveParams,
    TaskReservationReserveResult, TaskReservationStoreBackend, TaskReviewStoreBackend,
    TaskReviewUpdateParams, TaskStoreBackend, ToolStoreBackend,
};

use crate::context::OrbitStores;

#[derive(Default, Clone)]
pub(crate) struct TaskRecordUpdateParams {
    pub(crate) actor: String,
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) acceptance_criteria: Option<Vec<String>>,
    pub(crate) dependencies: Option<Vec<String>>,
    pub(crate) plan: Option<String>,
    pub(crate) execution_summary: Option<String>,
    pub(crate) context_files: Option<Vec<String>>,
    pub(crate) workspace_path: Option<Option<String>>,
    pub(crate) repo_root: Option<Option<String>>,
    pub(crate) created_by: Option<Option<String>>,
    pub(crate) planned_by: Option<Option<String>>,
    pub(crate) implemented_by: Option<Option<String>>,
    pub(crate) agent: Option<Option<String>>,
    pub(crate) model: Option<Option<String>>,
    pub(crate) status: Option<TaskStatus>,
    pub(crate) priority: Option<TaskPriority>,
    pub(crate) complexity: Option<TaskComplexity>,
    pub(crate) task_type: Option<TaskType>,
    pub(crate) pr_number: Option<Option<String>>,
    pub(crate) pr_status: Option<Option<String>>,
    pub(crate) source_task_id: Option<Option<String>>,
    pub(crate) batch_id: Option<Option<String>>,
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
            || self.plan.is_some()
            || self.execution_summary.is_some()
            || self.context_files.is_some()
            || self.workspace_path.is_some()
            || self.repo_root.is_some()
            || self.created_by.is_some()
            || self.planned_by.is_some()
            || self.implemented_by.is_some()
            || self.agent.is_some()
            || self.model.is_some()
            || self.priority.is_some()
            || self.complexity.is_some()
            || self.task_type.is_some()
            || self.pr_number.is_some()
            || self.pr_status.is_some()
            || self.source_task_id.is_some()
            || self.batch_id.is_some()
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
}

pub(crate) struct TaskReservationRecords<'a> {
    store: &'a dyn TaskReservationStoreBackend,
}

impl TaskRecords<'_> {
    pub(crate) fn create(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        self.store.create_task(params)
    }

    pub(crate) fn get(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        self.store.get_task(id)
    }

    pub(crate) fn get_artifacts(&self, id: &str) -> Result<Option<Vec<TaskArtifact>>, OrbitError> {
        self.artifact.get_task_artifacts(id)
    }

    pub(crate) fn list(&self) -> Result<Vec<Task>, OrbitError> {
        self.store.list_tasks()
    }

    pub(crate) fn list_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.store
            .list_tasks_filtered(status, priority, parent_id, batch_id)
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
                    plan: params.plan.clone(),
                    execution_summary: params.execution_summary.clone(),
                    context_files: params.context_files.clone(),
                    workspace_path: params.workspace_path.clone(),
                    repo_root: params.repo_root.clone(),
                    created_by: params.created_by.clone(),
                    planned_by: params.planned_by.clone(),
                    implemented_by: params.implemented_by.clone(),
                    agent: params.agent.clone(),
                    model: params.model.clone(),
                    priority: params.priority,
                    complexity: params.complexity,
                    task_type: params.task_type,
                    pr_number: params.pr_number.clone(),
                    pr_status: params.pr_status.clone(),
                    source_task_id: params.source_task_id.clone(),
                    batch_id: params.batch_id.clone(),
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
                    upsert_artifacts: params.upsert_artifacts.clone(),
                },
            )?;
        }

        self.get(id)?
            .ok_or_else(|| OrbitError::TaskNotFound(id.to_string()))
    }

    pub(crate) fn delete(&self, id: &str) -> Result<bool, OrbitError> {
        self.store.delete_task(id)
    }

    pub(crate) fn search(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.store.search_tasks(query)
    }
}

impl TaskReservationRecords<'_> {
    pub(crate) fn list_active(
        &self,
        workspace_orbit_dir: &str,
    ) -> Result<TaskReservationListResult, OrbitError> {
        self.store
            .list_active_task_reservations(workspace_orbit_dir)
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
