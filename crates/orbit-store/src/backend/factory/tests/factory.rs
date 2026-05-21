// Migrated from backend/factory.rs per ORB-00231
use orbit_common::types::{TaskPriority, TaskStatus, TaskType};
use tempfile::TempDir;

use super::super::*;
use crate::backend::TaskCreateParams;
use crate::sqlite::task_registry::{BindWorkspaceParams, TaskRegistryStore, task_registry_path};

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
            relations: Vec::new(),
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
            crew: None,
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

#[test]
fn workspace_learning_backend_rejects_legacy_flat_layout() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("learnings");
    std::fs::create_dir_all(&root).expect("create learnings");
    std::fs::write(root.join("L-0001.yaml"), "").expect("legacy learning");
    let store = Store::open_in_memory().expect("open store");

    let id_allocator =
        IdAllocator::for_test_roots(temp.path().join("adrs"), temp.path().join("learnings2"));
    let err = match workspace_learning_backend(root, store, id_allocator) {
        Ok(_) => panic!("legacy rejected"),
        Err(err) => err,
    };

    assert!(matches!(err, orbit_common::types::OrbitError::Migration(_)));
    assert!(err.to_string().contains("orbit learning migrate-layout"));
}
