//! Read-only JSON HTTP handlers for the dashboard.
//!
//! Each handler delegates to the same `*_to_json` helpers used by the CLI's
//! `--json` paths so the wire format stays in lockstep with the CLI.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use chrono::Utc;
use orbit_core::command::job_run::JobRunListParams;
use orbit_core::{OrbitRuntime, Task, TaskStatus};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::command::audit::audit_event_to_json;
use crate::command::job::{job_catalog_to_json_with_last_run, job_run_to_json};
use crate::command::task::output::task_to_json;

const DASHBOARD_TASK_STATUSES: &[TaskStatus] = &[
    TaskStatus::InProgress,
    TaskStatus::Review,
    TaskStatus::Blocked,
    TaskStatus::Proposed,
    TaskStatus::Backlog,
    TaskStatus::Someday,
    TaskStatus::Rejected,
];
const HISTORY_DEFAULT_LIMIT: usize = 50;
const JOB_RUN_DEFAULT_LIMIT: usize = 25;
const HISTORY_MAX_LIMIT: usize = 200;

#[derive(Deserialize, Default)]
pub(super) struct LimitQuery {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
pub(super) struct DiagnosticsQuery {
    #[serde(default)]
    month: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

fn current_year_month_utc() -> String {
    Utc::now().format("%Y-%m").to_string()
}

/// Validates a `YYYY-MM` string with month range 01..=12.
fn validate_year_month(raw: &str) -> Result<(), orbit_core::OrbitError> {
    let bytes = raw.as_bytes();
    let format_ok = bytes.len() == 7
        && bytes[4] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..].iter().all(u8::is_ascii_digit);
    if !format_ok {
        return Err(orbit_core::OrbitError::InvalidInput(format!(
            "month must be in YYYY-MM format, got '{raw}'"
        )));
    }
    let month: u32 = raw[5..].parse().unwrap_or(0);
    if !(1..=12).contains(&month) {
        return Err(orbit_core::OrbitError::InvalidInput(format!(
            "month component must be 01-12, got '{raw}'"
        )));
    }
    Ok(())
}

async fn require_localhost_origin(request: Request<Body>, next: Next) -> Response {
    if !request.method().is_safe()
        && let Some(origin) = request.headers().get(header::ORIGIN)
    {
        let origin = origin.to_str().unwrap_or("");
        if !origin.starts_with("http://localhost") && !origin.starts_with("http://127.0.0.1") {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "cross-origin requests not allowed"})),
            )
                .into_response();
        }
    }
    next.run(request).await
}

pub(super) fn router() -> Router<Arc<OrbitRuntime>> {
    Router::new()
        .route("/tasks", get(list_tasks))
        .route("/tasks/:id", get(get_task))
        .route("/tasks/:id/approve", post(approve_task_action))
        .route("/tasks/:id/reject", post(reject_task_action))
        .route("/tasks/:id/archive", post(archive_task_action))
        .route("/jobs", get(list_jobs))
        .route("/job-runs", get(list_job_runs))
        .route("/audit", get(list_audit))
        .route("/scoreboard", get(scoreboard))
        .route("/diagnostics/metrics", get(list_diagnostics_metrics))
        .route("/diagnostics/friction", get(list_diagnostics_friction))
        .layer(middleware::from_fn(require_localhost_origin))
}

#[derive(Deserialize, Default)]
pub(super) struct ApproveBody {
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    comment: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RejectBody {
    note: String,
    #[serde(default)]
    comment: Option<String>,
}

async fn list_tasks(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    match list_dashboard_tasks(&runtime) {
        Ok(tasks) => {
            let status_by_id = orbit_core::build_task_status_index(&tasks);
            let values: Vec<Value> = tasks
                .iter()
                .map(|task| task_to_json(task, &status_by_id))
                .collect();
            Json(Value::Array(values)).into_response()
        }
        Err(e) => server_error(e),
    }
}

fn list_dashboard_tasks(runtime: &OrbitRuntime) -> Result<Vec<Task>, orbit_core::OrbitError> {
    let mut tasks = Vec::new();
    for status in DASHBOARD_TASK_STATUSES {
        tasks.extend(runtime.list_tasks_filtered(Some(*status), None, None, None)?);
    }
    Ok(tasks)
}

fn dashboard_status_index(
    runtime: &OrbitRuntime,
) -> Result<std::collections::BTreeMap<String, TaskStatus>, orbit_core::OrbitError> {
    Ok(orbit_core::build_task_status_index(&list_dashboard_tasks(
        runtime,
    )?))
}

fn bounded_limit(requested: Option<usize>, default: usize) -> usize {
    requested.unwrap_or(default).min(HISTORY_MAX_LIMIT)
}

async fn get_task(State(runtime): State<Arc<OrbitRuntime>>, Path(id): Path<String>) -> Response {
    match runtime.get_task(&id) {
        Ok(task) => match dashboard_status_index(&runtime) {
            Ok(status_by_id) => Json(task_to_json(&task, &status_by_id)).into_response(),
            Err(e) => server_error(e),
        },
        Err(e) => map_runtime_error(e),
    }
}

async fn approve_task_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
    body: Option<Json<ApproveBody>>,
) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    match runtime.approve_task(&id, body.note, body.comment) {
        Ok(task) => match dashboard_status_index(&runtime) {
            Ok(status_by_id) => Json(task_to_json(&task, &status_by_id)).into_response(),
            Err(e) => server_error(e),
        },
        Err(e) => map_runtime_error(e),
    }
}

async fn reject_task_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
    Json(body): Json<RejectBody>,
) -> Response {
    match runtime.reject_task(&id, body.note, body.comment) {
        Ok(task) => match dashboard_status_index(&runtime) {
            Ok(status_by_id) => Json(task_to_json(&task, &status_by_id)).into_response(),
            Err(e) => server_error(e),
        },
        Err(e) => map_runtime_error(e),
    }
}

async fn archive_task_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
) -> Response {
    match runtime.archive_task(&id) {
        Ok(()) => Json(json!({ "ok": true, "id": id })).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

async fn list_jobs(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    use orbit_core::command::job::JobCatalogFilter;
    match runtime.list_job_catalog_with_last_run(true, JobCatalogFilter::All) {
        Ok(rows) => {
            let values: Vec<Value> = rows
                .iter()
                .map(|(entry, last_run)| {
                    job_catalog_to_json_with_last_run(entry, last_run.as_ref())
                })
                .collect();
            Json(Value::Array(values)).into_response()
        }
        Err(e) => server_error(e),
    }
}

async fn list_job_runs(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(q): Query<LimitQuery>,
) -> Response {
    let limit = bounded_limit(q.limit, JOB_RUN_DEFAULT_LIMIT);
    let params = JobRunListParams {
        job_id: None,
        state: None,
        since: None,
        limit: Some(limit),
    };
    match runtime.list_job_runs(params) {
        Ok(runs) => {
            let values: Vec<Value> = runs.iter().map(job_run_to_json).collect();
            Json(Value::Array(values)).into_response()
        }
        Err(e) => server_error(e),
    }
}

async fn list_audit(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(q): Query<LimitQuery>,
) -> Response {
    let limit = bounded_limit(q.limit, HISTORY_DEFAULT_LIMIT);
    match runtime.list_audit_events(None, None, None, None, limit) {
        Ok(events) => {
            let values: Vec<Value> = events.iter().map(audit_event_to_json).collect();
            Json(Value::Array(values)).into_response()
        }
        Err(e) => server_error(e),
    }
}

async fn scoreboard(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    match runtime.generate_scoreboard_summary() {
        Ok(summary) => match serde_json::to_value(&summary) {
            Ok(value) => Json(value).into_response(),
            Err(e) => server_error(orbit_core::OrbitError::Store(e.to_string())),
        },
        Err(e) => server_error(e),
    }
}

async fn list_diagnostics_metrics(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(q): Query<DiagnosticsQuery>,
) -> Response {
    let month = q.month.unwrap_or_else(current_year_month_utc);
    if let Err(e) = validate_year_month(&month) {
        return map_runtime_error(e);
    }
    let limit = bounded_limit(q.limit, HISTORY_DEFAULT_LIMIT);
    match runtime.read_metrics_entries_limited(&month, limit) {
        Ok(mut entries) => {
            entries.sort_by(|a, b| b.ts.cmp(&a.ts));
            entries.truncate(limit);
            match serde_json::to_value(&entries) {
                Ok(value) => Json(value).into_response(),
                Err(e) => server_error(orbit_core::OrbitError::Store(e.to_string())),
            }
        }
        Err(e) => map_runtime_error(e),
    }
}

async fn list_diagnostics_friction(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(q): Query<DiagnosticsQuery>,
) -> Response {
    let month = q.month.unwrap_or_else(current_year_month_utc);
    if let Err(e) = validate_year_month(&month) {
        return map_runtime_error(e);
    }
    let limit = bounded_limit(q.limit, HISTORY_DEFAULT_LIMIT);
    match runtime.read_friction_entries_limited(&month, limit) {
        Ok(mut entries) => {
            entries.sort_by(|a, b| b.ts.cmp(&a.ts));
            entries.truncate(limit);
            match serde_json::to_value(&entries) {
                Ok(value) => Json(value).into_response(),
                Err(e) => server_error(orbit_core::OrbitError::Store(e.to_string())),
            }
        }
        Err(e) => map_runtime_error(e),
    }
}

fn map_runtime_error(e: orbit_core::OrbitError) -> Response {
    match e {
        orbit_core::OrbitError::InvalidInput(msg) => bad_request(msg),
        orbit_core::OrbitError::TaskNotFound(msg) => not_found(format!("task not found: {msg}")),
        other => server_error(other),
    }
}

fn bad_request(message: String) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
}

fn not_found(message: String) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": message }))).into_response()
}

fn server_error(e: orbit_core::OrbitError) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": e.to_string() })),
    )
        .into_response()
}
