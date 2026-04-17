use std::path::PathBuf;
use std::sync::Arc;

use orbit_types::OrbitError;

use super::contracts::{
    ActivityStoreBackend, AuditEventStoreBackend, ExecutorDefStoreBackend, JobStoreBackend,
    PolicyDefStoreBackend, TaskStoreBackend, ToolStoreBackend,
};
use super::layered_executor_def::LayeredExecutorDefStore;
use super::layered_policy_def::LayeredPolicyDefStore;
use super::sqlite_backends::{SqliteAuditEventStoreBackend, SqliteToolStoreBackend};
use crate::Store;
use crate::file::activity_store::ActivityFileStore;
use crate::file::executor_def_store::ExecutorDefFileStore;
use crate::file::job_store::JobFileStore;
use crate::file::policy_def_store::PolicyDefFileStore;
use crate::file::task_store::TaskFileStore;

/// The resolved store path(s) after applying scope resolution rules.
#[derive(Debug, Clone)]
pub enum ResolvedScope {
    /// Use a single path for the store.
    Single(PathBuf),
    /// Merge two stores: workspace shadows global by key.
    Layered { global: PathBuf, workspace: PathBuf },
}

impl ResolvedScope {
    /// Extract the single path, panicking if this is a `Layered` scope.
    /// Use only when the resolution strategy guarantees `Single` (e.g. `GlobalOnly`).
    pub fn into_single(self) -> PathBuf {
        match self {
            Self::Single(path) => path,
            Self::Layered { .. } => {
                panic!("expected Single scope, got Layered")
            }
        }
    }
}

pub fn task_store_file(root: PathBuf) -> Arc<dyn TaskStoreBackend> {
    Arc::new(TaskFileStore::new(root))
}

pub fn activity_store_file(root: PathBuf) -> Arc<dyn ActivityStoreBackend> {
    Arc::new(ActivityFileStore::new(root))
}

pub fn job_store_file(root: PathBuf) -> Arc<dyn JobStoreBackend> {
    Arc::new(JobFileStore::new(root))
}

pub fn executor_def_store_file(root: PathBuf) -> Arc<dyn ExecutorDefStoreBackend> {
    Arc::new(ExecutorDefFileStore::new(root))
}

pub fn tool_store_sqlite(store: Store) -> Arc<dyn ToolStoreBackend> {
    Arc::new(SqliteToolStoreBackend { store })
}

pub fn audit_event_store_sqlite(store: Store) -> Arc<dyn AuditEventStoreBackend> {
    Arc::new(SqliteAuditEventStoreBackend { store })
}

/// Creates a task store from a resolved scope. Tasks only support `Single`.
pub fn task_store_resolved(scope: ResolvedScope) -> Result<Arc<dyn TaskStoreBackend>, OrbitError> {
    match scope {
        ResolvedScope::Single(path) => Ok(task_store_file(path)),
        ResolvedScope::Layered { .. } => Err(OrbitError::InvalidInput(
            "task store does not support layered resolution".to_string(),
        )),
    }
}

/// Creates an activity store from a resolved scope. Runtime scoping uses `Single`
/// because activities are globally scoped artifacts.
pub fn activity_store_resolved(
    scope: ResolvedScope,
) -> Result<Arc<dyn ActivityStoreBackend>, OrbitError> {
    match scope {
        ResolvedScope::Single(path) => Ok(activity_store_file(path)),
        ResolvedScope::Layered { .. } => Err(OrbitError::InvalidInput(
            "activity store does not support layered resolution".to_string(),
        )),
    }
}

/// Creates a job store from a resolved scope. Runtime scoping uses `Single`
/// because jobs are globally scoped artifacts.
pub fn job_store_resolved(scope: ResolvedScope) -> Result<Arc<dyn JobStoreBackend>, OrbitError> {
    match scope {
        ResolvedScope::Single(path) => Ok(job_store_file(path)),
        ResolvedScope::Layered { .. } => Err(OrbitError::InvalidInput(
            "job store does not support layered resolution".to_string(),
        )),
    }
}

pub fn policy_def_store_file(root: PathBuf) -> Arc<dyn PolicyDefStoreBackend> {
    Arc::new(PolicyDefFileStore::new(root))
}

pub fn executor_def_store_resolved(
    scope: ResolvedScope,
) -> Result<Arc<dyn ExecutorDefStoreBackend>, OrbitError> {
    match scope {
        ResolvedScope::Single(path) => Ok(executor_def_store_file(path)),
        ResolvedScope::Layered { global, workspace } => {
            let g = executor_def_store_file(global);
            let w = executor_def_store_file(workspace);
            Ok(Arc::new(LayeredExecutorDefStore::new(w, g)))
        }
    }
}

pub fn policy_def_store_resolved(
    scope: ResolvedScope,
) -> Result<Arc<dyn PolicyDefStoreBackend>, OrbitError> {
    match scope {
        ResolvedScope::Single(path) => Ok(policy_def_store_file(path)),
        ResolvedScope::Layered { global, workspace } => {
            let g = policy_def_store_file(global);
            let w = policy_def_store_file(workspace);
            Ok(Arc::new(LayeredPolicyDefStore::new(w, g)))
        }
    }
}
