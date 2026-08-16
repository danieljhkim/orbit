use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use orbit_common::types::{Task, TaskStatus, task_dependencies_ready};
use orbit_engine::DispatchError;
use serde_json::{Value, json};

use crate::OrbitRuntime;

use super::backlog_exclusion::{
    EpicFamilyMembership, epic_family_membership, list_backlog_tasks, sort_tasks_by_priority_age,
};

/// The job that supervises one epic root. `classify_workspace_auto_tasks`
/// reads its live runs to decide whether another root may start.
const EPIC_JOB_NAME: &str = "epic_pipeline";

/// Longest drain window a caller may request, in seconds (24h). The window is
/// the caller's, not a safety property, but an unbounded deadline would let a
/// typo hold `workspace_auto_pipeline`'s single active-run slot indefinitely.
const MAX_DRAIN_WINDOW_SECONDS: f64 = 86_400.0;

/// The admissible work for one drain iteration [ORB-10819].
///
/// This answers "what may start right now", not "what is the one action for
/// this tick". Loose leaves and an epic root are independent answers: a
/// conflict-free chore ships in the same iteration that an epic is running,
/// because an `in-progress` epic root already reserves the union of its
/// descendants' `context_files` (ORB-10816) and `list_backlog_tasks` drops
/// exactly the leaves that overlap it. That reservation is why the former
/// `hold` decision is gone — a blanket freeze excluded conflict-free work the
/// lock surface had no reason to exclude.
pub(super) fn classify_workspace_auto_tasks(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<Value, DispatchError> {
    let backlog = list_backlog_tasks(runtime, action, input)?;
    let loose_task_ids = backlog
        .get("task_ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let active_epic = active_epic_run(runtime, action)?;
    let epic_task_id = match &active_epic {
        // One epic at a time. `epic_pipeline` declares `max_active_runs: 1`,
        // so offering a second root would not run it — it would queue a
        // `pending` run behind the live one, and the drain loop would mint a
        // fresh one every iteration. Keying on the run rather than on the
        // root's status also closes the window between a detached submit and
        // the child's `worktree_setup` moving that root to `in-progress`.
        Some(_) => None,
        None => next_admissible_epic_root(runtime, action)?,
    };

    let has_leaves = !loose_task_ids.is_empty();
    let has_epic = epic_task_id.is_some();
    Ok(json!({
        "loose_task_ids": loose_task_ids,
        "has_leaves": has_leaves,
        "epic_task_id": epic_task_id,
        "has_epic": has_epic,
        "empty": !has_leaves && !has_epic,
        "active_epic_run_id": active_epic.as_ref().map(|epic| epic.run_id.clone()),
        "active_epic_task_id": active_epic.and_then(|epic| epic.task_id),
    }))
}

/// A live `epic_pipeline` run, if one is already supervising a root.
struct ActiveEpicRun {
    run_id: String,
    task_id: Option<String>,
}

fn active_epic_run(
    runtime: &OrbitRuntime,
    action: &str,
) -> Result<Option<ActiveEpicRun>, DispatchError> {
    // Reconcile first, exactly as the submit path does before it counts
    // active runs. Without this, one orphaned `running` row — a worker killed
    // by a reboot or an OOM — would read as a live epic forever and silently
    // stop every epic dispatch in the workspace. That failure would be
    // invisible: the drain keeps succeeding, it just never starts an epic.
    runtime
        .reconcile_stale_job_runs(Some(EPIC_JOB_NAME))
        .map_err(|err| DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("reconcile stale {EPIC_JOB_NAME} runs: {err}"),
        })?;
    let runs = runtime
        .stores()
        .jobs()
        .list_pending_or_running_job_runs(EPIC_JOB_NAME)
        .map_err(|err| DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("list live {EPIC_JOB_NAME} runs: {err}"),
        })?;
    Ok(runs.into_iter().next().map(|run| ActiveEpicRun {
        task_id: run
            .input
            .as_ref()
            .and_then(|input| input.get("epic_task_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        run_id: run.run_id,
    }))
}

/// The highest-priority `backlog` epic root whose dependencies are satisfied.
fn next_admissible_epic_root(
    runtime: &OrbitRuntime,
    action: &str,
) -> Result<Option<String>, DispatchError> {
    let all_tasks = runtime.stores().tasks().list_tasks().map_err(|err| {
        DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("list tasks: {err}"),
        }
    })?;
    let task_lookup: BTreeMap<String, Task> = all_tasks
        .iter()
        .cloned()
        .map(|task| (task.id.clone(), task))
        .collect();
    let status_by_id =
        runtime
            .task_status_index()
            .map_err(|err| DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!("load global task status projection: {err}"),
            })?;

    let mut backlog_epics = all_tasks
        .into_iter()
        .filter(|task| {
            task.status == TaskStatus::Backlog
                && epic_family_membership(task, &task_lookup) == Some(EpicFamilyMembership::Root)
                && task_dependencies_ready(task, &status_by_id)
        })
        .collect::<Vec<_>>();
    sort_tasks_by_priority_age(&mut backlog_epics);
    Ok(backlog_epics.into_iter().next().map(|epic| epic.id))
}

/// Open or re-read a drain window [ORB-10819].
///
/// Two call shapes, one action. Called with `for_seconds` and no `deadline` it
/// *stamps*: the deadline is `now + for_seconds`, returned as RFC3339. Called
/// with that `deadline` echoed back it *answers*: whether the window has since
/// expired. The stamp therefore lives in the stamping step's own pipeline
/// output, which the run state already persists — no new durable artifact, and
/// re-reading the window is a pure function of a value the run carries.
///
/// A zero or absent window is expired on the first answer. `break_when` is
/// evaluated after a loop body runs, so that yields exactly one iteration:
/// the one-tick behavior every pre-window caller of `orbit run auto` has.
///
/// The deadline gates *starting* work. Nothing here cancels anything, which is
/// what makes "the window does not affect tasks already in progress" true by
/// construction: an in-flight child is held by `invoke_and_wait`, not by the
/// window.
pub(super) fn drain_window(action: &str, input: &Value) -> Result<Value, DispatchError> {
    let now = Utc::now();
    let deadline = match optional_deadline(action, input)? {
        Some(deadline) => deadline,
        None => {
            let for_seconds = window_seconds(action, input)?;
            now.checked_add_signed(seconds_to_delta(action, for_seconds)?)
                .ok_or_else(|| {
                    action_failed(
                        action,
                        format!("`for_seconds` {for_seconds} overflows the drain deadline"),
                    )
                })?
        }
    };

    let remaining_seconds = (deadline - now).num_milliseconds() as f64 / 1000.0;
    Ok(json!({
        "deadline": deadline.to_rfc3339_opts(SecondsFormat::Secs, true),
        "expired": remaining_seconds <= 0.0,
        "remaining_seconds": remaining_seconds.max(0.0),
    }))
}

fn optional_deadline(action: &str, input: &Value) -> Result<Option<DateTime<Utc>>, DispatchError> {
    let Some(raw) = input
        .get("deadline")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    DateTime::parse_from_rfc3339(raw)
        .map(|parsed| Some(parsed.with_timezone(&Utc)))
        .map_err(|err| action_failed(action, format!("`deadline` '{raw}' is not RFC3339: {err}")))
}

/// Read `for_seconds`, tolerating the string a template renders when the
/// caller supplied no window at all (`"{{ input.for_seconds }}"` over an
/// absent key resolves to an empty string, not to JSON `null`).
fn window_seconds(action: &str, input: &Value) -> Result<f64, DispatchError> {
    let Some(raw) = input.get("for_seconds") else {
        return Ok(0.0);
    };
    let seconds = match raw {
        Value::Null => 0.0,
        Value::Number(number) => number
            .as_f64()
            .ok_or_else(|| action_failed(action, "`for_seconds` is not a finite number".into()))?,
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                0.0
            } else {
                text.parse::<f64>().map_err(|err| {
                    action_failed(
                        action,
                        format!("`for_seconds` '{text}' is not a number: {err}"),
                    )
                })?
            }
        }
        other => {
            return Err(action_failed(
                action,
                format!("`for_seconds` must be a number, got {other}"),
            ));
        }
    };
    if !seconds.is_finite() || !(0.0..=MAX_DRAIN_WINDOW_SECONDS).contains(&seconds) {
        return Err(action_failed(
            action,
            format!("`for_seconds` must be between 0 and {MAX_DRAIN_WINDOW_SECONDS}"),
        ));
    }
    Ok(seconds)
}

fn seconds_to_delta(action: &str, seconds: f64) -> Result<TimeDelta, DispatchError> {
    TimeDelta::try_milliseconds((seconds * 1000.0).round() as i64)
        .ok_or_else(|| action_failed(action, format!("`for_seconds` {seconds} is out of range")))
}

fn action_failed(action: &str, message: String) -> DispatchError {
    DispatchError::DeterministicActionFailed {
        action: action.to_string(),
        message,
    }
}

pub(super) fn list_epic_descendants(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<Value, DispatchError> {
    let epic_task_id = input
        .get("epic_task_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: "missing `epic_task_id`".to_string(),
        })?;
    let all_tasks = runtime.stores().tasks().list_tasks().map_err(|error| {
        DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("list tasks: {error}"),
        }
    })?;
    let task_lookup = all_tasks
        .iter()
        .cloned()
        .map(|task| (task.id.clone(), task))
        .collect::<BTreeMap<_, _>>();
    let epic =
        task_lookup
            .get(epic_task_id)
            .ok_or_else(|| DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!("epic task `{epic_task_id}` was not found"),
            })?;
    if !epic.tags.iter().any(|tag| tag == "epic") {
        return Err(DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("task `{epic_task_id}` is not tagged `epic`"),
        });
    }

    let mut remaining = all_tasks
        .into_iter()
        .filter(|task| {
            is_descendant_of(task, epic_task_id, &task_lookup)
                && !matches!(
                    task.status,
                    TaskStatus::Done | TaskStatus::Rejected | TaskStatus::Archived
                )
        })
        .map(|task| (task.id.clone(), task))
        .collect::<BTreeMap<_, _>>();
    let mut ordered = Vec::with_capacity(remaining.len());

    while !remaining.is_empty() {
        let remaining_ids = remaining.keys().cloned().collect::<BTreeSet<_>>();
        let mut ready = remaining
            .values()
            .filter(|task| {
                task.dependencies()
                    .iter()
                    .all(|dependency_id| !remaining_ids.contains(dependency_id))
            })
            .cloned()
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Err(DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!(
                    "epic `{epic_task_id}` has a dependency cycle among unfinished descendants: {}",
                    remaining_ids.into_iter().collect::<Vec<_>>().join(", ")
                ),
            });
        }
        sort_tasks_by_priority_age(&mut ready);
        for task in ready {
            remaining.remove(&task.id);
            ordered.push(task.id);
        }
    }

    let empty = ordered.is_empty();
    let fail_if_nonempty = input
        .get("fail_if_nonempty")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if fail_if_nonempty && !empty {
        return Err(DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!(
                "epic descendants remain after drain: tasks=[{}]",
                ordered.join(", ")
            ),
        });
    }

    Ok(json!({
        "epic_task_id": epic_task_id,
        "task_count": ordered.len(),
        "task_ids": ordered,
        "empty": empty,
    }))
}

fn is_descendant_of(task: &Task, ancestor_id: &str, task_lookup: &BTreeMap<String, Task>) -> bool {
    let mut visited = BTreeSet::from([task.id.clone()]);
    let mut next_parent_id = task.parent_id();
    for _ in 0..32 {
        let Some(parent_id) = next_parent_id else {
            return false;
        };
        if parent_id == ancestor_id {
            return true;
        }
        if !visited.insert(parent_id.to_string()) {
            return false;
        }
        let Some(parent) = task_lookup.get(parent_id) else {
            return false;
        };
        next_parent_id = parent.parent_id();
    }
    false
}
