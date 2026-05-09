//! Read-only JSON HTTP handlers for the dashboard.
//!
//! Each handler delegates to the same `*_to_json` helpers used by the CLI's
//! `--json` paths so the wire format stays in lockstep with the CLI.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Duration, TimeZone, Timelike, Utc};
use orbit_core::OrbitRuntime;
use serde::Deserialize;
use serde_json::json;
use url::Url;

mod audit;
mod denials;
mod diagnostics;
mod jobs;
mod log;
mod runs;
mod scoreboard;
mod tasks;
#[cfg(test)]
mod test_support;

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
        orbit_core::OrbitError::TaskNotFound(msg) => not_found(format!("task not found: {msg}")),
        orbit_core::OrbitError::JobNotFound(msg) => not_found(format!("job not found: {msg}")),
        orbit_core::OrbitError::JobRunNotFound(msg) => not_found(format!("run not found: {msg}")),
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
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        let allowed = origin
            .to_str()
            .ok()
            .and_then(|origin| Url::parse(origin).ok())
            .is_some_and(|origin| {
                origin.scheme() == "http"
                    && matches!(origin.host_str(), Some("localhost" | "127.0.0.1"))
            });
        if !allowed {
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
        .route(
            "/tasks",
            get(tasks::list_tasks).post(tasks::create_task_action),
        )
        .route(
            "/tasks/:id",
            get(tasks::get_task).patch(tasks::update_task_action),
        )
        .route("/tasks/:id/approve", post(tasks::approve_task_action))
        .route("/tasks/:id/reject", post(tasks::reject_task_action))
        .route("/tasks/:id/archive", post(tasks::archive_task_action))
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
        .route("/diagnostics/denials", get(denials::list_denials))
        .layer(middleware::from_fn(require_localhost_origin))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode, header};
    use orbit_core::{JobRunState, OrbitRuntime};
    use tower::ServiceExt;

    use super::test_support::seed_run;
    use super::*;

    async fn request_cancel(runtime: OrbitRuntime, run_id: &str, origin: Option<&str>) -> Response {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri(format!("/runs/{run_id}/cancel"));
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        router()
            .with_state(Arc::new(runtime))
            .oneshot(builder.body(Body::empty()).expect("request"))
            .await
            .expect("response")
    }

    #[tokio::test]
    async fn require_localhost_origin_rejects_prefix_match() {
        let cases = [
            ("http://localhost.evil.com", "localhost prefix"),
            ("http://127.0.0.1.evil.com", "127.0.0.1 prefix"),
        ];

        for (index, (origin, label)) in cases.into_iter().enumerate() {
            let runtime = OrbitRuntime::in_memory().expect("build runtime");
            let run = seed_run(
                &runtime,
                &format!("jrun-web-cancel-prefix-{index}"),
                "web_cancel_prefix",
                JobRunState::Pending,
            );

            let response = request_cancel(runtime.clone(), &run.run_id, Some(origin)).await;

            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{label}");
            let stored = runtime.show_job_run(&run.run_id).expect("show run");
            assert_eq!(stored.state, JobRunState::Pending, "{label}");
        }
    }

    #[tokio::test]
    async fn require_localhost_origin_rejects_https_origin() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let run = seed_run(
            &runtime,
            "jrun-web-cancel-https-origin",
            "web_cancel_https_origin",
            JobRunState::Pending,
        );

        let response =
            request_cancel(runtime.clone(), &run.run_id, Some("https://localhost")).await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let stored = runtime.show_job_run(&run.run_id).expect("show run");
        assert_eq!(stored.state, JobRunState::Pending);
    }

    #[tokio::test]
    async fn require_localhost_origin_accepts_localhost_with_port() {
        let cases = [
            ("http://localhost:7878", "localhost"),
            ("http://127.0.0.1:7878", "127-0-0-1"),
        ];

        for (origin, label) in cases {
            let runtime = OrbitRuntime::in_memory().expect("build runtime");
            let run = seed_run(
                &runtime,
                &format!("jrun-web-cancel-origin-port-{label}"),
                "web_cancel_origin_port",
                JobRunState::Pending,
            );

            let response = request_cancel(runtime.clone(), &run.run_id, Some(origin)).await;

            assert_eq!(response.status(), StatusCode::OK, "{label}");
            let stored = runtime.show_job_run(&run.run_id).expect("show run");
            assert_eq!(stored.state, JobRunState::Cancelled, "{label}");
        }
    }

    #[tokio::test]
    async fn require_localhost_origin_blocks_cross_origin_get_with_attacker_origin() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");

        let response = router()
            .with_state(Arc::new(runtime))
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/tasks")
                    .header(header::ORIGIN, "http://localhost.evil.com")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
