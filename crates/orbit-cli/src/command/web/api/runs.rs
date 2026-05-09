//! Run lifecycle: detail, cancel, replay, events, logs.

use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use orbit_common::utility::redaction::redact_all;
use orbit_core::runtime::run_audit::{RunAuditStep, RunCliInvocationRecord};
use orbit_core::{JobRun, OrbitRuntime};
use serde_json::{Value, json};

use super::{
    HISTORY_DEFAULT_LIMIT, LimitQuery, RunEventsQuery, bad_request, bounded_limit,
    map_runtime_error, server_error, v2_loop_dir, validate_id,
};
use crate::command::run::job_run_to_json;

const RUN_EVENTS_DEFAULT_LIMIT: usize = 100;
/// Maximum bytes included in stdout/stderr previews returned by run-log APIs.
const RUN_LOG_PREVIEW_MAX_BYTES: usize = 8192;
/// Maximum lines included in stdout/stderr previews returned by run-log APIs.
const RUN_LOG_PREVIEW_MAX_LINES: usize = 120;

pub(super) async fn get_run(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
) -> Response {
    let id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    match runtime.show_job_run(id) {
        Ok(run) => Json(job_run_detail_to_json(&runtime, &run)).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn cancel_run_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
) -> Response {
    let id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    match runtime.cancel_job_run_with_context(id, "dashboard", "web") {
        Ok(result) => Json(json!({
            "run_id": result.run_id,
            "previous_state": result.previous_state,
            "final_state": result.final_state,
            "actor": result.actor,
            "source": result.source,
            "signal_attempted": result.signal_attempted,
            "signal_outcome": result.signal_outcome,
        }))
        .into_response(),
        Err(orbit_core::OrbitError::JobValidation(msg))
        | Err(orbit_core::OrbitError::JobRunStateTransition(msg)) => {
            (StatusCode::CONFLICT, Json(json!({ "error": msg }))).into_response()
        }
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn replay_run_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
) -> Response {
    let id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    match runtime.replay_job_run(id) {
        Ok(result) => Json(json!({ "run_id": result.run_id })).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) fn job_run_detail_to_json(runtime: &OrbitRuntime, run: &JobRun) -> Value {
    let mut full = job_run_to_json(run);
    // Reshape into `{run, steps}` per the dashboard contract: peel the
    // `steps` array off the flat `job_run_to_json` output.
    let stored_steps = full
        .as_object_mut()
        .and_then(|m| m.remove("steps"))
        .unwrap_or(Value::Array(Vec::new()));

    let audit_steps = runtime
        .collect_run_audit_steps(&run.run_id)
        .unwrap_or_default();
    let steps = if audit_steps.is_empty() {
        stored_steps
    } else {
        Value::Array(audit_steps.iter().map(audit_step_to_json).collect())
    };

    json!({ "run": full, "steps": steps })
}

fn audit_step_to_json(step: &RunAuditStep) -> Value {
    let duration_ms = match (step.started_at, step.finished_at) {
        (Some(started), Some(finished)) => Some(
            finished
                .signed_duration_since(started)
                .num_milliseconds()
                .max(0) as u64,
        ),
        _ => None,
    };

    json!({
        "step_index": step.step_index,
        "target_type": "activity",
        "target_id": step.step_id,
        "state": step.state.as_deref().unwrap_or("running"),
        "started_at": step.started_at.map(|v| v.to_rfc3339()),
        "finished_at": step.finished_at.map(|v| v.to_rfc3339()),
        "duration_ms": duration_ms,
        "exit_code": null,
        "agent_response_json": null,
        "error_code": null,
        "error_message": step.error_message,
        "outcome": step.outcome,
    })
}

pub(super) async fn list_run_events(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
    Query(q): Query<RunEventsQuery>,
) -> Response {
    let run_id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    let limit = bounded_limit(q.limit, RUN_EVENTS_DEFAULT_LIMIT);
    let offset = q.offset.unwrap_or(0);
    let kind = q
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let path = v2_loop_path(&runtime, run_id);
    let path = match canonical_v2_loop_path(&runtime, &path) {
        Ok(Some(path)) => path,
        Ok(None) => return Json(Value::Array(Vec::new())).into_response(),
        Err(e) => return map_runtime_error(e),
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Json(Value::Array(Vec::new())).into_response();
        }
        Err(e) => {
            return server_error(orbit_core::OrbitError::Io(format!(
                "read {}: {e}",
                path.display()
            )));
        }
    };

    let mut events: Vec<Value> = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(ref needle) = kind {
            let body_kind = value.get("body_kind").and_then(Value::as_str).unwrap_or("");
            if body_kind != needle {
                continue;
            }
        }
        events.push(value);
    }

    if offset >= events.len() {
        return Json(Value::Array(Vec::new())).into_response();
    }
    let end = offset.saturating_add(limit).min(events.len());
    let page: Vec<Value> = events.drain(offset..end).collect();
    Json(Value::Array(page)).into_response()
}

pub(super) async fn list_run_logs(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
    Query(q): Query<LimitQuery>,
) -> Response {
    let run_id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    let limit = bounded_limit(q.limit, HISTORY_DEFAULT_LIMIT);
    match runtime.collect_run_cli_invocations(run_id) {
        Ok(records) => Json(Value::Array(
            records
                .into_iter()
                .take(limit)
                .map(run_cli_invocation_to_json)
                .collect(),
        ))
        .into_response(),
        Err(e) => map_runtime_error(e),
    }
}

fn run_cli_invocation_to_json(record: RunCliInvocationRecord) -> Value {
    let stdout_preview = bounded_preview(&record.stdout);
    let stderr_preview = bounded_preview(&record.stderr);
    json!({
        "run_id": record.run_id,
        "event_id": record.event_id,
        "ts": record.ts.map(|ts| ts.to_rfc3339()),
        "step_id": record.step_id,
        "step_index": record.step_index,
        "provider": record.provider,
        "stdout_blob_ref": record.stdout_blob_ref,
        "stderr_blob_ref": record.stderr_blob_ref,
        "stdout_preview": stdout_preview.text,
        "stderr_preview": stderr_preview.text,
        "stdout_truncated": stdout_preview.truncated,
        "stderr_truncated": stderr_preview.truncated,
        "exit_code": record.exit_code,
        "timed_out": record.timed_out,
        "duration_ms": record.duration_ms,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Preview {
    text: String,
    truncated: bool,
}

fn bounded_preview(raw: &str) -> Preview {
    let mut out = String::new();
    let mut truncated = false;
    for (index, line) in raw.lines().enumerate() {
        if index >= RUN_LOG_PREVIEW_MAX_LINES {
            truncated = true;
            break;
        }
        let needed = line.len() + usize::from(!out.is_empty());
        if out.len().saturating_add(needed) > RUN_LOG_PREVIEW_MAX_BYTES {
            if out.is_empty() {
                for ch in line.chars() {
                    if out.len().saturating_add(ch.len_utf8()) > RUN_LOG_PREVIEW_MAX_BYTES {
                        break;
                    }
                    out.push(ch);
                }
            }
            truncated = true;
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    if raw.ends_with('\n') && !out.is_empty() && out.len() < RUN_LOG_PREVIEW_MAX_BYTES {
        out.push('\n');
    }
    Preview {
        text: redact_all(&out),
        truncated,
    }
}

fn v2_loop_path(runtime: &OrbitRuntime, run_id: &str) -> PathBuf {
    v2_loop_dir(runtime).join(format!("{run_id}.jsonl"))
}

fn canonical_v2_loop_path(
    runtime: &OrbitRuntime,
    path: &FsPath,
) -> Result<Option<PathBuf>, orbit_core::OrbitError> {
    let canonical_path = match path.canonicalize() {
        Ok(path) => path,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(orbit_core::OrbitError::Io(format!(
                "resolve {}: {e}",
                path.display()
            )));
        }
    };
    let audit_dir = v2_loop_dir(runtime);
    let canonical_dir = audit_dir
        .canonicalize()
        .map_err(|e| orbit_core::OrbitError::Io(format!("resolve {}: {e}", audit_dir.display())))?;
    if !canonical_path.starts_with(&canonical_dir) {
        return Err(orbit_core::OrbitError::InvalidInput(
            "run id resolved outside audit log directory".to_string(),
        ));
    }
    Ok(Some(canonical_path))
}

#[cfg(test)]
#[path = "runs_tests.rs"]
mod tests;
