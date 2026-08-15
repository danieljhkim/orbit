//! Deterministic unresolved-work scan for `epic_pipeline` [ORB-10779].
//!
//! Read-only. Wakes on `proposed` / `backlog` / `blocked` tasks, `failed` /
//! `timeout` job-runs (except the drain job itself), and unresolved
//! `check_later` session-log entries. Empty is success, not an error.

use orbit_common::types::{JobRunState, TaskStatus};
use orbit_engine::DispatchError;
use orbit_store::{JobRunQuery, SessionLogFilter, SessionLogKind, SessionLogStore};
use serde_json::{Value, json};

use crate::OrbitRuntime;

/// Job whose own failed/timeout rows must not re-admit the drain loop.
pub(super) const EPIC_PIPELINE_JOB_ID: &str = "epic_pipeline";

const WAKE_TASK_STATUSES: [TaskStatus; 3] = [
    TaskStatus::Proposed,
    TaskStatus::Backlog,
    TaskStatus::Blocked,
];

const WAKE_RUN_STATES: [JobRunState; 2] = [JobRunState::Failed, JobRunState::Timeout];

pub(super) fn scan_unresolved_work(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<Value, DispatchError> {
    let fail_if_nonempty = input
        .get("fail_if_nonempty")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut task_ids: Vec<String> = runtime
        .stores()
        .tasks()
        .list_tasks()
        .map_err(|err| action_failed(action, format!("list tasks: {err}")))?
        .into_iter()
        .filter(|task| WAKE_TASK_STATUSES.contains(&task.status))
        .map(|task| task.id)
        .collect();
    task_ids.sort();

    let mut run_ids: Vec<String> = runtime
        .stores()
        .jobs()
        .list_job_runs_filtered(&JobRunQuery::default())
        .map_err(|err| action_failed(action, format!("list job runs: {err}")))?
        .into_iter()
        .filter(|run| run.job_id != EPIC_PIPELINE_JOB_ID && WAKE_RUN_STATES.contains(&run.state))
        .map(|run| run.run_id)
        .collect();
    run_ids.sort();

    let mut check_later_ids: Vec<String> = SessionLogStore::new(runtime.paths().orbit_dir.clone())
        .list(SessionLogFilter {
            kind: Some(SessionLogKind::CheckLater),
            unresolved_only: true,
            ..SessionLogFilter::default()
        })
        .map_err(|err| action_failed(action, format!("list session log: {err}")))?
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    check_later_ids.sort();

    let empty = task_ids.is_empty() && run_ids.is_empty() && check_later_ids.is_empty();
    if fail_if_nonempty && !empty {
        return Err(action_failed(
            action,
            format!(
                "unresolved work remains after drain: tasks=[{}], runs=[{}], check_later=[{}]",
                task_ids.join(", "),
                run_ids.join(", "),
                check_later_ids.join(", ")
            ),
        ));
    }

    Ok(json!({
        "empty": empty,
        "task_ids": task_ids,
        "run_ids": run_ids,
        "check_later_ids": check_later_ids,
        "task_count": task_ids.len(),
        "run_count": run_ids.len(),
        "check_later_count": check_later_ids.len(),
    }))
}

fn action_failed(action: &str, message: impl Into<String>) -> DispatchError {
    DispatchError::DeterministicActionFailed {
        action: action.to_string(),
        message: message.into(),
    }
}
