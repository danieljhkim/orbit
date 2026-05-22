//! Sibling tests for `builder.rs` (migrated per ORB-00246 / docs/design-patterns/test_layout.md).

use std::path::PathBuf;

use orbit_store::sqlite::task_registry::read_workspace_config_optional;

use crate::OrbitError;

use orbit_common::types::{NotFoundKind, TaskStatus};
use tempfile::tempdir;

use crate::OrbitRuntime;
use crate::command::task::{TaskAddParams, TaskUpdateParams};

fn v2_runtime() -> (tempfile::TempDir, PathBuf, PathBuf, OrbitRuntime) {
    let root = tempdir().expect("tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build runtime");
    (root, global_root, workspace_root, runtime)
}

#[test]
fn v2_task_backend_wires_through_runtime_add_show_list_and_update() {
    let (_root, _global_root, workspace_root, runtime) = v2_runtime();

    let task = runtime
        .add_task(TaskAddParams {
            title: "Runtime v2 task".to_string(),
            description: "Created through OrbitRuntime".to_string(),
            plan: "1. Start it".to_string(),
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("create task");
    assert_eq!(task.id, "ORB-00000");
    assert!(!workspace_root.join("tasks/backlog").exists());
    assert!(workspace_root.join("tasks/ORB-00000").exists());

    let started = runtime
        .start_task(&task.id, Some("start".to_string()), None)
        .expect("start task");
    assert_eq!(started.status, TaskStatus::InProgress);

    let updated = runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                comment: Some("Runtime comment".to_string()),
                execution_summary: Some("Finished the runtime smoke".to_string()),
                status: Some(TaskStatus::Review),
                ..Default::default()
            },
        )
        .expect("update task");
    assert_eq!(updated.status, TaskStatus::Review);
    assert!(
        runtime
            .get_task_comments(&task.id)
            .expect("read task comments")
            .iter()
            .any(|comment| comment.message == "Runtime comment")
    );
    assert_eq!(runtime.list_tasks().expect("list tasks").len(), 1);
    assert_eq!(
        runtime
            .search_tasks("runtime smoke")
            .expect("search tasks")
            .len(),
        1
    );

    runtime
        .delete_task_guarded(&updated.id, true)
        .expect("delete v2 task");
    assert!(matches!(
        runtime.get_task(&updated.id),
        Err(OrbitError::NotFound {
            kind: NotFoundKind::Task,
            ..
        })
    ));
}

#[test]
fn v2_task_backend_persists_workspace_binding_across_runtime_rebuild() {
    let (_root, global_root, workspace_root, runtime) = v2_runtime();
    let task = runtime
        .add_task(TaskAddParams {
            title: "Persistent v2 task".to_string(),
            description: "Survives runtime reconstruction".to_string(),
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("create task");
    let workspace_config =
        read_workspace_config_optional(&workspace_root).expect("read workspace config");
    let workspace_id = workspace_config
        .as_ref()
        .map(|config| config.workspace_id.as_str())
        .expect("workspace id");
    assert!(workspace_id.starts_with("repo-"), "{workspace_id}");
    assert_eq!(workspace_id.len(), "repo-000000".len());

    let rebuilt =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("rebuild runtime");
    let fetched = rebuilt.get_task(&task.id).expect("get task after rebuild");
    assert_eq!(fetched.title, "Persistent v2 task");
    assert_eq!(
        read_workspace_config_optional(&workspace_root)
            .expect("read workspace config")
            .map(|config| config.workspace_id),
        workspace_config.map(|config| config.workspace_id)
    );
}

#[test]
fn v2_task_backend_rebinds_when_workspace_config_is_missing() {
    let (_root, global_root, workspace_root, runtime) = v2_runtime();
    let task = runtime
        .add_task(TaskAddParams {
            title: "Rebind v2 task".to_string(),
            description: "Survives missing workspace config".to_string(),
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("create task");
    let original_config =
        read_workspace_config_optional(&workspace_root).expect("read workspace config");
    std::fs::remove_file(workspace_root.join("config.yaml")).expect("remove workspace config");

    let rebuilt =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("rebuild runtime");
    let fetched = rebuilt.get_task(&task.id).expect("get task after rebind");

    assert_eq!(fetched.title, "Rebind v2 task");
    assert_eq!(
        read_workspace_config_optional(&workspace_root)
            .expect("read rewritten workspace config")
            .map(|config| config.workspace_id),
        original_config.map(|config| config.workspace_id)
    );
}
