//! Guarded status transitions through `update_task` (ORB-10000): the
//! approve / reject / unarchive verbs folded into `--status` updates.

use orbit_common::types::{Task, TaskStatus};

use super::test_runtime;
use crate::OrbitRuntime;
use crate::command::task::{TaskAddParams, TaskUpdateParams};

fn add_proposed_task(runtime: &OrbitRuntime, title: &str) -> Task {
    runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            description: "Exercise guarded update transitions.".to_string(),
            acceptance_criteria: vec!["status lands where the update says.".to_string()],
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("add proposed task")
}

fn update_status(
    runtime: &OrbitRuntime,
    id: &str,
    status: TaskStatus,
) -> Result<Task, orbit_common::types::OrbitError> {
    runtime.update_task(
        id,
        TaskUpdateParams {
            status: Some(status),
            ..Default::default()
        },
    )
}

#[test]
fn update_status_backlog_restores_archived_task() {
    let (_root, runtime) = test_runtime();
    let task = add_proposed_task(&runtime, "Archive then restore");
    runtime.archive_task(&task.id).expect("archive task");

    let restored = update_status(&runtime, &task.id, TaskStatus::Backlog)
        .expect("archived task restores to backlog via update");
    assert_eq!(restored.status, TaskStatus::Backlog);
}

#[test]
fn archived_task_rejects_non_restore_mutations() {
    let (_root, runtime) = test_runtime();
    let task = add_proposed_task(&runtime, "Archived stays frozen");
    runtime.archive_task(&task.id).expect("archive task");

    let err = runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                title: Some("new title".to_string()),
                ..Default::default()
            },
        )
        .expect_err("archived task rejects field edits");
    assert!(err.to_string().contains("--status backlog"), "{err}");

    let err = update_status(&runtime, &task.id, TaskStatus::InProgress)
        .expect_err("archived task only restores to backlog");
    assert!(err.to_string().contains("--status backlog"), "{err}");
}

#[test]
fn update_status_rejected_is_guarded() {
    let (_root, runtime) = test_runtime();

    // Legal: proposed -> rejected.
    let task = add_proposed_task(&runtime, "Reject a proposal");
    let rejected = update_status(&runtime, &task.id, TaskStatus::Rejected)
        .expect("proposed task rejects via update");
    assert_eq!(rejected.status, TaskStatus::Rejected);

    // Legal: backlog -> rejected and in-progress -> rejected.
    let task = add_proposed_task(&runtime, "Reject from backlog");
    update_status(&runtime, &task.id, TaskStatus::Backlog).expect("approve to backlog");
    let rejected = update_status(&runtime, &task.id, TaskStatus::Rejected)
        .expect("backlog task rejects via update");
    assert_eq!(rejected.status, TaskStatus::Rejected);
}

#[test]
fn update_status_rejects_illegal_jumps() {
    let (_root, runtime) = test_runtime();
    let task = add_proposed_task(&runtime, "Done is terminal");
    let done = drive_to_done(&runtime, &task.id);
    assert_eq!(done.status, TaskStatus::Done);

    let err = update_status(&runtime, &task.id, TaskStatus::Rejected)
        .expect_err("done -> rejected is an illegal jump");
    assert!(err.to_string().contains("done"), "{err}");

    // Setting archived through update stays blocked too.
    let other = add_proposed_task(&runtime, "No bare archived writes");
    let err = update_status(&runtime, &other.id, TaskStatus::Archived)
        .expect_err("update --status archived is blocked");
    assert!(err.to_string().contains("archive"), "{err}");
}

#[test]
fn update_status_covers_approve_transitions() {
    let (_root, runtime) = test_runtime();
    let task = add_proposed_task(&runtime, "Approve via update");

    // proposed -> backlog (the former `task approve`).
    let approved = update_status(&runtime, &task.id, TaskStatus::Backlog)
        .expect("proposed task approves into backlog via update");
    assert_eq!(approved.status, TaskStatus::Backlog);

    // review -> done (the former review approval).
    let done = drive_to_done(&runtime, &task.id);
    assert_eq!(done.status, TaskStatus::Done);
}

/// Walks a task through backlog -> in-progress -> review -> done using only
/// `update_task`, satisfying the plan and execution-summary guards.
fn drive_to_done(runtime: &OrbitRuntime, id: &str) -> Task {
    let current = runtime.get_task(id).expect("get task");
    if current.status == TaskStatus::Proposed {
        update_status(runtime, id, TaskStatus::Backlog).expect("proposed -> backlog");
    }
    runtime
        .update_task(
            id,
            TaskUpdateParams {
                plan: Some("1) do the thing 2) verify".to_string()),
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .expect("backlog -> in-progress with plan");
    runtime
        .update_task(
            id,
            TaskUpdateParams {
                execution_summary: Some("did the thing; verified".to_string()),
                status: Some(TaskStatus::Review),
                ..Default::default()
            },
        )
        .expect("in-progress -> review with execution summary");
    update_status(runtime, id, TaskStatus::Done).expect("review -> done")
}
