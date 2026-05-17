use orbit_common::types::{
    ExternalRef, OrbitId, ReviewThread, TaskArtifact, TaskComplexity, TaskPriority, TaskRelation,
    TaskStatus, TaskType,
};

use crate::runtime::TaskRecordUpdateParams;

#[derive(Clone)]
pub struct TaskAddParams {
    pub parent_id: Option<OrbitId>,
    pub title: String,
    pub description: String,
    pub acceptance_criteria: Vec<String>,
    pub dependencies: Vec<OrbitId>,
    pub relations: Vec<TaskRelation>,
    pub tags: Vec<String>,
    pub plan: String,
    pub comment: Option<String>,
    pub context_files: Vec<String>,
    pub workspace_path: Option<String>,
    pub priority: TaskPriority,
    pub complexity: Option<TaskComplexity>,
    pub task_type: Option<TaskType>,
    pub status: Option<TaskStatus>,
    /// When true, the task metadata attributes creation to `system`.
    /// Used for auto-generated tasks such as job failure follow-ups.
    pub system_created: bool,
    pub external_refs: Vec<ExternalRef>,
    pub source_task_id: Option<String>,
    pub crew: Option<String>,
}

impl Default for TaskAddParams {
    fn default() -> Self {
        Self {
            parent_id: None,
            title: String::new(),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            dependencies: Vec::new(),
            relations: Vec::new(),
            tags: Vec::new(),
            plan: String::new(),
            comment: None,
            context_files: Vec::new(),
            workspace_path: None,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: None,
            status: None,
            system_created: false,
            external_refs: Vec::new(),
            source_task_id: None,
            crew: None,
        }
    }
}

#[derive(Default, Clone)]
pub struct TaskUpdateParams {
    pub title: Option<String>,
    pub description: Option<String>,
    pub acceptance_criteria: Option<Vec<String>>,
    pub dependencies: Option<Vec<OrbitId>>,
    pub relations: Option<Vec<TaskRelation>>,
    pub tags: Option<Vec<String>>,
    pub plan: Option<String>,
    pub execution_summary: Option<String>,
    pub comment: Option<String>,
    pub status: Option<TaskStatus>,
    pub task_type: Option<TaskType>,
    pub source_task_id: Option<Option<String>>,
    pub planned_by: Option<Option<String>>,
    pub implemented_by: Option<Option<String>>,
    pub pr_status: Option<Option<String>>,
    pub job_run_id: Option<Option<String>>,
    pub crew: Option<Option<String>>,
    pub context_files: Option<Vec<String>>,
    pub upsert_artifacts: Vec<TaskArtifact>,
    pub append_review_threads: Vec<ReviewThread>,
}

impl TaskUpdateParams {
    pub(crate) fn has_comment_update(&self) -> bool {
        self.comment.is_some()
    }

    pub(crate) fn has_non_comment_mutation(&self) -> bool {
        self.title.is_some()
            || self.description.is_some()
            || self.acceptance_criteria.is_some()
            || self.dependencies.is_some()
            || self.relations.is_some()
            || self.tags.is_some()
            || self.plan.is_some()
            || self.execution_summary.is_some()
            || self.status.is_some()
            || self.task_type.is_some()
            || self.source_task_id.is_some()
            || self.planned_by.is_some()
            || self.implemented_by.is_some()
            || self.pr_status.is_some()
            || self.job_run_id.is_some()
            || self.crew.is_some()
            || self.context_files.is_some()
            || !self.upsert_artifacts.is_empty()
            || !self.append_review_threads.is_empty()
    }

    pub(crate) fn has_any_mutation(&self) -> bool {
        self.has_comment_update() || self.has_non_comment_mutation()
    }
}

impl From<TaskUpdateParams> for TaskRecordUpdateParams {
    fn from(p: TaskUpdateParams) -> Self {
        Self {
            title: p.title,
            description: p.description,
            acceptance_criteria: p.acceptance_criteria,
            dependencies: p.dependencies,
            relations: p.relations,
            tags: p.tags,
            plan: p.plan,
            execution_summary: p.execution_summary,
            status: p.status,
            task_type: p.task_type,
            source_task_id: p.source_task_id,
            planned_by: p.planned_by,
            implemented_by: p.implemented_by,
            pr_status: p.pr_status,
            job_run_id: p.job_run_id,
            crew: p.crew,
            context_files: p.context_files,
            upsert_artifacts: p.upsert_artifacts,
            append_review_threads: p.append_review_threads,
            ..Default::default()
        }
    }
}
