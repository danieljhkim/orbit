use orbit_common::types::{Task, TaskPriority, TaskStatus, TaskType};
use orbit_engine::V2RuntimeHost;
use orbit_tools::ToolContext;
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::command::task::{TaskAddParams, TaskUpdateParams};
use crate::runtime::v2_host::test_support::{
    runtime_with_workspace_layout, seed_accepted_friction_task, seed_list_backlog_task,
    write_workspace_file,
};

fn list_backlog_tasks(runtime: &OrbitRuntime, input: Value) -> Value {
    runtime
        .run_deterministic(
            "list_backlog_tasks",
            &json!({}),
            &input,
            ToolContext::default(),
        )
        .expect("list backlog tasks")
}

fn load_epic(runtime: &OrbitRuntime, epic_task_id: &str) -> Value {
    runtime
        .run_deterministic(
            "load_epic",
            &json!({}),
            &json!({ "epic_task_id": epic_task_id }),
            ToolContext::default(),
        )
        .expect("load epic")
}

fn excluded_entry<'a>(output: &'a Value, task_id: &str) -> &'a Value {
    output["excluded"]
        .as_array()
        .expect("excluded array")
        .iter()
        .find(|entry| entry["id"] == task_id)
        .expect("excluded entry")
}

fn output_task_ids(output: &Value) -> Vec<String> {
    output["task_ids"]
        .as_array()
        .expect("task_ids array")
        .iter()
        .map(|task_id| {
            task_id
                .as_str()
                .expect("task_id should be a string")
                .to_string()
        })
        .collect()
}

fn seed_task_with_dependencies(
    runtime: &OrbitRuntime,
    title: &str,
    status: TaskStatus,
    dependencies: Vec<String>,
) -> Task {
    runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            description: format!("Fixture task: {title}"),
            acceptance_criteria: vec!["Fixture task is observable.".to_string()],
            dependencies,
            plan: "Fixture plan.".to_string(),
            workspace_path: Some(".".to_string()),
            priority: TaskPriority::Medium,
            task_type: Some(TaskType::Chore),
            status: Some(status),
            ..Default::default()
        })
        .expect("seed task with dependencies")
}

fn seed_backlog_task_with_dependencies(
    runtime: &OrbitRuntime,
    title: &str,
    dependencies: Vec<String>,
) -> Task {
    seed_task_with_dependencies(runtime, title, TaskStatus::Backlog, dependencies)
}

#[test]
fn load_epic_treats_review_subtasks_as_shipped_terminal_state() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let epic = seed_list_backlog_task(
        &runtime,
        "Epic",
        TaskStatus::InProgress,
        TaskPriority::High,
        TaskType::Feature,
        None,
        vec![],
    );
    let review = seed_list_backlog_task(
        &runtime,
        "Review child",
        TaskStatus::Review,
        TaskPriority::High,
        TaskType::Chore,
        Some(epic.id.clone()),
        vec![],
    );
    let done = seed_list_backlog_task(
        &runtime,
        "Done child",
        TaskStatus::Done,
        TaskPriority::Medium,
        TaskType::Chore,
        Some(epic.id.clone()),
        vec![],
    );

    let output = load_epic(&runtime, &epic.id);

    assert_eq!(output["all_terminal"], json!(true));
    assert_eq!(output["subtasks"], json!([]));
    assert_eq!(
        output["final_state"]["subtasks"][review.id.as_str()],
        json!({
            "state": "done",
            "status": "review",
            "title": "Review child"
        })
    );
    assert_eq!(
        output["final_state"]["subtasks"][done.id.as_str()],
        json!({
            "state": "done",
            "status": "done",
            "title": "Done child"
        })
    );
}

#[test]
fn load_epic_keeps_in_progress_subtasks_open_when_review_is_shipped() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let epic = seed_list_backlog_task(
        &runtime,
        "Epic",
        TaskStatus::InProgress,
        TaskPriority::High,
        TaskType::Feature,
        None,
        vec![],
    );
    let review = seed_list_backlog_task(
        &runtime,
        "Review child",
        TaskStatus::Review,
        TaskPriority::High,
        TaskType::Chore,
        Some(epic.id.clone()),
        vec![],
    );
    let in_progress = seed_list_backlog_task(
        &runtime,
        "In progress child",
        TaskStatus::InProgress,
        TaskPriority::High,
        TaskType::Chore,
        Some(epic.id.clone()),
        vec![],
    );

    let output = load_epic(&runtime, &epic.id);
    let subtasks = output["subtasks"].as_array().expect("subtasks array");

    assert_eq!(output["all_terminal"], json!(false));
    assert_eq!(subtasks.len(), 1);
    assert_eq!(subtasks[0]["id"], json!(in_progress.id));
    assert_eq!(subtasks[0]["status"], json!("in-progress"));
    assert!(
        subtasks
            .iter()
            .all(|entry| entry["id"].as_str() != Some(review.id.as_str()))
    );
}

#[test]
fn list_backlog_tasks_preserves_existing_fields_without_conflicts() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    write_workspace_file(&repo_root, "crates/alpha/src/lib.rs");
    write_workspace_file(&repo_root, "crates/beta/src/lib.rs");
    let medium = seed_list_backlog_task(
        &runtime,
        "Medium backlog",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec!["crates/alpha/src/lib.rs"],
    );
    let high = seed_list_backlog_task(
        &runtime,
        "High backlog",
        TaskStatus::Backlog,
        TaskPriority::High,
        TaskType::Chore,
        None,
        vec!["crates/beta/src/lib.rs"],
    );

    let output = list_backlog_tasks(&runtime, json!({}));

    assert_eq!(output["task_count"], json!(2));
    assert_eq!(output["task_ids"], json!([high.id, medium.id]));
    assert_eq!(
        output["tasks"],
        json!([
            {
                "id": high.id,
                "title": "High backlog",
                "type": "chore",
                "priority": "high",
                "context_files": high.context_files,
                "parent_id": null
            },
            {
                "id": medium.id,
                "title": "Medium backlog",
                "type": "chore",
                "priority": "medium",
                "context_files": medium.context_files,
                "parent_id": null
            }
        ])
    );
    assert_eq!(output["excluded"], json!([]));
}

#[test]
fn list_backlog_tasks_filters_dependency_readiness() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let proposed_dependency = seed_task_with_dependencies(
        &runtime,
        "Proposed dependency",
        TaskStatus::Proposed,
        vec![],
    );
    let backlog_dependency =
        seed_task_with_dependencies(&runtime, "Backlog dependency", TaskStatus::Backlog, vec![]);
    let in_progress_dependency = seed_task_with_dependencies(
        &runtime,
        "In-progress dependency",
        TaskStatus::InProgress,
        vec![],
    );
    let review_dependency =
        seed_task_with_dependencies(&runtime, "Review dependency", TaskStatus::Review, vec![]);
    let done_dependency =
        seed_task_with_dependencies(&runtime, "Done dependency", TaskStatus::Done, vec![]);
    let ready = seed_backlog_task_with_dependencies(
        &runtime,
        "Ready dependent",
        vec![done_dependency.id.clone()],
    );
    let no_dependencies = seed_backlog_task_with_dependencies(&runtime, "No dependencies", vec![]);
    let blocked_by_proposed = seed_backlog_task_with_dependencies(
        &runtime,
        "Blocked by proposed",
        vec![proposed_dependency.id.clone()],
    );
    let blocked_by_backlog = seed_backlog_task_with_dependencies(
        &runtime,
        "Blocked by backlog",
        vec![backlog_dependency.id.clone()],
    );
    let blocked_by_in_progress = seed_backlog_task_with_dependencies(
        &runtime,
        "Blocked by in-progress",
        vec![in_progress_dependency.id.clone()],
    );
    let blocked_by_review = seed_backlog_task_with_dependencies(
        &runtime,
        "Blocked by review",
        vec![review_dependency.id.clone()],
    );

    let output = list_backlog_tasks(&runtime, json!({}));
    let task_ids = output_task_ids(&output);

    assert!(task_ids.contains(&ready.id));
    assert!(task_ids.contains(&no_dependencies.id));
    assert!(task_ids.contains(&backlog_dependency.id));
    assert!(!task_ids.contains(&blocked_by_proposed.id));
    assert!(!task_ids.contains(&blocked_by_backlog.id));
    assert!(!task_ids.contains(&blocked_by_in_progress.id));
    assert!(!task_ids.contains(&blocked_by_review.id));
    assert_eq!(output["excluded"], json!([]));
}

#[test]
fn list_backlog_tasks_serializes_orb_00042_grok_epic_chain() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    let orb43 = seed_backlog_task_with_dependencies(&runtime, "ORB-00043 grok foundation", vec![]);
    let orb44 = seed_backlog_task_with_dependencies(
        &runtime,
        "ORB-00044 grok follow-up",
        vec![orb43.id.clone()],
    );
    let orb45 = seed_backlog_task_with_dependencies(
        &runtime,
        "ORB-00045 grok follow-up",
        vec![orb43.id.clone()],
    );
    let orb46 = seed_backlog_task_with_dependencies(
        &runtime,
        "ORB-00046 grok follow-up",
        vec![orb43.id.clone()],
    );
    let orb48 = seed_backlog_task_with_dependencies(
        &runtime,
        "ORB-00048 grok follow-up",
        vec![orb43.id.clone()],
    );

    let output = list_backlog_tasks(&runtime, json!({}));

    assert_eq!(output_task_ids(&output), vec![orb43.id.clone()]);

    runtime
        .update_task(
            &orb43.id,
            TaskUpdateParams {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        )
        .expect("mark ORB-00043 done");
    let output = list_backlog_tasks(&runtime, json!({}));
    let task_ids = output_task_ids(&output);

    assert_eq!(task_ids.len(), 4);
    assert!(task_ids.contains(&orb44.id));
    assert!(task_ids.contains(&orb45.id));
    assert!(task_ids.contains(&orb46.id));
    assert!(task_ids.contains(&orb48.id));
}

#[test]
fn list_backlog_tasks_includes_accepted_friction_reports() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    write_workspace_file(&repo_root, "crates/friction/src/lib.rs");
    let friction = seed_accepted_friction_task(
        &runtime,
        "Accepted friction",
        TaskPriority::Medium,
        vec!["crates/friction/src/lib.rs"],
    );

    let output = list_backlog_tasks(&runtime, json!({}));

    assert_eq!(output["task_count"], json!(1));
    assert_eq!(output["task_ids"], json!([friction.id]));
    assert_eq!(output["bundles"], json!([[friction.id]]));
    assert_eq!(
        output["tasks"],
        json!([{
            "id": friction.id,
            "title": "Accepted friction",
            "type": "chore",
            "priority": "medium",
            "context_files": friction.context_files,
            "parent_id": null
        }])
    );
    assert_eq!(output["excluded"], json!([]));
}

#[test]
fn list_backlog_tasks_omits_untriaged_friction_reports() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    write_workspace_file(&repo_root, "crates/friction/src/lib.rs");
    let friction = seed_list_backlog_task(
        &runtime,
        "Untriaged friction",
        TaskStatus::Friction,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec!["crates/friction/src/lib.rs"],
    );
    let friction_id = friction.id.clone();

    let output = list_backlog_tasks(&runtime, json!({}));

    assert_eq!(output["task_count"], json!(0));
    assert_eq!(output["task_ids"], json!([]));
    assert_eq!(output["tasks"], json!([]));
    assert_eq!(output["bundles"], json!([]));
    assert_eq!(output["excluded"], json!([]));
    assert!(
        output["task_ids"]
            .as_array()
            .expect("task_ids")
            .iter()
            .all(|task_id| task_id != &json!(friction_id))
    );
}

#[test]
fn list_backlog_tasks_reports_direct_context_lock_conflicts() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    write_workspace_file(&repo_root, "crates/foo/src/lib.rs");
    let locking = seed_list_backlog_task(
        &runtime,
        "Locking task",
        TaskStatus::InProgress,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec!["crates/foo/src/lib.rs"],
    );
    let backlog = seed_list_backlog_task(
        &runtime,
        "Backlog task",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec!["crates/foo/src/lib.rs"],
    );

    let output = list_backlog_tasks(&runtime, json!({}));

    assert_eq!(output["task_count"], json!(0));
    assert_eq!(output["task_ids"], json!([]));
    assert_eq!(output["tasks"], json!([]));
    assert_eq!(output["bundles"], json!([]));
    assert_eq!(
        output["excluded"],
        json!([{
            "id": backlog.id,
            "reason": "context_lock_conflict",
            "conflicts": [{
                "requested_file": backlog.context_files[0],
                "locking_task_id": locking.id
            }]
        }])
    );
}

#[test]
fn list_backlog_tasks_reports_group_member_conflicts_with_trigger_conflicts() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    write_workspace_file(&repo_root, "docs/parent.md");
    write_workspace_file(&repo_root, "crates/foo/src/lib.rs");
    write_workspace_file(&repo_root, "crates/bar/src/lib.rs");
    let foo_lock = seed_list_backlog_task(
        &runtime,
        "Foo lock",
        TaskStatus::InProgress,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec!["crates/foo/src/lib.rs"],
    );
    let bar_lock = seed_list_backlog_task(
        &runtime,
        "Bar lock",
        TaskStatus::InProgress,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec!["crates/bar/src/lib.rs"],
    );
    let parent = seed_list_backlog_task(
        &runtime,
        "Parent",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec!["docs/parent.md"],
    );
    let low_child = seed_list_backlog_task(
        &runtime,
        "Low child",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Chore,
        Some(parent.id.clone()),
        vec!["crates/foo/src/lib.rs"],
    );
    let high_child = seed_list_backlog_task(
        &runtime,
        "High child",
        TaskStatus::Backlog,
        TaskPriority::High,
        TaskType::Chore,
        Some(parent.id.clone()),
        vec!["crates/bar/src/lib.rs"],
    );

    let output = list_backlog_tasks(&runtime, json!({}));

    assert_eq!(output["task_count"], json!(0));
    assert_eq!(output["excluded"].as_array().expect("excluded").len(), 3);
    assert_eq!(
        excluded_entry(&output, &parent.id),
        &json!({
            "id": parent.id,
            "reason": "group_member_conflict",
            "conflicts": [{
                "requested_file": high_child.context_files[0],
                "locking_task_id": bar_lock.id
            }]
        })
    );
    assert_eq!(
        excluded_entry(&output, &high_child.id),
        &json!({
            "id": high_child.id,
            "reason": "context_lock_conflict",
            "conflicts": [{
                "requested_file": high_child.context_files[0],
                "locking_task_id": bar_lock.id
            }]
        })
    );
    assert_eq!(
        excluded_entry(&output, &low_child.id),
        &json!({
            "id": low_child.id,
            "reason": "context_lock_conflict",
            "conflicts": [{
                "requested_file": low_child.context_files[0],
                "locking_task_id": foo_lock.id
            }]
        })
    );
}

#[test]
fn list_backlog_tasks_reports_accepted_friction_context_lock_conflicts() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    write_workspace_file(&repo_root, "crates/friction/src/lib.rs");
    let locking = seed_list_backlog_task(
        &runtime,
        "Locking task",
        TaskStatus::InProgress,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec!["crates/friction/src/lib.rs"],
    );
    let friction = seed_accepted_friction_task(
        &runtime,
        "Accepted friction",
        TaskPriority::Medium,
        vec!["crates/friction/src/lib.rs"],
    );

    let output = list_backlog_tasks(&runtime, json!({}));

    assert_eq!(output["task_count"], json!(0));
    assert_eq!(output["task_ids"], json!([]));
    assert_eq!(output["tasks"], json!([]));
    assert_eq!(output["bundles"], json!([]));
    assert_eq!(
        output["excluded"],
        json!([{
            "id": friction.id,
            "reason": "context_lock_conflict",
            "conflicts": [{
                "requested_file": friction.context_files[0],
                "locking_task_id": locking.id
            }]
        }])
    );
}

#[test]
fn list_backlog_tasks_does_not_report_untriaged_friction_tasks_as_excluded() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    write_workspace_file(&repo_root, "crates/foo/src/lib.rs");
    let locking = seed_list_backlog_task(
        &runtime,
        "Locking task",
        TaskStatus::InProgress,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec!["crates/foo/src/lib.rs"],
    );
    let friction = seed_list_backlog_task(
        &runtime,
        "Friction task",
        TaskStatus::Friction,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec!["crates/foo/src/lib.rs"],
    );
    let backlog = seed_list_backlog_task(
        &runtime,
        "Backlog task",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec!["crates/foo/src/lib.rs"],
    );

    let output = list_backlog_tasks(&runtime, json!({}));

    assert_eq!(
        output["excluded"],
        json!([{
            "id": backlog.id,
            "reason": "context_lock_conflict",
            "conflicts": [{
                "requested_file": backlog.context_files[0],
                "locking_task_id": locking.id
            }]
        }])
    );
    assert!(
        output["excluded"]
            .as_array()
            .expect("excluded")
            .iter()
            .all(|entry| entry["id"] != friction.id)
    );
}

#[test]
fn list_backlog_tasks_does_not_report_max_tasks_truncation_as_excluded() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    for index in 0..3 {
        let path = format!("docs/task-{index}.md");
        write_workspace_file(&repo_root, &path);
        seed_list_backlog_task(
            &runtime,
            &format!("Task {index}"),
            TaskStatus::Backlog,
            TaskPriority::Medium,
            TaskType::Chore,
            None,
            vec![&path],
        );
    }

    let output = list_backlog_tasks(&runtime, json!({ "max_tasks": 2 }));

    assert_eq!(output["task_count"], json!(2));
    assert_eq!(output["task_ids"].as_array().expect("task_ids").len(), 2);
    assert_eq!(output["excluded"], json!([]));
}

#[test]
fn list_backlog_tasks_omits_excluded_for_explicit_task_ids() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    write_workspace_file(&repo_root, "crates/foo/src/lib.rs");
    seed_list_backlog_task(
        &runtime,
        "Locking task",
        TaskStatus::InProgress,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec!["crates/foo/src/lib.rs"],
    );
    let backlog = seed_list_backlog_task(
        &runtime,
        "Backlog task",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Chore,
        None,
        vec!["crates/foo/src/lib.rs"],
    );

    let output = list_backlog_tasks(&runtime, json!({ "task_ids": [backlog.id] }));

    assert_eq!(output["task_count"], json!(1));
    assert_eq!(output["task_ids"], json!([backlog.id]));
    assert!(output.get("excluded").is_none());
}
