use orbit_types::{
    OrbitId, ReviewThread, TaskArtifact, TaskComplexity, TaskPriority, TaskStatus, TaskType,
};

use crate::runtime::TaskRecordUpdateParams;

#[derive(Clone)]
pub struct TaskAddParams {
    pub parent_id: Option<OrbitId>,
    pub title: String,
    pub description: String,
    pub acceptance_criteria: Vec<String>,
    pub plan: String,
    pub comment: Option<String>,
    pub context_files: Vec<String>,
    pub workspace_path: Option<String>,
    pub priority: TaskPriority,
    pub complexity: Option<TaskComplexity>,
    pub task_type: TaskType,
    /// When true, the task metadata attributes creation to `system`.
    /// Used for auto-generated tasks such as job failure follow-ups.
    pub system_created: bool,
    pub source_task_id: Option<String>,
}

impl Default for TaskAddParams {
    fn default() -> Self {
        Self {
            parent_id: None,
            title: String::new(),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            plan: String::new(),
            comment: None,
            context_files: Vec::new(),
            workspace_path: None,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Task,
            system_created: false,
            source_task_id: None,
        }
    }
}

#[derive(Default, Clone)]
pub struct TaskUpdateParams {
    pub title: Option<String>,
    pub description: Option<String>,
    pub acceptance_criteria: Option<Vec<String>>,
    pub plan: Option<String>,
    pub execution_summary: Option<String>,
    pub comment: Option<String>,
    pub status: Option<TaskStatus>,
    pub pr_number: Option<Option<String>>,
    pub pr_status: Option<Option<String>>,
    pub batch_id: Option<Option<String>>,
    pub context_files: Option<Vec<String>>,
    pub upsert_artifacts: Vec<TaskArtifact>,
    pub append_review_threads: Vec<ReviewThread>,
}

impl From<TaskUpdateParams> for TaskRecordUpdateParams {
    fn from(p: TaskUpdateParams) -> Self {
        Self {
            title: p.title,
            description: p.description,
            acceptance_criteria: p.acceptance_criteria,
            plan: p.plan,
            execution_summary: p.execution_summary,
            status: p.status,
            pr_number: p.pr_number,
            pr_status: p.pr_status,
            batch_id: p.batch_id,
            context_files: p.context_files,
            upsert_artifacts: p.upsert_artifacts,
            append_review_threads: p.append_review_threads,
            ..Default::default()
        }
    }
}
