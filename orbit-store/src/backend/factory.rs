use std::path::PathBuf;
use std::sync::Arc;

use orbit_types::OrbitError;

use super::contracts::{
    ActivityStoreBackend, AuditEventStoreBackend, JobStoreBackend, LockStoreBackend,
    TaskStoreBackend, ToolStoreBackend,
};
use super::layered_activity::LayeredActivityStore;
use super::layered_job::LayeredJobStore;
use super::memory_activity::MemoryActivityStoreBackend;
use super::memory_backends::MemoryLockStoreBackend;
use super::memory_job::MemoryJobStoreBackend;
use super::memory_task::MemoryTaskStoreBackend;
use super::sqlite_backends::{SqliteAuditEventStoreBackend, SqliteToolStoreBackend};
use crate::Store;
use crate::file::activity_store::ActivityFileStore;
use crate::file::job_store::JobFileStore;
use crate::file::task_store::TaskFileStore;

pub fn task_store_file(root: PathBuf) -> Result<Arc<dyn TaskStoreBackend>, OrbitError> {
    let store = TaskFileStore::new(root);
    store.ensure_layout()?;
    Ok(Arc::new(store))
}

pub fn activity_store_file(root: PathBuf) -> Result<Arc<dyn ActivityStoreBackend>, OrbitError> {
    let store = ActivityFileStore::new(root);
    store.ensure_layout()?;
    Ok(Arc::new(store))
}

pub fn job_store_file(root: PathBuf) -> Result<Arc<dyn JobStoreBackend>, OrbitError> {
    let store = JobFileStore::new(root);
    store.ensure_layout()?;
    Ok(Arc::new(store))
}

pub fn tool_store_sqlite(store: Store) -> Arc<dyn ToolStoreBackend> {
    Arc::new(SqliteToolStoreBackend { store })
}

pub fn audit_event_store_sqlite(store: Store) -> Arc<dyn AuditEventStoreBackend> {
    Arc::new(SqliteAuditEventStoreBackend { store })
}

pub fn lock_store_memory() -> Arc<dyn LockStoreBackend> {
    Arc::new(MemoryLockStoreBackend::default())
}

pub fn task_store_memory() -> Arc<dyn TaskStoreBackend> {
    Arc::new(MemoryTaskStoreBackend::default())
}

pub fn activity_store_memory() -> Arc<dyn ActivityStoreBackend> {
    Arc::new(MemoryActivityStoreBackend::default())
}

pub fn job_store_memory() -> Arc<dyn JobStoreBackend> {
    Arc::new(MemoryJobStoreBackend::default())
}

/// Creates a layered activity store that merges workspace and global file stores.
/// Workspace entries shadow global entries by ID. If `workspace_root` is `None`
/// or the directory doesn't exist, returns the global store directly.
pub fn activity_store_layered(
    global_root: PathBuf,
    workspace_root: Option<PathBuf>,
) -> Result<Arc<dyn ActivityStoreBackend>, OrbitError> {
    let global = activity_store_file(global_root)?;
    match workspace_root {
        Some(ws_root) if ws_root.is_dir() => {
            let workspace = activity_store_file(ws_root)?;
            Ok(Arc::new(LayeredActivityStore::new(workspace, global)))
        }
        _ => Ok(global),
    }
}

/// Creates a layered job store that merges workspace and global file stores.
/// Workspace entries shadow global entries by job ID. If `workspace_root` is `None`
/// or the directory doesn't exist, returns the global store directly.
pub fn job_store_layered(
    global_root: PathBuf,
    workspace_root: Option<PathBuf>,
) -> Result<Arc<dyn JobStoreBackend>, OrbitError> {
    let global = job_store_file(global_root)?;
    match workspace_root {
        Some(ws_root) if ws_root.is_dir() => {
            let workspace = job_store_file(ws_root)?;
            Ok(Arc::new(LayeredJobStore::new(workspace, global)))
        }
        _ => Ok(global),
    }
}
