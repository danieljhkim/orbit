use std::path::PathBuf;
use std::sync::Arc;

use super::contracts::{
    AuditEventStoreBackend, ExecutorDefStoreBackend, JobRunStoreBackend, LearningStoreBackend,
    PolicyDefStoreBackend, TaskArtifactStoreBackend, TaskDocumentStoreBackend,
    TaskHistoryStoreBackend, TaskReservationStoreBackend, TaskStoreBackend, ToolStoreBackend,
};
use super::layered_policy_def::LayeredPolicyDefStore;
use super::sqlite_backends::{
    SqliteAuditEventStoreBackend, SqliteTaskReservationStoreBackend, SqliteToolStoreBackend,
};
use crate::file::executor_def_store::ExecutorDefFileStore;
use crate::file::learning_store::LearningFileStore;
use crate::file::policy_def_store::PolicyDefFileStore;
use crate::file::task_store::TaskV2Store;
use crate::sqlite::job_run_store::SqliteJobRunStore;
use crate::sqlite::task_registry::TaskRegistryStore;
use crate::{IdAllocator, Store};

pub struct WorkspaceTaskBackends {
    pub task: Arc<dyn TaskStoreBackend>,
    pub document: Arc<dyn TaskDocumentStoreBackend>,
    pub history: Arc<dyn TaskHistoryStoreBackend>,
    pub artifact: Arc<dyn TaskArtifactStoreBackend>,
}

pub fn workspace_task_backends(
    registry: TaskRegistryStore,
    workspace_id: String,
    workspace_orbit_dir: PathBuf,
    workspace_path: Option<String>,
    repo_root: Option<String>,
) -> WorkspaceTaskBackends {
    let store = Arc::new(TaskV2Store::new(
        registry,
        workspace_id,
        workspace_orbit_dir,
        workspace_path,
        repo_root,
    ));
    WorkspaceTaskBackends {
        task: store.clone(),
        document: store.clone(),
        history: store.clone(),
        artifact: store,
    }
}

/// Constructs coordination-only task backends for a logical workspace that
/// has no checkout on this machine. Canonical bundles and registry indexes
/// remain available; checkout-local projections are intentionally omitted.
pub fn coordination_task_backends(
    registry: TaskRegistryStore,
    workspace_id: String,
) -> WorkspaceTaskBackends {
    let store = Arc::new(TaskV2Store::new_checkoutless(registry, workspace_id));
    WorkspaceTaskBackends {
        task: store.clone(),
        document: store.clone(),
        history: store.clone(),
        artifact: store,
    }
}

pub fn workspace_job_run_store(
    store: Store,
    workspace_id: impl Into<String>,
) -> Arc<dyn JobRunStoreBackend> {
    Arc::new(SqliteJobRunStore::new(store, workspace_id))
}

/// Constructs the workspace-scoped project-learnings store backed by
/// `learning_dir` on disk and indexed in the shared SQLite `store`. The
/// returned `Arc<dyn LearningStoreBackend>` is the trait-object surface that
/// `orbit-tools::orbit.learning.*` consumes in C2.
pub fn workspace_learning_backend(
    learning_dir: PathBuf,
    store: Store,
    id_allocator: IdAllocator,
    workspace_id: String,
) -> Result<Arc<dyn LearningStoreBackend>, orbit_common::types::OrbitError> {
    LearningFileStore::reject_legacy_flat_layout(&learning_dir)?;
    Ok(Arc::new(LearningFileStore::new_with_index_and_allocator(
        learning_dir,
        store,
        id_allocator,
        workspace_id,
    )))
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

#[cfg(test)]
#[cfg(test)]
mod tests;
