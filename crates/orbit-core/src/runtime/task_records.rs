use orbit_common::types::{
    ExternalRef, NotFoundKind, OrbitError, Task, TaskArtifact, TaskComment, TaskComplexity,
    TaskHistoryEntry, TaskPriority, TaskRelation, TaskStatus, TaskType,
};
use orbit_search::{EmbedWorker, VectorStore};
use orbit_store::{
    TaskArtifactStoreBackend, TaskArtifactUpdateParams, TaskCreateParams, TaskDocumentStoreBackend,
    TaskDocumentUpdateParams, TaskHistoryStoreBackend, TaskHistoryUpdateParams, TaskStoreBackend,
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
    pub(crate) orchestrator: Option<Option<String>>,
    pub(crate) status_event: Option<String>,
    pub(crate) status_note: Option<String>,
    pub(crate) append_history: Vec<TaskHistoryEntry>,
    pub(crate) append_comments: Vec<TaskComment>,
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
            || self.orchestrator.is_some()
    }

    fn has_history_changes(&self) -> bool {
        self.status.is_some()
            || self.status_event.is_some()
            || self.status_note.is_some()
            || !self.append_history.is_empty()
            || !self.append_comments.is_empty()
    }

    fn has_artifact_changes(&self) -> bool {
        !self.upsert_artifacts.is_empty()
    }
}

impl OrbitStores {
    pub(crate) fn task_records(&self) -> TaskRecordService<'_> {
        TaskRecordService {
            store: self.tasks(),
            document: self.task_documents(),
            history: self.task_history(),
            artifact: self.task_artifacts(),
            semantic_vector: self.semantic_vector(),
            semantic_worker: self.semantic_worker(),
        }
    }
}

/// Coordinates task writes that span multiple persistence and indexing services.
///
/// Read-only calls use the typed backends exposed by [`OrbitStores`] directly.
pub(crate) struct TaskRecordService<'a> {
    store: &'a dyn TaskStoreBackend,
    document: &'a dyn TaskDocumentStoreBackend,
    history: &'a dyn TaskHistoryStoreBackend,
    artifact: &'a dyn TaskArtifactStoreBackend,
    semantic_vector: &'a VectorStore,
    semantic_worker: &'a EmbedWorker,
}

impl TaskRecordService<'_> {
    pub(crate) fn create(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        let task = self.store.create_task(params)?;
        self.semantic_worker.enqueue(task.clone());
        Ok(task)
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
                    orchestrator: params.orchestrator.clone(),
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
            .store
            .get_task(id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, id.to_string()))?;
        if params.has_document_changes()
            || params.has_history_changes()
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
}
