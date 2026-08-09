//! Run lifecycle: detail, cancel, replay, events, logs.

use crate::state::Ws;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use orbit_common::utility::redaction::redact_all;
use orbit_core::runtime::run_audit::{RunAuditStep, RunCliInvocationRecord, RunProviderProcess};
use orbit_core::{JobRun, OrbitRuntime, V2AuditEventFilter};
use serde_json::{Value, json};

use super::{
    HISTORY_DEFAULT_LIMIT, LimitQuery, RunEventsQuery, bad_request, bounded_limit,
    map_runtime_error, validate_id,
};
use crate::projections::job_run_to_json;

const RUN_EVENTS_DEFAULT_LIMIT: usize = 100;
/// Hard cap on rows scanned from a single run's persisted v2 audit events.
pub(super) const RUN_EVENTS_MAX_SCAN_LINES: usize = 50_000;
/// Maximum bytes included in stdout/stderr previews returned by run-log APIs.
const RUN_LOG_PREVIEW_MAX_BYTES: usize = 8192;
/// Maximum lines included in stdout/stderr previews returned by run-log APIs.
const RUN_LOG_PREVIEW_MAX_LINES: usize = 120;

#[derive(serde::Deserialize, Default)]
pub(super) struct ShipBody {
    /// Explicit task selection; empty selects auto (backlog-discovery) mode.
    #[serde(default)]
    task_ids: Vec<String>,
    /// `"pr"` or `"local"`. Omitted means the workspace's own configured ship
    /// mode (ORB-10444) — what the dashboard's one-click Ship sends.
    #[serde(default)]
    mode: Option<String>,
    /// Base branch override; defaults to the workspace's `[workflow] base_branch`.
    #[serde(default)]
    base: Option<String>,
}

/// Submit a `ship` workflow run (`POST /workflows/ship?workspace=<id>`).
///
/// One-shot: responds as soon as the run is persisted, with the same
/// `queued`/`submitted` states the CLI reports. Callers poll
/// `GET /runs/:id` for progress.
///
/// An omitted `mode` resolves to the selected workspace's own ship mode
/// (ORB-10444), so the dashboard's one-click Ship needs no PR/local toggle;
/// a workspace with no registry binding (single/embedded mode) keeps the
/// historical `pr` default.
///
/// [ORB-10544] Duplicate dispatch of an explicitly-selected task is refused by
/// the shared submission path, not here: `submit_ship_run` returns
/// `OrbitError::ShipRunInFlight` when one of the named tasks is already carried
/// by a non-terminal run, which `map_runtime_error` projects to this endpoint's
/// stable 409. Auto (backlog-discovery) mode has no task ids to guard and is
/// unaffected.
pub(super) async fn ship_workflow_action(
    Ws(runtime): Ws,
    body: Option<Json<ShipBody>>,
) -> Response {
    let Json(body) = body.unwrap_or_default();
    let mode = match body.mode.as_deref() {
        Some(raw) => match orbit_core::ShipMode::parse(raw) {
            Ok(mode) => mode,
            Err(e) => return bad_request(e.to_string()),
        },
        None => workspace_default_ship_mode(&runtime),
    };
    match runtime.submit_ship_run(
        mode,
        body.base.as_deref(),
        &body.task_ids,
        Some("dashboard"),
    ) {
        Ok(invoke) => Json(json!({
            "workflow": "ship",
            "job_id": invoke.job_name,
            "run_id": invoke.run_id,
            "state": if invoke.queued { "queued" } else { "submitted" },
            "submitted_at": invoke.submitted_at,
        }))
        .into_response(),
        Err(orbit_core::OrbitError::InvalidInput(msg)) => bad_request(msg),
        Err(e) => map_runtime_error(e),
    }
}

/// The selected workspace's configured ship mode, or `pr` when this runtime was
/// built without a registry binding (single/embedded serving mode), which is the
/// default the endpoint has always applied to a body with no `mode`.
fn workspace_default_ship_mode(runtime: &OrbitRuntime) -> orbit_core::ShipMode {
    runtime
        .workspace_runtime_binding()
        .map_or(orbit_core::ShipMode::Pr, |binding| binding.ship_mode)
}

pub(super) async fn get_run(Ws(runtime): Ws, Path(id): Path<String>) -> Response {
    let id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    match runtime.show_job_run(id) {
        Ok(run) => Json(job_run_detail_to_json(&runtime, &run)).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn cancel_run_action(Ws(runtime): Ws, Path(id): Path<String>) -> Response {
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

pub(super) async fn replay_run_action(Ws(runtime): Ws, Path(id): Path<String>) -> Response {
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

    // [ORB-10496] Provider subprocesses for this run's agent steps, with a
    // liveness verdict for any that have not reported an exit. Without this a
    // healthy long-running ship-pipeline implementation agent is
    // indistinguishable from a dead child without shell access to the host.
    let provider_processes = runtime
        .collect_run_provider_processes(&run.run_id)
        .unwrap_or_default();

    json!({
        "run": full,
        "steps": steps,
        "provider_processes": provider_processes
            .iter()
            .map(run_provider_process_to_json)
            .collect::<Vec<_>>(),
    })
}

fn run_provider_process_to_json(record: &RunProviderProcess) -> Value {
    json!({
        "run_id": record.run_id,
        "event_id": record.event_id,
        "ts": record.ts.map(|ts| ts.to_rfc3339()),
        "step_id": record.step_id,
        "step_index": record.step_index,
        "provider": record.provider,
        "pid": record.pid,
        "pid_start_time": record.pid_start_time,
        "finished": record.finished,
        "liveness": record.liveness.as_str(),
        "exit_code": record.exit_code,
        "timed_out": record.timed_out,
        "duration_ms": record.duration_ms,
    })
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
    Ws(runtime): Ws,
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

    let rows = match runtime.list_v2_audit_events(V2AuditEventFilter {
        workspace_id: String::new(),
        run_id: Some(run_id.to_string()),
        source: Some("v2_envelope".to_string()),
        limit: Some(RUN_EVENTS_MAX_SCAN_LINES + 1),
        ..Default::default()
    }) {
        Ok(rows) => rows,
        Err(e) => return map_runtime_error(e),
    };
    let mut page: Vec<Value> = Vec::with_capacity(limit.min(64));
    let mut matched: usize = 0;
    let mut lines_scanned: usize = 0;
    let mut budget_exceeded = false;

    for row in rows.into_iter().rev() {
        lines_scanned = lines_scanned.saturating_add(1);
        if lines_scanned > RUN_EVENTS_MAX_SCAN_LINES {
            budget_exceeded = true;
            break;
        }
        let value: Value = match serde_json::from_str(&row.payload_json) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(ref needle) = kind {
            let body_kind = value.get("body_kind").and_then(Value::as_str).unwrap_or("");
            if body_kind != needle {
                continue;
            }
        }
        if matched < offset {
            matched = matched.saturating_add(1);
            continue;
        }
        page.push(value);
        matched = matched.saturating_add(1);
        if page.len() >= limit {
            break;
        }
    }

    if budget_exceeded && page.len() < limit {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({
                "error": "run-events audit rows exceed bounded scan budget; narrow the kind filter or reduce offset"
            })),
        )
            .into_response();
    }

    Json(Value::Array(page)).into_response()
}

pub(super) async fn list_run_logs(
    Ws(runtime): Ws,
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
