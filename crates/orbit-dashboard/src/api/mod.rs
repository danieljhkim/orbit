//! Read-only JSON HTTP handlers for the dashboard.
//!
//! Each handler delegates to the same `*_to_json` helpers used by the CLI's
//! `--json` paths so the wire format stays in lockstep with the CLI.

// Test-only allowlist (mirrors the original placement under orbit-cli): the many
// `.expect` / `.unwrap` calls in the in-file integration tests and the included
// `*_tests` modules are the documented exception for test harness code.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Duration, TimeZone, Timelike, Utc};
use orbit_core::OrbitRuntime;
use serde::Deserialize;
use serde_json::json;
use url::Url;

mod adrs;
mod audit;
mod crews;
mod denials;
mod diagnostics;
mod frictions;
mod jobs;
mod learnings;
mod log;
mod metrics;
mod runs;
mod scoreboard;
mod tasks;

pub(super) const HISTORY_DEFAULT_LIMIT: usize = 50;
pub(super) const HISTORY_MAX_LIMIT: usize = 200;
/// Default time window for header tile counts when `?since=` is omitted.
pub(super) const DEFAULT_SUMMARY_WINDOW: &str = "24h";
/// Cap on how many `state/audit/v2_loop/*.jsonl` run files we read in one
/// request when aggregating denials. Each file is small (KB-scale) but reads
/// are sync, so we bound iteration to keep the endpoint within budget on
/// long-lived workspaces.
pub(super) const V2_LOOP_FILE_SCAN_CAP: usize = 1500;

#[derive(Deserialize, Default)]
pub(super) struct LimitQuery {
    #[serde(default)]
    pub(super) limit: Option<usize>,
}

#[derive(Deserialize)]
pub(super) struct DiagnosticsQuery {
    #[serde(default)]
    pub(super) month: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<usize>,
}

#[derive(Deserialize, Default)]
pub(super) struct AuditQuery {
    #[serde(default)]
    pub(super) since: Option<String>,
    #[serde(default)]
    pub(super) tool: Option<String>,
    #[serde(default)]
    pub(super) status: Option<String>,
    #[serde(default)]
    pub(super) role: Option<String>,
    /// Filters audit events by orbit invocation id. The SQLite `audit_events`
    /// schema has no `run_id` column; `run_id` here is a backward-compat alias
    /// of `execution_id` (T20260427-26). When both are supplied, `execution_id`
    /// takes precedence.
    #[serde(default)]
    pub(super) execution_id: Option<String>,
    #[serde(default)]
    pub(super) run_id: Option<String>,
    #[serde(default)]
    pub(super) q: Option<String>,
    /// fsProfile filter. The SQLite `audit_events` schema has no first-class
    /// `profile` column; matching is best-effort against `arguments_json`. The
    /// canonical denials view (`/api/diagnostics/denials`) reads the v2 envelope
    /// JSONL where `profile` is a typed field.
    #[serde(default)]
    pub(super) profile: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<usize>,
    #[serde(default)]
    pub(super) offset: Option<usize>,
}

#[derive(Deserialize, Default)]
pub(super) struct AuditSummaryQuery {
    #[serde(default)]
    pub(super) since: Option<String>,
    #[serde(default)]
    pub(super) denial_threshold: Option<i64>,
}

#[derive(Deserialize, Default)]
pub(super) struct DenialsQuery {
    #[serde(default)]
    pub(super) since: Option<String>,
    /// `fs`, `tool`, or omitted (combined).
    #[serde(default)]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) profile: Option<String>,
    #[serde(default)]
    pub(super) agent: Option<String>,
}

#[derive(Deserialize, Default)]
pub(super) struct RunEventsQuery {
    #[serde(default)]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<usize>,
    #[serde(default)]
    pub(super) offset: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub(super) struct LogQuery {
    #[serde(default)]
    pub(super) limit: Option<usize>,
    #[serde(default)]
    pub(super) target: Option<String>,
    #[serde(default)]
    pub(super) level: Option<String>,
    #[serde(default)]
    pub(super) since: Option<String>,
}

pub(super) fn current_year_month_utc() -> String {
    Utc::now().format("%Y-%m").to_string()
}

/// Validates a `YYYY-MM` string with month range 01..=12.
pub(super) fn validate_year_month(raw: &str) -> Result<(), orbit_core::OrbitError> {
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

pub(super) fn month_bounds_utc(
    raw: &str,
) -> Result<(DateTime<Utc>, DateTime<Utc>), orbit_core::OrbitError> {
    validate_year_month(raw)?;
    let year = raw[..4].parse::<i32>().map_err(|_| {
        orbit_core::OrbitError::InvalidInput(format!("invalid year component in '{raw}'"))
    })?;
    let month = raw[5..].parse::<u32>().map_err(|_| {
        orbit_core::OrbitError::InvalidInput(format!("invalid month component in '{raw}'"))
    })?;
    let start = Utc
        .with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| {
            orbit_core::OrbitError::InvalidInput(format!("invalid month boundary for '{raw}'"))
        })?;
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let next_start = Utc
        .with_ymd_and_hms(next_year, next_month, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| {
            orbit_core::OrbitError::InvalidInput(format!("invalid month boundary for '{raw}'"))
        })?;
    Ok((start, next_start - Duration::nanoseconds(1)))
}

pub(super) fn truncate_to_hour(ts: DateTime<Utc>) -> DateTime<Utc> {
    ts.with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(ts)
}

pub(super) fn bounded_limit(requested: Option<usize>, default: usize) -> usize {
    requested.unwrap_or(default).min(HISTORY_MAX_LIMIT)
}

pub(super) fn validate_id(id: &str) -> Result<&str, String> {
    let valid = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        Ok(id)
    } else {
        Err("id must contain only ASCII letters, digits, '-' or '_'".to_string())
    }
}

pub(super) fn non_empty_string(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(super) fn v2_loop_dir(runtime: &OrbitRuntime) -> PathBuf {
    runtime
        .data_root()
        .join("state")
        .join("audit")
        .join("v2_loop")
}

pub(super) fn map_runtime_error(e: orbit_core::OrbitError) -> Response {
    match e {
        orbit_core::OrbitError::InvalidInput(msg) => bad_request(msg),
        orbit_core::OrbitError::InvalidInputDiagnostic { message, .. } => bad_request(message),
        orbit_core::OrbitError::NotFound {
            kind: orbit_core::NotFoundKind::Task,
            id,
        } => not_found(format!("task not found: {id}")),
        orbit_core::OrbitError::NotFound {
            kind: orbit_core::NotFoundKind::Job,
            id,
        } => not_found(format!("job not found: {id}")),
        orbit_core::OrbitError::NotFound {
            kind: orbit_core::NotFoundKind::JobRun,
            id,
        } => not_found(format!("run not found: {id}")),
        orbit_core::OrbitError::NotFound {
            kind: orbit_core::NotFoundKind::Learning,
            id,
        } => not_found(format!("learning not found: {id}")),
        orbit_core::OrbitError::NotFound {
            kind: orbit_core::NotFoundKind::Adr,
            id,
        } => not_found(format!("ADR not found: {id}")),
        orbit_core::OrbitError::AdrInvalidTransition(message) => {
            bad_request(format!("Invalid ADR status transition: {message}"))
        }
        other => server_error(other),
    }
}

pub(super) fn bad_request(message: String) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
}

pub(super) fn not_found(message: String) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": message }))).into_response()
}

pub(super) fn server_error(e: orbit_core::OrbitError) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": e.to_string() })),
    )
        .into_response()
}

async fn require_localhost_origin(request: Request<Body>, next: Next) -> Response {
    let unsafe_method = matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    let origin = request.headers().get(header::ORIGIN);
    let allowed = origin
        .and_then(|origin| origin.to_str().ok())
        .and_then(|origin| Url::parse(origin).ok())
        .is_some_and(|origin| {
            origin.scheme() == "http"
                && matches!(origin.host_str(), Some("localhost" | "127.0.0.1"))
        });
    if !allowed && (unsafe_method || origin.is_some()) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "cross-origin requests not allowed"})),
        )
            .into_response();
    }
    next.run(request).await
}

pub(super) fn router() -> Router<Arc<OrbitRuntime>> {
    Router::new()
        .route(
            "/tasks",
            get(tasks::list_tasks).post(tasks::create_task_action),
        )
        .route("/tasks/locks", get(tasks::list_task_locks))
        .route(
            "/tasks/:id",
            get(tasks::get_task).patch(tasks::update_task_action),
        )
        .route("/crews", get(crews::list_crews))
        .route("/tasks/:id/artifacts/*path", get(tasks::get_task_artifact))
        .route("/tasks/:id/approve", post(tasks::approve_task_action))
        .route("/tasks/:id/reject", post(tasks::reject_task_action))
        .route("/tasks/:id/archive", post(tasks::archive_task_action))
        .route("/learnings", get(learnings::list_learnings))
        .route("/learnings/:id", get(learnings::get_learning))
        .route(
            "/learnings/:id/supersede",
            post(learnings::supersede_learning_action),
        )
        .route("/adrs", get(adrs::list_adrs))
        .route("/adrs/:id", get(adrs::get_adr))
        .route("/adrs/:id/accept", post(adrs::accept_adr_action))
        .route("/adrs/:id/supersede", post(adrs::supersede_adr_action))
        .route("/frictions", get(frictions::list_frictions))
        .route("/frictions/stats", get(frictions::friction_stats))
        .route(
            "/frictions/:id",
            get(frictions::get_friction).patch(frictions::update_friction_action),
        )
        .route(
            "/frictions/:id/resolve",
            post(frictions::resolve_friction_action),
        )
        .route("/jobs", get(jobs::list_jobs))
        .route("/job-runs", get(jobs::list_job_runs))
        .route("/runs/:id", get(runs::get_run))
        .route("/runs/:id/cancel", post(runs::cancel_run_action))
        .route("/runs/:id/replay", post(runs::replay_run_action))
        .route("/runs/:id/events", get(runs::list_run_events))
        .route("/runs/:id/logs", get(runs::list_run_logs))
        .route("/audit", get(audit::list_audit))
        .route("/log", get(log::get_log))
        .route("/log/stream", get(log::stream_log))
        .route("/audit/summary", get(audit::audit_summary))
        .route("/scoreboard", get(scoreboard::scoreboard))
        .route("/metrics/knowledge", get(metrics::knowledge_metrics))
        .route("/metrics/activity", get(metrics::activity_metrics))
        .route("/metrics/tools", get(metrics::tool_metrics))
        .route("/metrics/task/:id", get(metrics::task_metrics))
        .route("/metrics/invocations", get(metrics::invocation_metrics))
        .route(
            "/diagnostics/metrics",
            get(diagnostics::list_diagnostics_metrics),
        )
        .route(
            "/diagnostics/errors",
            get(diagnostics::list_diagnostics_errors),
        )
        .route(
            "/diagnostics/friction",
            get(diagnostics::list_diagnostics_friction),
        )
        .route(
            "/diagnostics/implement_one",
            get(diagnostics::diagnostics_implement_one),
        )
        .route("/diagnostics/denials", get(denials::list_denials))
        .layer(middleware::from_fn(require_localhost_origin))
}

#[cfg(test)]
mod tests;
