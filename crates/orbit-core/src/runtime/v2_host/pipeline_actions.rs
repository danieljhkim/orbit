use orbit_common::types::{
    AuditEventStatus, Role, TaskStatus, audit_execution_id, optional_string_list_alias,
};
use orbit_engine::{DispatchError, ensure_task_can_enter_workflow};
use orbit_store::AuditEventInsertParams;
use orbit_tools::ToolContext;
use serde_json::Value;

use crate::OrbitRuntime;
use crate::command::job::JobRunListParams;
use crate::runtime::orbit_tool_host::parse_task_ids;

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

fn deduped_child_run_id(
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

pub(super) fn independent_review_guard(
    action: &str,
    input: &Value,
) -> Result<Value, DispatchError> {
    let verdict = input
        .get("verdict")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| action_failed(action, "missing non-blank `verdict`".to_string()))?;
    if !matches!(verdict, "approve" | "request_changes") {
        return Err(action_failed(
            action,
            format!(
                "unknown independent review verdict '{verdict}' (expected 'approve' or 'request_changes')"
            ),
        ));
    }

    let reviewed_head_sha = input
        .get("reviewed_head_sha")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            action_failed(action, "missing non-blank `reviewed_head_sha`".to_string())
        })?;
    let candidate_head_sha = input
        .get("candidate_head_sha")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
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

    Ok(serde_json::json!({
        "verdict": verdict,
        "reviewed_head_sha": reviewed_head_sha,
        "exact_head": true,
    }))
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

    let payload = serde_json::json!({
        "task_ids": task_ids_vec,
        "conflicting_files": conflicting_files,
        "conflicts": conflicts,
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
             (conflicting_files={:?}, max_wait_seconds={:?})",
            task_ids_vec, conflicting_files, max_wait_seconds
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn action_failure_message(err: DispatchError) -> String {
        match err {
            DispatchError::DeterministicActionFailed { action, message } => {
                assert_eq!(action, "pipeline_success_guard");
                message
            }
            other => panic!("expected deterministic action failure, got {other}"),
        }
    }

    #[test]
    fn pipeline_success_guard_accepts_succeeded_result() {
        let output = pipeline_success_guard(
            "pipeline_success_guard",
            &json!({
                "result": {
                    "run_id": "jrun-ok",
                    "status": "succeeded"
                }
            }),
        )
        .expect("succeeded result should pass");

        assert_eq!(output["succeeded"], json!(true));
        assert_eq!(output["checked_count"], json!(1));
    }

    #[test]
    fn pipeline_success_guard_rejects_failed_result() {
        let err = pipeline_success_guard(
            "pipeline_success_guard",
            &json!({
                "context": "task gate child",
                "result": {
                    "run_id": "jrun-failed",
                    "status": "failed",
                    "error": "implementation failed"
                }
            }),
        )
        .expect_err("failed child run should fail the guard");

        let message = action_failure_message(err);
        assert!(message.contains("task gate child did not succeed"));
        assert!(message.contains("jrun-failed"));
        assert!(message.contains("status failed"));
        assert!(message.contains("implementation failed"));
    }

    #[test]
    fn pipeline_success_guard_rejects_mixed_results() {
        let err = pipeline_success_guard(
            "pipeline_success_guard",
            &json!({
                "results": [
                    {
                        "run_id": "jrun-ok",
                        "status": "succeeded"
                    },
                    {
                        "run_id": "jrun-cancelled",
                        "status": "cancelled"
                    },
                    null
                ]
            }),
        )
        .expect_err("any non-succeeded result should fail the guard");

        let message = action_failure_message(err);
        assert!(message.contains("results[1] run jrun-cancelled status cancelled"));
        assert!(message.contains("results[2] missing string status"));
    }

    #[test]
    fn invoke_and_wait_dedupe_reuses_one_run_and_rejects_multiple_matches() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let first = runtime
            .stores()
            .jobs()
            .insert_run(
                "task_review_pipeline",
                1,
                chrono::Utc::now(),
                Some(json!({ "parent_run_id": "jrun-parent" })),
                None,
            )
            .expect("insert first review run");
        let action_input = json!({ "dedupe_run_input_field": "parent_run_id" });
        let run_input = json!({ "parent_run_id": "jrun-parent" });

        let reused = deduped_child_run_id(
            &runtime,
            "invoke_and_wait",
            &action_input,
            "task_review_pipeline",
            &run_input,
        )
        .expect("dedupe lookup");
        assert_eq!(reused.as_deref(), Some(first.run_id.as_str()));

        runtime
            .stores()
            .jobs()
            .insert_run(
                "task_review_pipeline",
                1,
                chrono::Utc::now(),
                Some(json!({ "parent_run_id": "jrun-parent" })),
                None,
            )
            .expect("insert duplicate review run");
        let error = deduped_child_run_id(
            &runtime,
            "invoke_and_wait",
            &action_input,
            "task_review_pipeline",
            &run_input,
        )
        .expect_err("multiple matching review runs fail closed");
        assert!(matches!(
            error,
            DispatchError::DeterministicActionFailed { .. }
        ));
    }

    #[test]
    fn independent_review_guard_accepts_both_exact_head_verdicts() {
        for verdict in ["approve", "request_changes"] {
            let output = independent_review_guard(
                "independent_review_guard",
                &json!({
                    "verdict": verdict,
                    "reviewed_head_sha": "abc123",
                    "candidate_head_sha": "abc123",
                }),
            )
            .expect("exact-head verdict passes");

            assert_eq!(output["verdict"], verdict);
            assert_eq!(output["reviewed_head_sha"], "abc123");
            assert_eq!(output["exact_head"], true);
        }
    }

    #[test]
    fn independent_review_guard_fails_closed_on_missing_or_mismatched_verdict() {
        for input in [
            json!({
                "reviewed_head_sha": "abc123",
                "candidate_head_sha": "abc123",
            }),
            json!({
                "verdict": "looks_good",
                "reviewed_head_sha": "abc123",
                "candidate_head_sha": "abc123",
            }),
            json!({
                "verdict": "approve",
                "reviewed_head_sha": "def456",
                "candidate_head_sha": "abc123",
            }),
        ] {
            let error = independent_review_guard("independent_review_guard", &input)
                .expect_err("invalid verdict must fail");
            assert!(matches!(
                error,
                DispatchError::DeterministicActionFailed { .. }
            ));
        }
    }
}
