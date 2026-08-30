//! `classify_ci_evidence` — deterministic triage of one evidence snapshot.
//!
//! Three outcomes, and the whole point of the stage is that they never
//! collapse into each other. "We could not look" is not "nothing is failing",
//! and neither of those is "here is what to repair".
//!
//! Two of the three end without an agent run:
//!
//! - `capability_unavailable` blocks the bundle with the exact preflight
//!   detail and then fails the step, which ends the run before anything can be
//!   dispatched. Failing is the honest report: the host was asked to find out
//!   whether CI is red and could not.
//! - `no_current_failure` persists the evidenced account as the task's durable
//!   `execution_summary` and lets the run continue into `git_commit`. There is
//!   nothing to commit, the task carries `no-diff-expected`, and the ordinary
//!   no-diff promotion route takes it from there — unchanged, and with no
//!   agent in between.

use orbit_common::OrbitError;
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate};

use super::{
    CI_CAPABILITY_UNAVAILABLE_EVENT, OUTCOME_CAPABILITY_UNAVAILABLE, OUTCOME_CURRENT_FAILURES,
    OUTCOME_NO_CURRENT_FAILURE, block_tasks, task_ids_from_input,
};

/// Task event recorded when triage, not an agent, authored the summary.
const NO_CURRENT_FAILURE_EVENT: &str = "ci_no_current_failure";

pub(super) fn classify<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let evidence = input.get("ci_evidence").ok_or_else(|| {
        OrbitError::InvalidInput(
            "classify_ci_evidence requires input.ci_evidence from collect_ci_evidence".to_string(),
        )
    })?;
    let capability = evidence.get("capability").cloned().unwrap_or(Value::Null);
    let task_ids = task_ids_from_input(input);

    if !evidence
        .get("collected")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let note = capability_unavailable_note(&capability);
        block_tasks(host, &task_ids, CI_CAPABILITY_UNAVAILABLE_EVENT, &note)?;
        return Err(OrbitError::Execution(note));
    }

    let current_failures = evidence
        .get("current_failures")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    if current_failures.is_empty() {
        let summary = no_current_failure_summary(evidence);
        for task_id in &task_ids {
            host.apply_task_automation_update(
                task_id,
                TaskAutomationUpdate {
                    execution_summary: Some(summary.clone()),
                    status_event: Some(NO_CURRENT_FAILURE_EVENT.to_string()),
                    status_note: Some(
                        "automation: CI evidence shows no current, non-superseded failure"
                            .to_string(),
                    ),
                    ..TaskAutomationUpdate::default()
                },
            )?;
        }
        return Ok(json!({
            "phase": "classify_ci_evidence",
            "outcome": OUTCOME_NO_CURRENT_FAILURE,
            "current_failure_count": 0,
            "capability": capability,
            "affected_workflows": Vec::<String>::new(),
            "summarized_task_ids": task_ids,
            "detail": summary,
        }));
    }

    Ok(json!({
        "phase": "classify_ci_evidence",
        "outcome": OUTCOME_CURRENT_FAILURES,
        "current_failure_count": current_failures.len(),
        "capability": capability,
        // The workflows a candidate commit has to come back green on. A repair
        // that never re-runs an affected workflow is not a verified repair, so
        // this list travels to `verify_candidate_ci`.
        "affected_workflows": affected_workflows(current_failures),
        "summarized_task_ids": Vec::<String>::new(),
        "detail": format!(
            "{} current failure(s) require repair across workflow(s): {}.",
            current_failures.len(),
            affected_workflows(current_failures).join(", "),
        ),
    }))
}

fn capability_unavailable_note(capability: &Value) -> String {
    let detail = capability
        .get("detail")
        .and_then(Value::as_str)
        .unwrap_or("no preflight detail was recorded");
    let available = capability
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let authenticated = capability
        .get("authenticated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    format!(
        "{OUTCOME_CAPABILITY_UNAVAILABLE}: the GitHub capability preflight failed \
         (available={available}, authenticated={authenticated}). Preflight detail: {detail}. \
         No CI evidence was gathered and no agent was launched. This is not a \
         {OUTCOME_NO_CURRENT_FAILURE} result and must not be reported as one."
    )
}

/// The durable account of an evidenced clean pass.
///
/// Written as the task's `execution_summary` because that field, not this
/// step's output, is what delivery and every later reader consult. It names
/// the outcome explicitly so it can never be confused with the
/// capability-unavailable ending.
fn no_current_failure_summary(evidence: &Value) -> String {
    let heads = evidence
        .get("heads")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let head_lines: Vec<String> = heads
        .iter()
        .map(|head| {
            format!(
                "- {} `{}` at {}",
                head.get("kind").and_then(Value::as_str).unwrap_or("ref"),
                head.get("branch").and_then(Value::as_str).unwrap_or("?"),
                head.get("current_head_sha")
                    .and_then(Value::as_str)
                    .unwrap_or("unresolved"),
            )
        })
        .collect();
    let stale = evidence
        .get("stale_or_superseded")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let stale_lines: Vec<String> = stale
        .iter()
        .map(|entry| {
            format!(
                "- {} ({}): {}",
                entry.get("url").and_then(Value::as_str).unwrap_or("?"),
                entry.get("reason").and_then(Value::as_str).unwrap_or("?"),
                entry
                    .get("evidence")
                    .and_then(Value::as_str)
                    .unwrap_or("superseded by a later successful run at the same commit"),
            )
        })
        .collect();
    let in_flight = evidence
        .get("in_flight")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    format!(
        "Outcome: {OUTCOME_NO_CURRENT_FAILURE}\n\n\
         Host-owned CI discovery ran with an authenticated GitHub client and found no current, \
         non-superseded failure. This is an evidenced clean pass, not a \
         {OUTCOME_CAPABILITY_UNAVAILABLE} one.\n\n\
         Heads queried ({head_count}):\n{heads}\n\n\
         Stale or superseded failures ({stale_count}):\n{stale}\n\n\
         Runs still in flight at a current head: {in_flight}.\n\
         Discovery bounds: {truncation}\n",
        head_count = heads.len(),
        heads = if head_lines.is_empty() {
            "- none".to_string()
        } else {
            head_lines.join("\n")
        },
        stale_count = stale.len(),
        stale = if stale_lines.is_empty() {
            "- none".to_string()
        } else {
            stale_lines.join("\n")
        },
        truncation = evidence
            .get("truncation")
            .map(ToString::to_string)
            .unwrap_or_else(|| "{}".to_string()),
    )
}

fn affected_workflows(failures: &[Value]) -> Vec<String> {
    let mut workflows: Vec<String> = Vec::new();
    for failure in failures {
        let Some(workflow) = failure
            .get("workflow")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if !workflows.iter().any(|known| known == workflow) {
            workflows.push(workflow.to_string());
        }
    }
    workflows.sort();
    workflows
}
