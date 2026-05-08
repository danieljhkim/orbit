use orbit_common::types::{TaskPriority, TaskStatus, TaskType};
use orbit_engine::activity_job::V2RuntimeHost;
use orbit_tools::ToolContext;
use serde_json::{Value, json};

use crate::OrbitRuntime;
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

fn excluded_entry<'a>(output: &'a Value, task_id: &str) -> &'a Value {
    output["excluded"]
        .as_array()
        .expect("excluded array")
        .iter()
        .find(|entry| entry["id"] == task_id)
        .expect("excluded entry")
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
        TaskType::Task,
        None,
        vec!["crates/alpha/src/lib.rs"],
    );
    let high = seed_list_backlog_task(
        &runtime,
        "High backlog",
        TaskStatus::Backlog,
        TaskPriority::High,
        TaskType::Task,
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
                "type": "task",
                "priority": "high",
                "context_files": high.context_files,
                "parent_id": null
            },
            {
                "id": medium.id,
                "title": "Medium backlog",
                "type": "task",
                "priority": "medium",
                "context_files": medium.context_files,
                "parent_id": null
            }
        ])
    );
    assert_eq!(output["excluded"], json!([]));
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
            "type": "friction",
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
        TaskType::Friction,
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
        TaskType::Task,
        None,
        vec!["crates/foo/src/lib.rs"],
    );
    let backlog = seed_list_backlog_task(
        &runtime,
        "Backlog task",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Task,
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
        TaskType::Task,
        None,
        vec!["crates/foo/src/lib.rs"],
    );
    let bar_lock = seed_list_backlog_task(
        &runtime,
        "Bar lock",
        TaskStatus::InProgress,
        TaskPriority::Medium,
        TaskType::Task,
        None,
        vec!["crates/bar/src/lib.rs"],
    );
    let parent = seed_list_backlog_task(
        &runtime,
        "Parent",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Task,
        None,
        vec!["docs/parent.md"],
    );
    let low_child = seed_list_backlog_task(
        &runtime,
        "Low child",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Task,
        Some(parent.id.clone()),
        vec!["crates/foo/src/lib.rs"],
    );
    let high_child = seed_list_backlog_task(
        &runtime,
        "High child",
        TaskStatus::Backlog,
        TaskPriority::High,
        TaskType::Task,
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
        TaskType::Task,
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
        TaskType::Task,
        None,
        vec!["crates/foo/src/lib.rs"],
    );
    let friction = seed_list_backlog_task(
        &runtime,
        "Friction task",
        TaskStatus::Friction,
        TaskPriority::Medium,
        TaskType::Friction,
        None,
        vec!["crates/foo/src/lib.rs"],
    );
    let backlog = seed_list_backlog_task(
        &runtime,
        "Backlog task",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Task,
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
            TaskType::Task,
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
        TaskType::Task,
        None,
        vec!["crates/foo/src/lib.rs"],
    );
    let backlog = seed_list_backlog_task(
        &runtime,
        "Backlog task",
        TaskStatus::Backlog,
        TaskPriority::Medium,
        TaskType::Task,
        None,
        vec!["crates/foo/src/lib.rs"],
    );

    let output = list_backlog_tasks(&runtime, json!({ "task_ids": [backlog.id] }));

    assert_eq!(output["task_count"], json!(1));
    assert_eq!(output["task_ids"], json!([backlog.id]));
    assert!(output.get("excluded").is_none());
}
