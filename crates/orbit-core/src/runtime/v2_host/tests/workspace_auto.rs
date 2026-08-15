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

fn list_epic_descendants(runtime: &OrbitRuntime, epic_task_id: &str) -> Value {
    runtime
        .run_deterministic(
            "list_epic_descendants",
            &json!({}),
            &json!({ "epic_task_id": epic_task_id }),
            ToolContext::default(),
        )
        .expect("list epic descendants")
}

#[test]
fn epic_descendants_are_dependency_then_priority_ordered_and_terminal_tasks_are_skipped() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let epic = runtime
        .add_task(TaskAddParams {
            title: "Epic root".to_string(),
            description: "Epic fixture".to_string(),
            acceptance_criteria: vec!["Supervised".to_string()],
            tags: vec!["epic".to_string()],
            plan: "Delegate children".to_string(),
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        })
        .expect("seed epic root");
    let foundation = runtime
        .add_task(TaskAddParams {
            parent_id: Some(epic.id.clone()),
            title: "Foundation".to_string(),
            description: "Foundation fixture".to_string(),
            acceptance_criteria: vec!["Done".to_string()],
            plan: "Implement".to_string(),
            priority: TaskPriority::Low,
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("seed foundation");
    let dependent = runtime
        .add_task(TaskAddParams {
            parent_id: Some(epic.id.clone()),
            title: "Dependent".to_string(),
            description: "Dependent fixture".to_string(),
            acceptance_criteria: vec!["Done".to_string()],
            dependencies: vec![foundation.id.clone()],
            plan: "Implement".to_string(),
            priority: TaskPriority::High,
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("seed dependent");
    let independent = runtime
        .add_task(TaskAddParams {
            parent_id: Some(epic.id.clone()),
            title: "Independent".to_string(),
            description: "Independent fixture".to_string(),
            acceptance_criteria: vec!["Done".to_string()],
            plan: "Implement".to_string(),
            priority: TaskPriority::Critical,
            status: Some(TaskStatus::Backlog),
            ..Default::default()
        })
        .expect("seed independent");
    let done = runtime
        .add_task(TaskAddParams {
            parent_id: Some(epic.id.clone()),
            title: "Already done".to_string(),
            description: "Done fixture".to_string(),
            acceptance_criteria: vec!["Done".to_string()],
            plan: "Implemented".to_string(),
            status: Some(TaskStatus::Done),
            ..Default::default()
        })
        .expect("seed done child");

    let output = list_epic_descendants(&runtime, &epic.id);
    assert_eq!(
        output["task_ids"],
        json!([independent.id, foundation.id, dependent.id])
    );
    assert_eq!(output["task_count"], 3);
    assert!(
        !output["task_ids"]
            .as_array()
            .expect("task ids")
            .contains(&json!(done.id))
    );
}

#[test]
fn epic_with_no_descendants_has_an_empty_drain() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let epic = runtime
        .add_task(TaskAddParams {
            title: "Empty epic".to_string(),
            description: "Epic fixture".to_string(),
            acceptance_criteria: vec!["No children".to_string()],
            tags: vec!["epic".to_string()],
            plan: "No-op".to_string(),
            status: Some(TaskStatus::InProgress),
            ..Default::default()
        })
        .expect("seed empty epic");

    let output = list_epic_descendants(&runtime, &epic.id);
    assert_eq!(output["task_ids"], json!([]));
    assert_eq!(output["task_count"], 0);
    assert_eq!(output["empty"], true);
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
