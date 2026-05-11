use std::path::PathBuf;
use std::sync::Arc;

use super::contracts::{
    AdrStoreBackend, AuditEventStoreBackend, ExecutorDefStoreBackend, JobRunStoreBackend,
    LearningStoreBackend, PolicyDefStoreBackend, TaskArtifactStoreBackend,
    TaskDocumentStoreBackend, TaskHistoryStoreBackend, TaskReservationStoreBackend,
    TaskReviewStoreBackend, TaskStoreBackend, ToolStoreBackend,
};
use super::layered_policy_def::LayeredPolicyDefStore;
use super::sqlite_backends::{
    SqliteAuditEventStoreBackend, SqliteTaskReservationStoreBackend, SqliteToolStoreBackend,
};
use crate::Store;
use crate::file::adr_store::AdrFileStore;
use crate::file::executor_def_store::ExecutorDefFileStore;
use crate::file::job_store::JobFileStore;
use crate::file::learning_store::LearningFileStore;
use crate::file::policy_def_store::PolicyDefFileStore;
use crate::file::task_store::TaskFileStore;

pub struct WorkspaceTaskBackends {
    pub task: Arc<dyn TaskStoreBackend>,
    pub document: Arc<dyn TaskDocumentStoreBackend>,
    pub history: Arc<dyn TaskHistoryStoreBackend>,
    pub review: Arc<dyn TaskReviewStoreBackend>,
    pub artifact: Arc<dyn TaskArtifactStoreBackend>,
}

pub fn workspace_task_backends(root: PathBuf, task_index: Store) -> WorkspaceTaskBackends {
    let store = Arc::new(TaskFileStore::new_with_index(root, task_index));
    WorkspaceTaskBackends {
        task: store.clone(),
        document: store.clone(),
        history: store.clone(),
        review: store.clone(),
        artifact: store,
    }
}

pub fn workspace_job_run_store(root: PathBuf) -> Arc<dyn JobRunStoreBackend> {
    Arc::new(JobFileStore::new(root))
}

/// Constructs the workspace-scoped ADR store backed by `adr_dir` on disk and
/// indexed in the shared SQLite `store`. The returned `Arc<dyn AdrStoreBackend>`
/// is the trait-object surface consumed by `orbit-tools::orbit.adr.*` once
/// T20260511-2 wires it through `orbit-core`.
pub fn workspace_adr_backends(adr_dir: PathBuf, store: Store) -> Arc<dyn AdrStoreBackend> {
    Arc::new(AdrFileStore::new_with_index(adr_dir, store))
}

/// Constructs the workspace-scoped project-learnings store backed by
/// `learning_dir` on disk and indexed in the shared SQLite `store`. The
/// returned `Arc<dyn LearningStoreBackend>` is the trait-object surface that
/// `orbit-tools::orbit.learning.*` consumes in C2.
pub fn workspace_learning_backend(
    learning_dir: PathBuf,
    store: Store,
) -> Arc<dyn LearningStoreBackend> {
    Arc::new(LearningFileStore::new_with_index(learning_dir, store))
}

pub fn global_executor_def_store(root: PathBuf) -> Arc<dyn ExecutorDefStoreBackend> {
    Arc::new(ExecutorDefFileStore::new(root))
}

pub fn tool_store_sqlite(store: Store) -> Arc<dyn ToolStoreBackend> {
    Arc::new(SqliteToolStoreBackend { store })
}

pub fn audit_event_store_sqlite(store: Store) -> Arc<dyn AuditEventStoreBackend> {
    Arc::new(SqliteAuditEventStoreBackend { store })
}

pub fn task_reservation_store_sqlite(store: Store) -> Arc<dyn TaskReservationStoreBackend> {
    Arc::new(SqliteTaskReservationStoreBackend { store })
}

pub fn global_policy_def_store(root: PathBuf) -> Arc<dyn PolicyDefStoreBackend> {
    Arc::new(PolicyDefFileStore::new(root))
}

pub fn workspace_policy_def_store(root: PathBuf) -> Arc<dyn PolicyDefStoreBackend> {
    Arc::new(PolicyDefFileStore::new(root))
}

pub fn layered_policy_def_store(
    workspace: Arc<dyn PolicyDefStoreBackend>,
    global: Arc<dyn PolicyDefStoreBackend>,
) -> Arc<dyn PolicyDefStoreBackend> {
    Arc::new(LayeredPolicyDefStore::new(workspace, global))
}
