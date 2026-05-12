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
use crate::file::task_store::TaskV2Store;
use crate::sqlite::task_registry::TaskRegistryStore;

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

#[cfg(test)]
mod tests {
    use orbit_common::types::{TaskPriority, TaskStatus, TaskType};
    use tempfile::TempDir;

    use super::*;
    use crate::backend::TaskCreateParams;
    use crate::sqlite::task_registry::{
        BindWorkspaceParams, TaskRegistryStore, task_registry_path,
    };

    #[test]
    fn workspace_task_backends_exposes_create_get_and_list_trait_surface() {
        let temp = TempDir::new().expect("tempdir");
        let registry =
            TaskRegistryStore::open(&task_registry_path(temp.path())).expect("open registry");
        let repo_dir = temp.path().join("repo");
        let orbit_dir = repo_dir.join(".orbit");
        std::fs::create_dir_all(&orbit_dir).expect("create orbit dir");
        let binding = registry
            .bind_workspace(BindWorkspaceParams {
                workspace_id: Some("orbit-test-123456".to_string()),
                slug: "Orbit Test".to_string(),
                repo_root: repo_dir.clone(),
                workspace_path: repo_dir.clone(),
                orbit_dir: orbit_dir.clone(),
                repo_fingerprint: None,
            })
            .expect("bind workspace");
        let backends = workspace_task_backends(
            registry,
            binding.workspace_id,
            orbit_dir,
            Some(repo_dir.to_string_lossy().into_owned()),
            Some(repo_dir.to_string_lossy().into_owned()),
        );

        let created = backends
            .task
            .create_task(TaskCreateParams {
                actor: "codex:gpt-5.5".to_string(),
                parent_id: None,
                title: "Trait-created v2 task".to_string(),
                description: "A task created through the trait surface.".to_string(),
                acceptance_criteria: vec!["Round trip through trait backend".to_string()],
                dependencies: Vec::new(),
                tags: vec!["task-artifacts".to_string()],
                plan: "1. Exercise backend".to_string(),
                execution_summary: String::new(),
                context_files: Vec::new(),
                workspace_path: None,
                repo_root: None,
                created_by: Some("codex:gpt-5.5".to_string()),
                planned_by: None,
                implemented_by: None,
                status: TaskStatus::Backlog,
                priority: TaskPriority::Medium,
                complexity: None,
                task_type: TaskType::Feature,
                external_refs: Vec::new(),
                source_task_id: None,
                comments: Vec::new(),
            })
            .expect("create task");

        assert_eq!(created.id, "ORB-00000");
        assert_eq!(
            backends
                .task
                .get_task("ORB-00000")
                .expect("get task")
                .expect("task exists")
                .title,
            "Trait-created v2 task"
        );
        assert_eq!(backends.task.list_tasks().expect("list tasks").len(), 1);
    }
}
