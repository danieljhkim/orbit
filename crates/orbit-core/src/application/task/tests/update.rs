//! Guarded status transitions through `update_task` (ORB-10000): the
//! approve / reject / unarchive verbs folded into `--status` updates.

use orbit_types::task::{Task, TaskStatus};

use super::test_runtime;
use crate::OrbitRuntime;
use crate::application::task::{TaskAddParams, TaskUpdateParams};

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
) -> Result<Task, orbit_common::OrbitError> {
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

#[test]
fn required_tools_are_normalized_before_execution_and_frozen_after_admission() {
    let (_root, runtime) = test_runtime();
    let task = add_proposed_task(&runtime, "Freeze required tools");

    let updated = runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                required_tools: Some(vec![
                    "github.run.list".to_string(),
                    "github.auth.status".to_string(),
                    "github.run.list".to_string(),
                ]),
                ..Default::default()
            },
        )
        .expect("set required tools while proposed");
    assert_eq!(
        updated.required_tools,
        vec!["github.auth.status", "github.run.list"]
    );

    let entering = runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                plan: Some("Execute the task.".to_string()),
                status: Some(TaskStatus::InProgress),
                required_tools: Some(vec!["github.job.get".to_string()]),
                ..Default::default()
            },
        )
        .expect_err("the admitting update cannot change required tools");
    assert!(entering.to_string().contains(&task.id), "{entering}");
    assert!(entering.to_string().contains("frozen"), "{entering}");

    runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                plan: Some("Execute the task.".to_string()),
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .expect("enter in-progress with fixed requirements");
    let active = runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                required_tools: Some(vec!["github.job.get".to_string()]),
                ..Default::default()
            },
        )
        .expect_err("active executor cannot change current requirements");
    assert!(active.to_string().contains("frozen"), "{active}");

    runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                status: Some(TaskStatus::Blocked),
                ..Default::default()
            },
        )
        .expect("block task for retry");
    let retry = runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                required_tools: Some(vec!["github.job.get".to_string()]),
                ..Default::default()
            },
        )
        .expect_err("retry requirements remain frozen after in-progress");
    assert!(retry.to_string().contains("frozen"), "{retry}");

    let unchanged = runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                required_tools: Some(updated.required_tools.clone()),
                ..Default::default()
            },
        )
        .expect("an identical replacement is not an authority change");
    assert_eq!(unchanged.required_tools, updated.required_tools);
}

#[test]
fn orchestrator_is_explicit_mutable_before_start_and_never_routes_execution() {
    let (_root, runtime) = test_runtime();
    let task = runtime
        .add_task(TaskAddParams {
            title: "Orchestration ownership".to_string(),
            description: "Keep orchestration attribution separate from execution.".to_string(),
            crew: Some("implementer".to_string()),
            orchestrator: Some("  orchestration  ".to_string()),
            ..Default::default()
        })
        .expect("add task with orchestration attribution");
    assert_eq!(task.orchestrator.as_deref(), Some("orchestration"));
    assert_eq!(
        runtime
            .resolve_crew_for_task(None, task.crew.as_deref())
            .expect("resolve execution crew")
            .name,
        "implementer"
    );

    let changed = runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                orchestrator: Some(Some("  implementer  ".to_string())),
                ..Default::default()
            },
        )
        .expect("change orchestration attribution while proposed");
    assert_eq!(changed.orchestrator.as_deref(), Some("implementer"));
    let cleared = runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                orchestrator: Some(None),
                ..Default::default()
            },
        )
        .expect("clear orchestration attribution while proposed");
    assert_eq!(cleared.orchestrator, None);

    let invalid = runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                orchestrator: Some(Some("missing".to_string())),
                ..Default::default()
            },
        )
        .expect_err("reject unconfigured orchestrator");
    assert!(invalid.to_string().contains("missing"), "{invalid}");

    runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                plan: Some("Implement it.".to_string()),
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .expect("start task");
    let immutable = runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                orchestrator: Some(Some("orchestration".to_string())),
                ..Default::default()
            },
        )
        .expect_err("orchestrator becomes immutable after execution starts");
    assert!(
        immutable.to_string().contains("proposed or backlog"),
        "{immutable}"
    );
}

#[test]
fn orchestrator_is_rejected_on_non_draft_initial_statuses_including_someday() {
    let (_root, runtime) = test_runtime();

    for status in [TaskStatus::Someday, TaskStatus::InProgress] {
        let error = runtime
            .add_task(TaskAddParams {
                title: format!("Invalid {status} orchestration attribution"),
                description: "Orchestration ownership must be assigned before this state."
                    .to_string(),
                status: Some(status),
                orchestrator: Some("orchestration".to_string()),
                ..Default::default()
            })
            .expect_err("non-draft initial status rejects orchestrator");
        assert!(
            error.to_string().contains("proposed or backlog"),
            "{status}: {error}"
        );
    }
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

/// ORB-10988 / F2026-07-119: the update path must hold the task lock across
/// its *whole* read-modify-write, not just around the store write.
///
/// The body reads the task, decides from that snapshot whether the mutation is
/// even legal, and only then writes. When the lock covered the write alone, a
/// concurrent update could commit a status change inside that gap: the second
/// writer had already cleared the guard against a status it never saw, and its
/// write landed on a task that had since become unmodifiable.
///
/// The other thread holds the bundle lock directly, so the window is opened
/// deliberately rather than raced for.
#[test]
fn concurrent_update_cannot_write_through_a_status_change_it_never_saw() {
    use std::sync::mpsc::sync_channel;
    use std::time::Duration;

    let (root, runtime) = test_runtime();
    let task = add_proposed_task(&runtime, "Guard under contention");
    let lock_target = task_bundle_dir(root.path(), &task.id).join("task.yaml");

    let (locked_tx, locked_rx) = sync_channel::<()>(0);
    let (contender_tx, contender_rx) = sync_channel::<()>(0);

    // `move` on the holder closure captures only the channel endpoints; the
    // runtime and paths cross as shared references, which are `Copy`.
    let holder_runtime = &runtime;
    let holder_lock_target = lock_target.as_path();
    let holder_id = task.id.clone();
    let contended = std::thread::scope(|scope| {
        scope.spawn(move || {
            orbit_common::fs::io::with_exclusive_file_lock::<(), orbit_common::OrbitError, _>(
                holder_lock_target,
                "ORB-10988 regression",
                || {
                    locked_tx.send(()).expect("announce the held lock");
                    contender_rx.recv().expect("await the contending update");
                    // Long enough that an update which reads before locking has
                    // certainly taken its stale snapshot by now.
                    std::thread::sleep(Duration::from_millis(250));
                    holder_runtime
                        .archive_task(&holder_id)
                        .expect("archive under the lock");
                    Ok(())
                },
            )
            .expect("hold the task lock");
        });

        locked_rx.recv().expect("await the held lock");
        contender_tx
            .send(())
            .expect("announce the contending update");
        runtime.update_task(
            &task.id,
            TaskUpdateParams {
                title: Some("renamed by the loser of the race".to_string()),
                ..Default::default()
            },
        )
    });

    let err = contended.expect_err("an archived task must refuse a rename");
    assert!(
        err.to_string().contains("cannot be modified"),
        "expected the archived-task guard, got: {err}"
    );
    let reread = runtime.get_task(&task.id).expect("task still readable");
    assert_eq!(reread.status, TaskStatus::Archived);
    assert_eq!(
        reread.title, "Guard under contention",
        "the losing writer must not have renamed an archived task"
    );
}

/// Locate a task's bundle directory under a test runtime's roots. The store
/// owns the layout; the test only needs *a* path to contend on.
fn task_bundle_dir(root: &std::path::Path, id: &str) -> std::path::PathBuf {
    fn walk(dir: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if path.file_name().is_some_and(|name| name == id) && path.join("task.yaml").is_file() {
                return Some(path);
            }
            if let Some(found) = walk(&path, id) {
                return Some(found);
            }
        }
        None
    }
    walk(root, id).unwrap_or_else(|| panic!("no bundle directory for {id} under {root:?}"))
}
