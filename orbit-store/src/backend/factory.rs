use std::path::PathBuf;
use std::sync::Arc;

use orbit_types::OrbitError;

use super::contracts::{
    AgentSessionStoreBackend, AuditEventStoreBackend, AuditStoreBackend, JobStoreBackend,
    LockStoreBackend, SchedulerStoreBackend, TaskStoreBackend, ToolStoreBackend, WatchStoreBackend,
};
use super::sqlite_backends::{
    SqliteAgentSessionStoreBackend, SqliteAuditEventStoreBackend, SqliteAuditStoreBackend,
    SqliteJobStoreBackend, SqliteLockStoreBackend, SqliteTaskStoreBackend, SqliteToolStoreBackend,
    SqliteWatchStoreBackend, SqliteWorkStoreBackend,
};
use crate::Store;
use crate::file::job_store::JobFileStore;
use crate::file::scheduler_store::SchedulerFileStore;
use crate::file::task_store::TaskFileStore;

pub fn task_store_file(root: PathBuf) -> Result<Arc<dyn TaskStoreBackend>, OrbitError> {
    let store = TaskFileStore::new(root);
    store.ensure_layout()?;
    Ok(Arc::new(store))
}

pub fn task_store_sqlite(store: Store) -> Arc<dyn TaskStoreBackend> {
    Arc::new(SqliteTaskStoreBackend { store })
}

pub fn job_store_file(root: PathBuf) -> Result<Arc<dyn JobStoreBackend>, OrbitError> {
    let store = JobFileStore::new(root);
    store.ensure_layout()?;
    Ok(Arc::new(store))
}

pub fn job_store_sqlite(store: Store) -> Arc<dyn JobStoreBackend> {
    Arc::new(SqliteWorkStoreBackend { store })
}

pub fn scheduler_store_file(root: PathBuf) -> Result<Arc<dyn SchedulerStoreBackend>, OrbitError> {
    let store = SchedulerFileStore::new(root);
    store.ensure_layout()?;
    Ok(Arc::new(store))
}

pub fn scheduler_store_sqlite(store: Store) -> Arc<dyn SchedulerStoreBackend> {
    Arc::new(SqliteJobStoreBackend { store })
}

pub fn tool_store_sqlite(store: Store) -> Arc<dyn ToolStoreBackend> {
    Arc::new(SqliteToolStoreBackend { store })
}

pub fn watch_store_sqlite(store: Store) -> Arc<dyn WatchStoreBackend> {
    Arc::new(SqliteWatchStoreBackend { store })
}

pub fn audit_store_sqlite(store: Store) -> Arc<dyn AuditStoreBackend> {
    Arc::new(SqliteAuditStoreBackend { store })
}

pub fn audit_event_store_sqlite(store: Store) -> Arc<dyn AuditEventStoreBackend> {
    Arc::new(SqliteAuditEventStoreBackend { store })
}

pub fn agent_session_store_sqlite(store: Store) -> Arc<dyn AgentSessionStoreBackend> {
    Arc::new(SqliteAgentSessionStoreBackend { store })
}

pub fn lock_store_sqlite(store: Store) -> Arc<dyn LockStoreBackend> {
    Arc::new(SqliteLockStoreBackend { store })
}
