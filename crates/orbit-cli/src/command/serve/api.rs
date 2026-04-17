//! Read-only JSON HTTP handlers for the dashboard.
//!
//! Each handler delegates to the same `*_to_json` helpers used by the CLI's
//! `--json` paths so the wire format stays in lockstep with the CLI.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use chrono::Utc;
use orbit_core::OrbitRuntime;
use orbit_core::command::job_run::JobRunListParams;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::command::audit::audit_event_to_json;
use crate::command::job::{job_run_to_json, job_to_json_with_last_run};
use crate::command::task::output::task_to_json;

const DIAG_DEFAULT_LIMIT: usize = 200;

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

pub(super) fn router() -> Router<Arc<OrbitRuntime>> {
    Router::new()
        .route("/tasks", get(list_tasks))
        .route("/tasks/:id", get(get_task))
        .route("/jobs", get(list_jobs))
        .route("/job-runs", get(list_job_runs))
        .route("/audit", get(list_audit))
        .route("/scoreboard", get(scoreboard))
        .route("/diagnostics/metrics", get(list_diagnostics_metrics))
        .route("/diagnostics/friction", get(list_diagnostics_friction))
}

async fn list_tasks(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    match runtime.list_tasks() {
        Ok(tasks) => {
            let values: Vec<Value> = tasks.iter().map(task_to_json).collect();
            Json(Value::Array(values)).into_response()
        }
        Err(e) => server_error(e),
    }
}

async fn get_task(State(runtime): State<Arc<OrbitRuntime>>, Path(id): Path<String>) -> Response {
    match runtime.get_task(&id) {
        Ok(task) => Json(task_to_json(&task)).into_response(),
        Err(e) => match e {
            orbit_core::OrbitError::TaskNotFound(_) => not_found(format!("task not found: {id}")),
            other => server_error(other),
        },
    }
}

async fn list_jobs(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    match runtime.list_jobs_with_last_run(true) {
        Ok(rows) => {
            let values: Vec<Value> = rows
                .iter()
                .map(|(job, last_run)| job_to_json_with_last_run(job, last_run.as_ref()))
                .collect();
            Json(Value::Array(values)).into_response()
        }
        Err(e) => server_error(e),
    }
}

async fn list_job_runs(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    let params = JobRunListParams {
        job_id: None,
        state: None,
        since: None,
        limit: Some(50),
    };
    match runtime.list_job_runs(params) {
        Ok(runs) => {
            let values: Vec<Value> = runs.iter().map(job_run_to_json).collect();
            Json(Value::Array(values)).into_response()
        }
        Err(e) => server_error(e),
    }
}

async fn list_audit(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    match runtime.list_audit_events(None, None, None, None, 100) {
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
    let limit = q.limit.unwrap_or(DIAG_DEFAULT_LIMIT);
    match runtime.read_metrics_entries(&month) {
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
    let limit = q.limit.unwrap_or(DIAG_DEFAULT_LIMIT);
    match runtime.read_friction_entries(&month) {
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
