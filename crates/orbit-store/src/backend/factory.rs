use std::path::PathBuf;
use std::sync::Arc;

use super::contracts::{
    ActivityStoreBackend, AuditEventStoreBackend, ExecutorDefStoreBackend,
    JobDefinitionStoreBackend, JobRunStoreBackend, PolicyDefStoreBackend, TaskArtifactStoreBackend,
    TaskDocumentStoreBackend, TaskHistoryStoreBackend, TaskReservationStoreBackend,
    TaskReviewStoreBackend, TaskStoreBackend, ToolStoreBackend,
};
use super::layered_policy_def::LayeredPolicyDefStore;
use super::sqlite_backends::{
    SqliteAuditEventStoreBackend, SqliteTaskReservationStoreBackend, SqliteToolStoreBackend,
};
use crate::Store;
use crate::file::activity_store::ActivityFileStore;
use crate::file::executor_def_store::ExecutorDefFileStore;
use crate::file::job_store::JobFileStore;
use crate::file::policy_def_store::PolicyDefFileStore;
use crate::file::task_store::TaskFileStore;

pub struct WorkspaceTaskBackends {
    pub task: Arc<dyn TaskStoreBackend>,
    pub document: Arc<dyn TaskDocumentStoreBackend>,
    pub history: Arc<dyn TaskHistoryStoreBackend>,
    pub review: Arc<dyn TaskReviewStoreBackend>,
    pub artifact: Arc<dyn TaskArtifactStoreBackend>,
}

pub fn workspace_task_backends(root: PathBuf) -> WorkspaceTaskBackends {
    let store = Arc::new(TaskFileStore::new(root));
    WorkspaceTaskBackends {
        task: store.clone(),
        document: store.clone(),
        history: store.clone(),
        review: store.clone(),
        artifact: store,
    }
}

pub fn global_activity_store(root: PathBuf) -> Arc<dyn ActivityStoreBackend> {
    Arc::new(ActivityFileStore::new(root))
}

pub struct ScopedJobBackends {
    pub definition: Arc<dyn JobDefinitionStoreBackend>,
    pub run: Arc<dyn JobRunStoreBackend>,
}

pub fn scoped_job_backends(global_root: PathBuf, workspace_root: PathBuf) -> ScopedJobBackends {
    ScopedJobBackends {
        definition: Arc::new(JobFileStore::new(global_root)),
        run: Arc::new(JobFileStore::new(workspace_root)),
    }
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
