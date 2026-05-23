use std::path::PathBuf;
use std::sync::Arc;

use super::contracts::{
    AdrStoreBackend, AuditEventStoreBackend, ExecutorDefStoreBackend, JobRunStoreBackend,
    LearningStoreBackend, PolicyDefStoreBackend, SessionLearningStateStoreBackend,
    TaskArtifactStoreBackend, TaskDocumentStoreBackend, TaskHistoryStoreBackend,
    TaskReservationStoreBackend, TaskReviewStoreBackend, TaskStoreBackend, ToolStoreBackend,
    V2AuditEnvelopeStoreBackend,
};
use super::layered_policy_def::LayeredPolicyDefStore;
use super::sqlite_backends::{
    SqliteAuditEventStoreBackend, SqliteSessionLearningStateStoreBackend,
    SqliteTaskReservationStoreBackend, SqliteToolStoreBackend, SqliteV2AuditEnvelopeStoreBackend,
};
use crate::file::adr_store::AdrFileStore;
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
    pub review: Arc<dyn TaskReviewStoreBackend>,
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
        review: store.clone(),
        artifact: store,
    }
}

pub fn workspace_job_run_store(
    store: Store,
    workspace_id: impl Into<String>,
) -> Arc<dyn JobRunStoreBackend> {
    Arc::new(SqliteJobRunStore::new(store, workspace_id))
}

/// Constructs the workspace-scoped ADR store backed by `adr_dir` on disk and
/// indexed in the shared SQLite `store`. The returned `Arc<dyn AdrStoreBackend>`
/// is the trait-object surface consumed by `orbit-tools::orbit.adr.*` once
/// T20260511-2 wires it through `orbit-core`.
pub fn workspace_adr_backends(
    adr_dir: PathBuf,
    store: Store,
    id_allocator: IdAllocator,
) -> Arc<dyn AdrStoreBackend> {
    Arc::new(AdrFileStore::new_with_index_and_allocator(
        adr_dir,
        store,
        id_allocator,
    ))
}

/// Constructs the workspace-scoped project-learnings store backed by
/// `learning_dir` on disk and indexed in the shared SQLite `store`. The
/// returned `Arc<dyn LearningStoreBackend>` is the trait-object surface that
/// `orbit-tools::orbit.learning.*` consumes in C2.
pub fn workspace_learning_backend(
    learning_dir: PathBuf,
    store: Store,
    id_allocator: IdAllocator,
) -> Result<Arc<dyn LearningStoreBackend>, orbit_common::types::OrbitError> {
    LearningFileStore::reject_legacy_flat_layout(&learning_dir)?;
    Ok(Arc::new(LearningFileStore::new_with_index_and_allocator(
        learning_dir,
        store,
        id_allocator,
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

pub fn v2_audit_event_store_sqlite(store: Store) -> Arc<dyn V2AuditEnvelopeStoreBackend> {
    Arc::new(SqliteV2AuditEnvelopeStoreBackend { store })
}

pub fn session_learning_state_store_sqlite(
    store: Store,
) -> Arc<dyn SessionLearningStateStoreBackend> {
    Arc::new(SqliteSessionLearningStateStoreBackend { store })
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
