use orbit_common::types::{
    ArtifactManifestV2, ReviewThreadMetadataV2, TASK_ACCEPTANCE_FILE_NAME,
    TASK_DESCRIPTION_FILE_NAME, TASK_EXECUTION_SUMMARY_FILE_NAME, TASK_PLAN_FILE_NAME,
    TaskCommentRowV2, TaskEnvelopeV2, TaskEventRowV2,
};

use crate::sqlite::task_registry::{ProjectionRebuildResult, TaskBundleBinding};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskBundleV2 {
    pub(crate) envelope: TaskEnvelopeV2,
    pub(crate) description: String,
    pub(crate) acceptance: String,
    pub(crate) plan: String,
    pub(crate) execution_summary: String,
    pub(crate) events: Vec<TaskEventRowV2>,
    pub(crate) comments: Vec<TaskCommentRowV2>,
    pub(crate) review_threads: Vec<TaskReviewThreadV2>,
    pub(crate) artifact_manifest: Option<ArtifactManifestV2>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskReviewThreadV2 {
    pub(crate) metadata: ReviewThreadMetadataV2,
    pub(crate) body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskDocumentV2 {
    Description,
    Acceptance,
    Plan,
    ExecutionSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskBundleCreateResult {
    pub(crate) binding: TaskBundleBinding,
    pub(crate) projection: ProjectionRebuildResult,
}

impl TaskDocumentV2 {
    pub(crate) fn file_name(self) -> &'static str {
        match self {
            Self::Description => TASK_DESCRIPTION_FILE_NAME,
            Self::Acceptance => TASK_ACCEPTANCE_FILE_NAME,
            Self::Plan => TASK_PLAN_FILE_NAME,
            Self::ExecutionSummary => TASK_EXECUTION_SUMMARY_FILE_NAME,
        }
    }
}
