//! CI remediation: host-owned discovery, triage, and candidate verification.
//!
//! These three stages exist because the two halves of CI remediation are
//! structurally impossible where an implementation agent runs. Discovery needs
//! GitHub credentials inside a sandbox that denies them; post-publication
//! verification needs a candidate commit that does not exist yet while the
//! agent is running. Both belong on the host.
//!
//! They sit on the same engine-private automation boundary as
//! `automation::vcs`: no new agent-facing tool, no activity-level filesystem
//! exception, and no GitHub credential visible from inside the sandbox. What
//! crosses into the sandbox is one bounded, redacted JSON snapshot and nothing
//! else — no token, no host configuration, no caller-selected environment, and
//! no way to run another query.
//!
//! The three endings the remediation contract is careful about stay distinct
//! here and are never allowed to collapse into one another or into a clean
//! pass: [`OUTCOME_CAPABILITY_UNAVAILABLE`] (we could not look),
//! [`OUTCOME_NO_CURRENT_FAILURE`] (we looked and nothing is failing), and
//! [`OUTCOME_CURRENT_FAILURES`] (we looked and something is).

mod classify;
mod collect;
mod query;
mod verify;

use std::path::PathBuf;

use orbit_common::OrbitError;
use orbit_types::task::TaskStatus;
use serde_json::Value;

use crate::context::{MAX_NOTE_ERROR_BYTES, RuntimeHost, TaskAutomationUpdate};

use super::input::{canonicalize_existing_dir, input_string_field};

/// A GitHub client was absent or unauthenticated on this host, so no CI
/// evidence was gathered. Never a clean bill of health.
pub(crate) const OUTCOME_CAPABILITY_UNAVAILABLE: &str = "capability_unavailable";
/// The queries ran and found no current, non-superseded failure.
pub(crate) const OUTCOME_NO_CURRENT_FAILURE: &str = "no_current_failure";
/// The queries ran and found at least one current failure to repair.
pub(crate) const OUTCOME_CURRENT_FAILURES: &str = "current_failures";

/// Status event recorded when triage stops the pipeline before any agent runs.
const CI_CAPABILITY_UNAVAILABLE_EVENT: &str = "ci_capability_unavailable";
/// Status event recorded when a published candidate does not come back green.
const CI_CANDIDATE_NOT_GREEN_EVENT: &str = "ci_candidate_not_green";

/// Run conclusions that are not a pass.
///
/// `cancelled` and `timed_out` are in here deliberately: a run that never
/// produced a verdict is not a green run, and treating it as one is exactly
/// how a red pipeline gets reported as clean.
pub(super) fn unsuccessful_conclusion(conclusion: Option<&str>) -> bool {
    matches!(
        conclusion,
        Some("failure" | "cancelled" | "timed_out" | "action_required" | "startup_failure")
    )
}

pub(super) fn optional_input_string(input: &Value, key: &str) -> Option<String> {
    input_string_field(input, key)
}

/// Read an optional bound, clamped to `max`.
///
/// Zero is a legitimate value, not a mistake: `max_wait_seconds: 0` is how a
/// caller asks `verify_candidate_ci` for a single non-waiting poll.
pub(super) fn bounded_u64(
    input: &Value,
    key: &str,
    default: u64,
    max: u64,
) -> Result<u64, OrbitError> {
    let Some(value) = input.get(key).filter(|value| !value.is_null()) else {
        return Ok(default);
    };
    let raw = match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
    .ok_or_else(|| {
        OrbitError::InvalidInput(format!("input.{key} must be a non-negative integer"))
    })?;
    Ok(raw.min(max))
}

/// Task ids this stage acts on, accepting either shipped spelling.
pub(super) fn task_ids_from_input(input: &Value) -> Vec<String> {
    for key in ["completed_task_ids", "task_ids"] {
        let ids: Vec<String> = input
            .get(key)
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if !ids.is_empty() {
            return ids;
        }
    }
    Vec::new()
}

/// Statuses left alone when a CI stage blocks a bundle.
///
/// Mirrors the run-failure block: `Done`/`Archived`/`Rejected` are terminal or
/// human decisions and `Blocked` is already where we want it, which keeps the
/// transition idempotent under a resume.
fn blockable(status: TaskStatus) -> bool {
    !matches!(
        status,
        TaskStatus::Done | TaskStatus::Blocked | TaskStatus::Rejected | TaskStatus::Archived
    )
}

/// Move a bundle to `blocked` with one bounded, structured note.
///
/// `blocked` is the deliberate dead end for automation: it is not in the
/// workflow-admission allowlist, so the sweep will not pick these tasks up
/// again until a human or orchestrator looks. The note is capped at the same
/// bound the workflow-failure note uses — a durable status note must carry the
/// verdict, not a log.
fn block_tasks<H: RuntimeHost + ?Sized>(
    host: &H,
    task_ids: &[String],
    event: &str,
    note: &str,
) -> Result<Vec<String>, OrbitError> {
    let note = bound_note(note);
    let mut blocked = Vec::new();
    for task_id in task_ids {
        let task = host.get_task(task_id)?;
        if !blockable(task.status) {
            continue;
        }
        host.apply_task_automation_update(
            task_id,
            TaskAutomationUpdate {
                status: Some(TaskStatus::Blocked),
                status_event: Some(event.to_string()),
                status_note: Some(note.clone()),
                ..TaskAutomationUpdate::default()
            },
        )?;
        blocked.push(task_id.clone());
    }
    Ok(blocked)
}

fn bound_note(note: &str) -> String {
    if note.len() <= MAX_NOTE_ERROR_BYTES {
        return note.to_string();
    }
    let mut end = MAX_NOTE_ERROR_BYTES;
    while end > 0 && !note.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}… [elided: note is {} B; the full evidence is the step output of this run]",
        &note[..end],
        note.len()
    )
}

/// The checkout these stages query from: the run's worktree when it has one,
/// otherwise the workspace repository root. Both resolve to the same remote.
fn query_root<H: RuntimeHost + ?Sized>(host: &H, input: &Value) -> Result<PathBuf, OrbitError> {
    match input_string_field(input, "workspace_path") {
        Some(path) => canonicalize_existing_dir(&path, "workspace_path"),
        None => Ok(PathBuf::from(host.repo_root()?)),
    }
}

pub(super) fn collect_ci_evidence<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let queries = query::HostCiQueries::new(&query_root(host, input)?);
    let evidence = collect::collect(&queries, input)?;
    Ok(serde_json::json!({
        "phase": "collect_ci_evidence",
        "ci_evidence": evidence,
    }))
}

pub(super) fn classify_ci_evidence<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    classify::classify(host, input)
}

pub(super) fn verify_candidate_ci<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let queries = query::HostCiQueries::new(&query_root(host, input)?);
    verify::verify(host, &queries, &verify::RealWaiter, input)
}

#[cfg(test)]
mod tests;
