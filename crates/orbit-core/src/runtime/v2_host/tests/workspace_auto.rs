use orbit_common::types::{TaskPriority, TaskStatus, TaskType};
use orbit_engine::RuntimeHost;
use orbit_tools::ToolContext;
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::command::task::{TaskAddParams, TaskUpdateParams};
use crate::runtime::v2_host::test_support::{
    runtime_with_workspace_layout, seed_list_backlog_task,
};

fn classify(runtime: &OrbitRuntime) -> Value {
    runtime
        .run_deterministic(
            "classify_workspace_auto_tasks",
            &json!({}),
            &json!({}),
            ToolContext::default(),
        )
        .expect("classify workspace auto tasks")
}

#[test]
fn two_loose_tasks_win_before_one_epic_and_three_children() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let loose_one = seed_list_backlog_task(
        &runtime,
        "Loose high",
        TaskStatus::Backlog,
        TaskPriority::High,
        TaskType::Chore,
        None,
        vec![],
    );
    let loose_two = seed_list_backlog_task(
        &runtime,
        "Loose medium",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec![],
    );
    let epic = runtime
        .add_task(TaskAddParams {
            title: "Epic root".to_string(),
            description: "Epic fixture".to_string(),
            acceptance_criteria: vec!["Supervised".to_string()],
            tags: vec!["epic".to_string()],
            plan: "Delegate children".to_string(),
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("seed epic root");
    for index in 0..3 {
        seed_list_backlog_task(
            &runtime,
            &format!("Epic child {index}"),
            TaskStatus::Backlog,
            TaskPriority::Medium,
            TaskType::Chore,
            Some(epic.id.clone()),
            vec![],
        );
    }

    let first = classify(&runtime);
    assert_eq!(first["decision"], "ship");
    assert_eq!(first["loose_task_ids"], json!([loose_one.id, loose_two.id]));
    assert_eq!(first["epic_task_id"], Value::Null);

    for loose in [&loose_one, &loose_two] {
        runtime
            .update_task(
                &loose.id,
                TaskUpdateParams {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .expect("complete loose task");
    }
    let second = classify(&runtime);
    assert_eq!(second["decision"], "epic");
    assert_eq!(second["epic_task_id"], epic.id);
    assert_eq!(second["loose_task_ids"], json!([]));
}

#[test]
fn in_progress_epic_holds_and_empty_workspace_succeeds() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let epic = runtime
        .add_task(TaskAddParams {
            title: "Active epic".to_string(),
            description: "Epic fixture".to_string(),
            acceptance_criteria: vec!["Supervised".to_string()],
            tags: vec!["epic".to_string()],
            plan: "Delegate children".to_string(),
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        })
        .expect("seed active epic");
    seed_list_backlog_task(
        &runtime,
        "Late loose task",
        TaskStatus::Backlog,
        TaskPriority::Critical,
        TaskType::Chore,
        None,
        vec![],
    );

    let held = classify(&runtime);
    assert_eq!(held["decision"], "hold");
    assert_eq!(held["epic_task_id"], epic.id);

    let (_empty_root, empty_runtime, _empty_repo_root) = runtime_with_workspace_layout();
    let empty = classify(&empty_runtime);
    assert_eq!(empty["decision"], "empty");
    assert_eq!(empty["loose_task_ids"], json!([]));
    assert_eq!(empty["epic_task_id"], Value::Null);
}
