use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use orbit_common::types::{
    AuditEventStatus, Role, TaskComment, TaskStatus, audit_execution_id, optional_string_list_alias,
};
use orbit_engine::{DispatchError, ensure_task_can_enter_workflow};
use orbit_store::AuditEventInsertParams;
use orbit_tools::ToolContext;
use serde_json::Value;

use crate::OrbitRuntime;
use crate::command::job::JobRunListParams;
use crate::runtime::task_locks::parse_task_ids;

pub(super) fn validate_bundles(action: &str, input: &Value) -> Result<Value, DispatchError> {
    let bundles_raw = input
        .get("bundles")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: "`bundles` must be an array".to_string(),
        })?;
    let max_bundle_size = input
        .get("max_bundle_size")
        .and_then(Value::as_u64)
        .unwrap_or(5) as usize;
    let known: std::collections::BTreeSet<String> = input
        .get("known_task_ids")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();

    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut violations: Vec<String> = Vec::new();
    let mut bundles: Vec<Vec<String>> = Vec::with_capacity(bundles_raw.len());
    for (idx, bundle) in bundles_raw.iter().enumerate() {
        let items = bundle
            .as_array()
            .ok_or_else(|| DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!("bundle[{idx}] is not an array"),
            })?;
        if items.len() > max_bundle_size {
            violations.push(format!(
                "bundle[{idx}] size {} exceeds max_bundle_size {}",
                items.len(),
                max_bundle_size
            ));
        }
        let mut bundle_ids: Vec<String> = Vec::with_capacity(items.len());
        for item in items {
            let id = item
                .as_str()
                .ok_or_else(|| DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: format!("bundle[{idx}] contains a non-string task_id"),
                })?;
            if !known.is_empty() && !known.contains(id) {
                violations.push(format!("bundle[{idx}] references unknown task_id {id}"));
            }
            if !seen.insert(id.to_string()) {
                violations.push(format!("task_id {id} appears in more than one bundle"));
            }
            bundle_ids.push(id.to_string());
        }
        bundles.push(bundle_ids);
    }
    if !violations.is_empty() {
        return Err(DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("invalid bundles: {}", violations.join("; ")),
        });
    }
    Ok(serde_json::json!({
        "bundles": bundles,
        "bundle_count": bundles.len(),
    }))
}

pub(super) fn invoke_and_wait(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
    tool_context: ToolContext,
) -> Result<Value, DispatchError> {
    if let Some(noop) = stale_gate_admission_noop(runtime, action, input)? {
        return Ok(noop);
    }

    let job_name = input
        .get("job_name")
        .and_then(Value::as_str)
        .ok_or_else(|| DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: "missing `job_name`".to_string(),
        })?
        .to_string();
    let run_input = input
        .get("run_input")
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    let run_id = match deduped_child_run_id(runtime, action, input, &job_name, &run_input)? {
        Some(run_id) => run_id,
        None => {
            let mut invoke_args = serde_json::Map::new();
            invoke_args.insert("job_name".to_string(), Value::String(job_name.clone()));
            invoke_args.insert("input".to_string(), run_input);
            if let Some(priority) = input.get("priority").cloned() {
                invoke_args.insert("priority".to_string(), priority);
            }

            let invoke_ctx = tool_context.clone();
            let invoke_output = runtime
                .run_tool_with_context_and_role(
                    "orbit.pipeline.invoke",
                    Value::Object(invoke_args),
                    Role::Admin,
                    invoke_ctx,
                )
                .map_err(|err| DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: format!("pipeline.invoke failed: {err}"),
                })?;

            invoke_output
                .get("run_id")
                .and_then(Value::as_str)
                .ok_or_else(|| DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: "pipeline.invoke returned no run_id".to_string(),
                })?
                .to_string()
        }
    };

    let mut wait_args = serde_json::Map::new();
    wait_args.insert(
        "run_ids".to_string(),
        Value::Array(vec![Value::String(run_id.clone())]),
    );
    if let Some(timeout) = input.get("timeout_seconds").cloned() {
        wait_args.insert("timeout_seconds".to_string(), timeout);
    }
    if let Some(poll) = input.get("poll_interval_seconds").cloned() {
        wait_args.insert("poll_interval_seconds".to_string(), poll);
    }

    let wait_output = runtime
        .run_tool_with_context_and_role(
            "orbit.pipeline.wait",
            Value::Object(wait_args),
            Role::Admin,
            tool_context,
        )
        .map_err(|err| DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("pipeline.wait failed: {err}"),
        })?;

    let first = wait_output
        .get("results")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .cloned()
        .unwrap_or_else(|| {
            serde_json::json!({
                "run_id": run_id,
                "status": "pending",
            })
        });
    Ok(first)
}

// `pub(super)` (not private): the sibling `tests/pipeline_actions.rs`
// unit-tests dedupe behavior directly — see
// docs/design-patterns/test_layout.md migration recipe step 6.
pub(super) fn deduped_child_run_id(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
    job_name: &str,
    run_input: &Value,
) -> Result<Option<String>, DispatchError> {
    let Some(field) = input
        .get("dedupe_run_input_field")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let key = run_input
        .get(field)
        .filter(|value| !value.is_null())
        .ok_or_else(|| {
            action_failed(
                action,
                format!("dedupe field '{field}' is missing from run_input"),
            )
        })?;
    let runs = runtime
        .list_job_runs(JobRunListParams {
            job_id: Some(job_name.to_string()),
            ..JobRunListParams::default()
        })
        .map_err(|error| {
            action_failed(
                action,
                format!("list existing {job_name} Runs for dedupe: {error}"),
            )
        })?;
    let matches = runs
        .into_iter()
        .filter(|run| {
            run.input
                .as_ref()
                .and_then(|value| value.get(field))
                .is_some_and(|value| value == key)
        })
        .map(|run| run.run_id)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [run_id] => Ok(Some(run_id.clone())),
        _ => Err(action_failed(
            action,
            format!(
                "multiple {job_name} Runs match dedupe field '{field}' value {key}: {}",
                matches.join(", ")
            ),
        )),
    }
}

fn stale_gate_admission_noop(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<Option<Value>, DispatchError> {
    let raw_task_ids = optional_string_list_alias(
        input,
        &[
            "admission_task_ids",
            "admissionTaskIds",
            "admission-task-ids",
        ],
    )
    .map_err(|err| action_failed(action, err.to_string()))?;
    let Some(raw_task_ids) = raw_task_ids else {
        return Ok(None);
    };
    let task_ids = parse_task_ids(&serde_json::json!({ "task_ids": raw_task_ids }))
        .map_err(|err| action_failed(action, err.to_string()))?;
    let workflow = input
        .get("admission_workflow")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("worktree_setup");

    let mut task_statuses = Vec::with_capacity(task_ids.len());
    let mut stale_statuses = Vec::new();
    let mut admission_errors = Vec::new();

    for task_id in &task_ids {
        match ensure_task_can_enter_workflow(runtime, task_id, workflow) {
            Ok(task) => {
                task_statuses.push(serde_json::json!({
                    "task_id": task.id,
                    "status": task.status.to_string(),
                    "admissible": true,
                }));
            }
            Err(error) => match runtime.get_task(task_id) {
                Ok(task) => {
                    let status = task.status;
                    task_statuses.push(serde_json::json!({
                        "task_id": task.id,
                        "status": status.to_string(),
                        "admissible": false,
                    }));
                    if matches!(status, TaskStatus::Review | TaskStatus::Done) {
                        stale_statuses.push((task_id.clone(), status.to_string()));
                    } else {
                        admission_errors.push(error.to_string());
                    }
                }
                Err(_) => admission_errors.push(error.to_string()),
            },
        }
    }

    if !admission_errors.is_empty() {
        return Err(action_failed(
            action,
            format!(
                "workflow admission check before child dispatch failed: {}",
                admission_errors.join("; ")
            ),
        ));
    }

    if stale_statuses.is_empty() {
        return Ok(None);
    }

    let status_summary = stale_statuses
        .iter()
        .map(|(task_id, status)| format!("{task_id}={status}"))
        .collect::<Vec<_>>()
        .join(", ");
    let reason = format!(
        "task_gate_pipeline stale/no-op: workflow admission for '{workflow}' skipped child dispatch because {status_summary}"
    );
    record_gate_stale_noop(runtime, action, input, &task_ids, &task_statuses, &reason)?;
    let parent_run_id = input
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");

    Ok(Some(serde_json::json!({
        "status": "succeeded",
        "run_id": format!("stale-noop-{parent_run_id}"),
        "skipped": true,
        "reason": reason,
        "task_statuses": task_statuses,
    })))
}

fn record_gate_stale_noop(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
    task_ids: &[String],
    task_statuses: &[Value],
    reason: &str,
) -> Result<(), DispatchError> {
    let parent_run_id = input
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let payload = serde_json::json!({
        "task_ids": task_ids,
        "task_statuses": task_statuses,
        "reason": reason,
        "parent_run_id": parent_run_id,
    });
    let arguments_json = serde_json::to_string(&payload).map_err(|err| {
        action_failed(action, format!("serialize gate.stale_noop payload: {err}"))
    })?;
    let execution_id = audit_execution_id("audit-gate-stale-noop");
    let working_directory = runtime.paths().repo_root.to_string_lossy().into_owned();

    runtime
        .record_audit_event(&AuditEventInsertParams {
            execution_id,
            command: "gate.stale_noop".to_string(),
            subcommand: None,
            tool_name: None,
            target_type: Some("task_bundle".to_string()),
            target_id: task_ids.first().cloned(),
            role: "admin".to_string(),
            status: AuditEventStatus::Success,
            exit_code: 0,
            duration_ms: 0,
            working_directory,
            arguments_json: Some(arguments_json),
            stdout_truncated: None,
            stderr_truncated: None,
            error_message: None,
            host: std::env::var("HOSTNAME").ok(),
            pid: std::process::id(),
            session_id: None,
            workspace_id: None,
            caller_machine_id: None,
            caller_host_id: None,
            process_machine_id: None,
            process_host_id: None,
            transport: None,
            effective_capabilities: Default::default(),
            origin_session_id: None,
            mcp_call_id: None,
            lease_id: None,
            task_id: task_ids.first().cloned(),
            job_run_id: parent_run_id,
            activity_id: None,
            step_index: None,
        })
        .map_err(|err| action_failed(action, format!("record gate.stale_noop audit: {err}")))
}

pub(super) fn pipeline_success_guard(action: &str, input: &Value) -> Result<Value, DispatchError> {
    let context = input
        .get("context")
        .and_then(Value::as_str)
        .unwrap_or("pipeline child run");
    if ["review_step", "verdict_step", "required_verdict"]
        .iter()
        .any(|field| input.get(*field).is_some())
    {
        return review_pipeline_success_guard(action, input, context);
    }
    let mut checked_count = 0usize;
    let mut failures = Vec::new();

    if let Some(result) = input.get("result")
        && !result.is_null()
    {
        checked_count += 1;
        if let Some(failure) = pipeline_wait_entry_failure("result", result) {
            failures.push(failure);
        }
    }

    if let Some(results) = input.get("results")
        && !results.is_null()
    {
        let entries =
            results
                .as_array()
                .ok_or_else(|| DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: "`results` must be an array".to_string(),
                })?;
        for (idx, entry) in entries.iter().enumerate() {
            checked_count += 1;
            if let Some(failure) = pipeline_wait_entry_failure(&format!("results[{idx}]"), entry) {
                failures.push(failure);
            }
        }
    }

    if checked_count == 0 {
        return Err(DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: "expected `result` or `results` to check".to_string(),
        });
    }

    if !failures.is_empty() {
        return Err(DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("{context} did not succeed: {}", failures.join("; ")),
        });
    }

    Ok(serde_json::json!({
        "succeeded": true,
        "checked_count": checked_count,
    }))
}

/// Review-aware specialization of the generic child-run gate.
///
/// A successful step checkpoint is the durable boundary between "the reviewer
/// never produced a review" and "review ran but did not pass". This keeps the
/// classification independent of provider-specific error prose while still
/// preserving that prose in the parent diagnostic.
fn review_pipeline_success_guard(
    action: &str,
    input: &Value,
    context: &str,
) -> Result<Value, DispatchError> {
    let review_step = non_blank_str(input, "review_step")
        .ok_or_else(|| action_failed(action, "missing non-blank `review_step`".to_string()))?;
    let verdict_step = non_blank_str(input, "verdict_step")
        .ok_or_else(|| action_failed(action, "missing non-blank `verdict_step`".to_string()))?;
    let required_verdict = non_blank_str(input, "required_verdict")
        .ok_or_else(|| action_failed(action, "missing non-blank `required_verdict`".to_string()))?;
    let result = input
        .get("result")
        .filter(|value| !value.is_null())
        .ok_or_else(|| {
            action_failed(
                action,
                "review-aware guard requires one non-null `result`".to_string(),
            )
        })?;
    let status = result
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| action_failed(action, "result missing string status".to_string()))?;
    let run_id = result
        .get("run_id")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let pipeline = result.get("pipeline").and_then(Value::as_object);
    let review_checkpoint = pipeline
        .and_then(|pipeline| pipeline.get(review_step))
        .filter(|value| !value.is_null());

    if status != "succeeded" {
        let failure = pipeline_wait_entry_failure("result", result)
            .unwrap_or_else(|| format!("result run {run_id} status {status}"));
        if review_checkpoint.is_none() {
            return Err(DispatchError::IndependentReviewNotStarted {
                diagnostic: format!("{context}: {failure}"),
            });
        }
        return Err(action_failed(
            action,
            format!("{context} ran but did not pass: {failure}"),
        ));
    }

    let verdict = pipeline
        .and_then(|pipeline| pipeline.get(verdict_step))
        .and_then(|checkpoint| checkpoint.get("verdict"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            action_failed(
                action,
                format!(
                    "{context} ran but did not produce a durable verdict in child run {run_id} checkpoint `{verdict_step}`"
                ),
            )
        })?;
    if verdict != required_verdict {
        return Err(action_failed(
            action,
            format!(
                "{context} ran and returned verdict '{verdict}' in child run {run_id} (required '{required_verdict}'); review findings are persisted on the task"
            ),
        ));
    }

    Ok(serde_json::json!({
        "succeeded": true,
        "checked_count": 1,
    }))
}

/// Prefix of the durable review record an independent reviewer appends to each
/// participating task as a comment.
///
/// The reviewer's structured response is advisory, so the per-criterion verdict
/// it claims cannot be the thing this guard checks. The record below is durable
/// task state: it survives the run, is readable by anyone later, and is what the
/// guard reads. The marker also partitions comments — a comment carrying it is a
/// review record, and every other comment is task authority the reviewer must
/// have reconciled before approving.
pub(super) const REVIEW_RECORD_MARKER: &str = "[independent-review]";

/// One reviewer-persisted per-criterion record, recovered from durable task
/// comments rather than from the agent's response.
struct ReviewRecord {
    task_id: String,
    verdict: String,
    reconciled_through: DateTime<Utc>,
    /// 1-based acceptance-criterion index → the reviewer's verdict for it.
    criteria: BTreeMap<u64, String>,
    late_corrections: usize,
}

pub(super) fn independent_review_guard(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<Value, DispatchError> {
    let reported_verdict = match non_blank_str(input, "verdict") {
        Some(verdict) => {
            if !matches!(verdict, "approve" | "request_changes") {
                return Err(action_failed(
                    action,
                    format!(
                        "unknown independent review verdict '{verdict}' (expected 'approve' or 'request_changes')"
                    ),
                ));
            }
            Some(verdict.to_string())
        }
        None => None,
    };

    let reviewed_head_sha = non_blank_str(input, "reviewed_head_sha").ok_or_else(|| {
        action_failed(action, "missing non-blank `reviewed_head_sha`".to_string())
    })?;
    let candidate_head_sha = non_blank_str(input, "candidate_head_sha").ok_or_else(|| {
        action_failed(action, "missing non-blank `candidate_head_sha`".to_string())
    })?;
    if reviewed_head_sha != candidate_head_sha {
        return Err(action_failed(
            action,
            format!(
                "independent review head mismatch: reviewer reported '{reviewed_head_sha}', published candidate is '{candidate_head_sha}'"
            ),
        ));
    }

    // The bundle comes from the parent pipeline's own input, never from the
    // reviewer, so the set of criteria that must be covered cannot be narrowed
    // by the thing being checked.
    let task_ids = parse_task_ids(input).map_err(|error| {
        action_failed(
            action,
            format!("independent review guard cannot resolve its task bundle: {error}"),
        )
    })?;

    let mut records = Vec::with_capacity(task_ids.len());
    for task_id in &task_ids {
        let comments = runtime.get_task_comments(task_id).map_err(|error| {
            action_failed(action, format!("read comments for task {task_id}: {error}"))
        })?;
        records.push((
            latest_review_record(action, task_id, candidate_head_sha, &comments)?,
            latest_task_authority(&comments),
        ));
    }

    // Any task whose durable record withholds approval decides the bundle.
    let durable_verdict = if records
        .iter()
        .any(|(record, _)| record.verdict == "request_changes")
    {
        "request_changes"
    } else {
        "approve"
    };
    if let Some(reported) = reported_verdict.as_deref()
        && reported != durable_verdict
    {
        return Err(action_failed(
            action,
            format!(
                "independent review verdict mismatch: response reported '{reported}', persisted review records say '{durable_verdict}'"
            ),
        ));
    }

    if durable_verdict == "approve" {
        let mut violations = Vec::new();
        for (record, latest_authority) in &records {
            let task = runtime.get_task(&record.task_id).map_err(|error| {
                action_failed(action, format!("read task {}: {error}", record.task_id))
            })?;
            violations.extend(approval_violations(
                record,
                task.acceptance_criteria.len(),
                *latest_authority,
            ));
        }
        if !violations.is_empty() {
            return Err(action_failed(
                action,
                format!(
                    "independent review approval is not admissible: {}",
                    violations.join("; ")
                ),
            ));
        }
    }

    let criteria_covered: usize = records
        .iter()
        .map(|(record, _)| record.criteria.len())
        .sum();
    let tasks_reviewed = records
        .iter()
        .map(|(record, _)| {
            serde_json::json!({
                "task_id": record.task_id,
                "verdict": record.verdict,
                "criteria_covered": record.criteria.len(),
                "reconciled_through": record.reconciled_through.to_rfc3339(),
                "late_corrections": record.late_corrections,
            })
        })
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "verdict": durable_verdict,
        "reviewed_head_sha": reviewed_head_sha,
        "exact_head": true,
        "criteria_covered": criteria_covered,
        "tasks_reviewed": tasks_reviewed,
    }))
}

/// Newest durable review record for `candidate_head_sha`.
///
/// Structurally invalid records are skipped rather than fatal: a reviewer that
/// re-runs after a rejected record must be able to supersede it, and a record
/// that named an older candidate belongs to an older review.
fn latest_review_record(
    action: &str,
    task_id: &str,
    candidate_head_sha: &str,
    comments: &[TaskComment],
) -> Result<ReviewRecord, DispatchError> {
    let mut rejected: Vec<String> = Vec::new();
    let mut latest: Option<(DateTime<Utc>, ReviewRecord)> = None;

    for comment in comments {
        let Some(payload) = review_record_payload(&comment.message) else {
            continue;
        };
        let payload = match payload {
            Ok(payload) => payload,
            Err(error) => {
                rejected.push(format!("unparsable record at {}: {error}", comment.at));
                continue;
            }
        };
        if payload
            .get("candidate_head_sha")
            .and_then(Value::as_str)
            .map(str::trim)
            != Some(candidate_head_sha)
        {
            continue;
        }
        match review_record_from_payload(task_id, &payload) {
            Ok(record) => {
                let supersedes = latest
                    .as_ref()
                    .map(|(at, _)| comment.at >= *at)
                    .unwrap_or(true);
                if supersedes {
                    latest = Some((comment.at, record));
                }
            }
            Err(error) => rejected.push(format!("invalid record at {}: {error}", comment.at)),
        }
    }

    match latest {
        Some((_, record)) => Ok(record),
        None => Err(action_failed(
            action,
            format!(
                "independent review persisted no `{REVIEW_RECORD_MARKER}` per-criterion record on task {task_id} for candidate {candidate_head_sha}{}",
                if rejected.is_empty() {
                    String::new()
                } else {
                    format!(" (rejected: {})", rejected.join(", "))
                }
            ),
        )),
    }
}

/// The JSON body of a review-record comment, or `None` when the comment is
/// ordinary task authority rather than a review record.
fn review_record_payload(message: &str) -> Option<Result<Value, String>> {
    let body = message.trim_start().strip_prefix(REVIEW_RECORD_MARKER)?;
    Some(serde_json::from_str::<Value>(body.trim()).map_err(|error| error.to_string()))
}

fn review_record_from_payload(task_id: &str, payload: &Value) -> Result<ReviewRecord, String> {
    let verdict = payload
        .get("verdict")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| matches!(*value, "approve" | "request_changes"))
        .ok_or_else(|| "`verdict` must be 'approve' or 'request_changes'".to_string())?;
    let reconciled_through = payload
        .get("reconciled_through")
        .and_then(Value::as_str)
        .map(str::trim)
        .ok_or_else(|| "`reconciled_through` is missing".to_string())?;
    let reconciled_through = DateTime::parse_from_rfc3339(reconciled_through)
        .map_err(|error| format!("`reconciled_through` is not an RFC 3339 timestamp: {error}"))?
        .with_timezone(&Utc);
    let late_corrections = payload
        .get("late_corrections")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "`late_corrections` must be an array (empty when the task carries none)".to_string()
        })?
        .len();

    let entries = payload
        .get("criteria")
        .and_then(Value::as_array)
        .ok_or_else(|| "`criteria` must be an array".to_string())?;
    let mut criteria = BTreeMap::new();
    for entry in entries {
        let index = entry
            .get("index")
            .and_then(Value::as_u64)
            .ok_or_else(|| "every `criteria` entry needs a 1-based integer `index`".to_string())?;
        let criterion_verdict = entry
            .get("verdict")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| matches!(*value, "met" | "not_met"))
            .ok_or_else(|| format!("criterion {index} verdict must be 'met' or 'not_met'"))?;
        if criteria
            .insert(index, criterion_verdict.to_string())
            .is_some()
        {
            return Err(format!("criterion {index} is reported more than once"));
        }
    }

    Ok(ReviewRecord {
        task_id: task_id.to_string(),
        verdict: verdict.to_string(),
        reconciled_through,
        criteria,
        late_corrections,
    })
}

/// Timestamp of the newest comment that is task authority rather than a review
/// record — the correction an approval must not have been written before.
fn latest_task_authority(comments: &[TaskComment]) -> Option<DateTime<Utc>> {
    comments
        .iter()
        .filter(|comment| review_record_payload(&comment.message).is_none())
        .map(|comment| comment.at)
        .max()
}

/// Why this record may not carry an approval. Empty means it may.
fn approval_violations(
    record: &ReviewRecord,
    criteria_count: usize,
    latest_authority: Option<DateTime<Utc>>,
) -> Vec<String> {
    let mut violations = Vec::new();
    let task_id = &record.task_id;

    let expected: BTreeSet<u64> = (1..=criteria_count as u64).collect();
    let reported: BTreeSet<u64> = record.criteria.keys().copied().collect();
    let missing = expected.difference(&reported).copied().collect::<Vec<_>>();
    if !missing.is_empty() {
        violations.push(format!(
            "task {task_id} has no verdict for acceptance criteria {missing:?} (of {criteria_count})"
        ));
    }
    let unknown = reported.difference(&expected).copied().collect::<Vec<_>>();
    if !unknown.is_empty() {
        violations.push(format!(
            "task {task_id} reports verdicts for criteria {unknown:?} that do not exist (it has {criteria_count})"
        ));
    }
    let unmet = record
        .criteria
        .iter()
        .filter(|(_, verdict)| verdict.as_str() != "met")
        .map(|(index, _)| *index)
        .collect::<Vec<_>>();
    if !unmet.is_empty() {
        violations.push(format!(
            "task {task_id} approves while reporting criteria {unmet:?} as not met"
        ));
    }

    if let Some(latest_authority) = latest_authority
        && record.reconciled_through < latest_authority
    {
        violations.push(format!(
            "task {task_id} was approved against authority reconciled through {}, but a later task comment landed at {latest_authority}",
            record.reconciled_through
        ));
    }

    violations
}

fn non_blank_str<'a>(input: &'a Value, field: &str) -> Option<&'a str> {
    input
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn pipeline_wait_entry_failure(label: &str, entry: &Value) -> Option<String> {
    let Some(status) = entry.get("status").and_then(Value::as_str) else {
        return Some(format!("{label} missing string status"));
    };
    if status == "succeeded" {
        return None;
    }

    let run_id = entry
        .get("run_id")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let error = entry
        .get("error")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    Some(match error {
        Some(error) => format!("{label} run {run_id} status {status}: {error}"),
        None => format!("{label} run {run_id} status {status}"),
    })
}

fn action_failed(action: &str, message: String) -> DispatchError {
    DispatchError::DeterministicActionFailed {
        action: action.to_string(),
        message,
    }
}

pub(super) fn gate_starvation_fail(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<Value, DispatchError> {
    let task_ids_vec: Vec<String> = input
        .get("task_ids")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let conflicts = input
        .get("conflicts")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    let max_wait_seconds = input.get("max_wait_seconds").and_then(Value::as_f64);
    let conflicting_files: Vec<String> = conflicts
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| {
                    entry
                        .get("file")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect()
        })
        .unwrap_or_default();
    // The gate can starve on either axis. Reporting only `conflicting_files`
    // left a dependency-starved bundle with an empty list and no blocker
    // named at all, so carry the last-observed unmet dependency IDs too.
    let waiting_on_deps: Vec<String> = input
        .get("waiting_on_deps")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.as_str().map(str::trim))
                .filter(|entry| !entry.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();

    let payload = serde_json::json!({
        "task_ids": task_ids_vec,
        "conflicting_files": conflicting_files,
        "conflicts": conflicts,
        "waiting_on_deps": waiting_on_deps,
        "max_wait_seconds": max_wait_seconds,
    });

    let execution_id = audit_execution_id("audit-gate-starvation");
    let working_directory = runtime.paths().repo_root.to_string_lossy().into_owned();
    runtime
        .record_audit_event(&AuditEventInsertParams {
            execution_id,
            command: "gate.starvation".to_string(),
            subcommand: None,
            tool_name: None,
            target_type: Some("task_bundle".to_string()),
            target_id: task_ids_vec.first().cloned(),
            role: "admin".to_string(),
            status: AuditEventStatus::Failure,
            exit_code: 1,
            duration_ms: 0,
            working_directory,
            arguments_json: Some(serde_json::to_string(&payload).map_err(|error| {
                DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: format!("serialize gate.starvation payload: {error}"),
                }
            })?),
            stdout_truncated: None,
            stderr_truncated: None,
            error_message: Some("gate.starvation".to_string()),
            host: std::env::var("HOSTNAME").ok(),
            pid: std::process::id(),
            session_id: None,
            workspace_id: None,
            caller_machine_id: None,
            caller_host_id: None,
            process_machine_id: None,
            process_host_id: None,
            transport: None,
            effective_capabilities: Default::default(),
            origin_session_id: None,
            mcp_call_id: None,
            lease_id: None,
            task_id: task_ids_vec.first().cloned(),
            job_run_id: None,
            activity_id: None,
            step_index: None,
        })
        .map_err(|err| DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("record gate.starvation audit: {err}"),
        })?;

    Err(DispatchError::DeterministicActionFailed {
        action: action.to_string(),
        message: format!(
            "gate.starvation: admission window never opened for bundle {:?} \
             (conflicting_files={:?}, waiting_on_deps={:?}, max_wait_seconds={:?})",
            task_ids_vec, conflicting_files, waiting_on_deps, max_wait_seconds
        ),
    })
}
