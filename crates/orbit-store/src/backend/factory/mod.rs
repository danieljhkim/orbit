use std::path::PathBuf;
use std::sync::Arc;

use super::contracts::{
    AuditEventStoreBackend, ExecutorDefStoreBackend, JobRunStoreBackend, PolicyDefStoreBackend,
    TaskArtifactStoreBackend, TaskDocumentStoreBackend, TaskHistoryStoreBackend,
    TaskReservationStoreBackend, TaskStoreBackend, ToolStoreBackend,
};
use super::layered_policy_def::LayeredPolicyDefStore;
use super::sqlite_backends::{
    SqliteAuditEventStoreBackend, SqliteTaskReservationStoreBackend, SqliteToolStoreBackend,
};
use crate::Store;
use crate::file::executor_def_store::ExecutorDefFileStore;
use crate::file::policy_def_store::PolicyDefFileStore;
use crate::file::task_store::TaskV2Store;
use crate::sqlite::job_run_store::SqliteJobRunStore;
use crate::sqlite::task_registry::TaskRegistryStore;

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
