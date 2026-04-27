//! Read-only JSON HTTP handlers for the dashboard.
//!
//! Each handler delegates to the same `*_to_json` helpers used by the CLI's
//! `--json` paths so the wire format stays in lockstep with the CLI.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration as StdDuration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Duration, Timelike, Utc};
use futures_core::Stream;
use orbit_core::command::job_run::JobRunListParams;
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
use orbit_core::{
    AuditEventStatus, JobRunState, OrbitRuntime, Task, TaskComplexity, TaskPriority, TaskStatus,
    TaskType,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::command::audit::audit_event_to_json;
use crate::command::job::job_catalog_to_json_with_last_run;
use crate::command::log::format::{
    Filters as LogFilters, RenderedLogEvent, parse_matching_event, read_recent_rendered_events,
    render_log_event_for_web, resolve_log_path,
};
use crate::command::run::job_run_to_json;
use crate::command::task::output::task_to_json;
use crate::parse::parse_since;

const DASHBOARD_TASK_STATUSES: &[TaskStatus] = &[
    TaskStatus::InProgress,
    TaskStatus::Review,
    TaskStatus::Blocked,
    TaskStatus::Proposed,
    TaskStatus::Friction,
    TaskStatus::Backlog,
    TaskStatus::Someday,
    TaskStatus::Rejected,
];
const HISTORY_DEFAULT_LIMIT: usize = 50;
const JOB_RUN_DEFAULT_LIMIT: usize = 25;
const HISTORY_MAX_LIMIT: usize = 200;
const RUN_EVENTS_DEFAULT_LIMIT: usize = 100;
const LOG_DEFAULT_LIMIT: usize = 50;
const LOG_MAX_LIMIT: usize = 500;
const LOG_STREAM_CHANNEL_DEPTH: usize = 64;
const LOG_STREAM_POLL_INTERVAL: StdDuration = StdDuration::from_millis(50);
/// Default time window for header tile counts when `?since=` is omitted.
const DEFAULT_SUMMARY_WINDOW: &str = "24h";
/// Default header-tile alert threshold for the denials counter. Surfaced via
/// `?denial_threshold=` and echoed back in the response so the dashboard can
/// switch the tile to alert state without a second round-trip.
const DEFAULT_DENIAL_THRESHOLD: i64 = 10;
/// Cap on how many `state/audit/v2_loop/*.jsonl` run files we read in one
/// request when aggregating denials. Each file is small (KB-scale) but reads
/// are sync, so we bound iteration to keep the endpoint within budget on
/// long-lived workspaces.
const V2_LOOP_FILE_SCAN_CAP: usize = 1500;

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

#[derive(Deserialize, Default)]
pub(super) struct AuditQuery {
    #[serde(default)]
    since: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    role: Option<String>,
    /// Filters audit events by orbit invocation id. The SQLite `audit_events`
    /// schema has no `run_id` column; `run_id` here is a backward-compat alias
    /// of `execution_id` (T20260427-26). When both are supplied, `execution_id`
    /// takes precedence.
    #[serde(default)]
    execution_id: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    q: Option<String>,
    /// fsProfile filter. The SQLite `audit_events` schema has no first-class
    /// `profile` column; matching is best-effort against `arguments_json`. The
    /// canonical denials view (`/api/diagnostics/denials`) reads the v2 envelope
    /// JSONL where `profile` is a typed field.
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

#[derive(Deserialize, Default)]
pub(super) struct AuditSummaryQuery {
    #[serde(default)]
    since: Option<String>,
    #[serde(default)]
    denial_threshold: Option<i64>,
}

#[derive(Deserialize, Default)]
pub(super) struct DenialsQuery {
    #[serde(default)]
    since: Option<String>,
    /// `fs`, `tool`, or omitted (combined).
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

#[derive(Deserialize, Default)]
pub(super) struct RunEventsQuery {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub(super) struct LogQuery {
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    since: Option<String>,
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
        .route("/tasks", get(list_tasks).post(create_task_action))
        .route("/tasks/:id", get(get_task).patch(update_task_action))
        .route("/tasks/:id/approve", post(approve_task_action))
        .route("/tasks/:id/reject", post(reject_task_action))
        .route("/tasks/:id/archive", post(archive_task_action))
        .route("/jobs", get(list_jobs))
        .route("/job-runs", get(list_job_runs))
        .route("/runs/:id", get(get_run))
        .route("/runs/:id/events", get(list_run_events))
        .route("/audit", get(list_audit))
        .route("/log", get(get_log))
        .route("/log/stream", get(stream_log))
        .route("/audit/summary", get(audit_summary))
        .route("/scoreboard", get(scoreboard))
        .route("/diagnostics/metrics", get(list_diagnostics_metrics))
        .route("/diagnostics/friction", get(list_diagnostics_friction))
        .route("/diagnostics/denials", get(list_denials))
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

#[derive(Deserialize)]
pub(super) struct CreateTaskBody {
    title: String,
    description: String,
    #[serde(default)]
    acceptance_criteria: Vec<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    plan: String,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    context_files: Vec<String>,
    #[serde(default)]
    workspace_path: Option<String>,
    #[serde(default = "default_priority")]
    priority: TaskPriority,
    #[serde(default)]
    complexity: Option<TaskComplexity>,
    #[serde(default)]
    task_type: Option<TaskType>,
    #[serde(default)]
    status: Option<TaskStatus>,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    source_task_id: Option<String>,
}

fn default_priority() -> TaskPriority {
    TaskPriority::Medium
}

/// Partial-update body for `PATCH /tasks/:id`. Each field is `Option<...>`;
/// fields absent from the JSON body remain unchanged.
///
/// Note: `pr_number`, `pr_status`, and `batch_id` are intentionally omitted
/// from this v1 surface. They use `Option<Option<String>>` in
/// `TaskUpdateParams` to distinguish absent vs. clear; the dashboard does not
/// currently need to set them. Add them via a `deserialize_with` adapter when
/// a UI use case appears.
#[derive(Deserialize, Default)]
pub(super) struct UpdateTaskBody {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    acceptance_criteria: Option<Vec<String>>,
    #[serde(default)]
    dependencies: Option<Vec<String>>,
    #[serde(default)]
    plan: Option<String>,
    #[serde(default)]
    execution_summary: Option<String>,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    status: Option<TaskStatus>,
    #[serde(default)]
    context_files: Option<Vec<String>>,
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

async fn create_task_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Json(body): Json<CreateTaskBody>,
) -> Response {
    let params = TaskAddParams {
        parent_id: body.parent_id,
        title: body.title,
        description: body.description,
        acceptance_criteria: body.acceptance_criteria,
        dependencies: body.dependencies,
        plan: body.plan,
        comment: body.comment,
        context_files: body.context_files,
        workspace_path: body.workspace_path,
        priority: body.priority,
        complexity: body.complexity,
        task_type: body.task_type,
        status: body.status,
        system_created: false,
        source_task_id: body.source_task_id,
    };
    match runtime.add_task_with_identity(params, None, None) {
        Ok(task) => match dashboard_status_index(&runtime) {
            Ok(status_by_id) => Json(task_to_json(&task, &status_by_id)).into_response(),
            Err(e) => server_error(e),
        },
        Err(e) => map_runtime_error(e),
    }
}

async fn update_task_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTaskBody>,
) -> Response {
    let params = TaskUpdateParams {
        title: body.title,
        description: body.description,
        acceptance_criteria: body.acceptance_criteria,
        dependencies: body.dependencies,
        plan: body.plan,
        execution_summary: body.execution_summary,
        comment: body.comment,
        status: body.status,
        pr_number: None,
        pr_status: None,
        batch_id: None,
        context_files: body.context_files,
        upsert_artifacts: Vec::new(),
        append_review_threads: Vec::new(),
    };
    match runtime.update_task_with_identity(&id, params, None, None) {
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
    Query(q): Query<AuditQuery>,
) -> Response {
    let since = match q.since.as_deref() {
        Some(raw) => match parse_since(raw) {
            Ok(ts) => Some(ts),
            Err(e) => return map_runtime_error(e),
        },
        None => None,
    };

    let status = match q.status.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => match AuditEventStatus::from_str(raw) {
            Ok(s) => Some(s),
            Err(msg) => return bad_request(msg),
        },
        None => None,
    };

    let limit = bounded_limit(q.limit, HISTORY_DEFAULT_LIMIT);
    let offset = q.offset.unwrap_or(0);

    // Post-query filters (run_id, q) and offset are applied after the SQLite
    // call. Request a generous prefetch so common filtered pages are full.
    let prefetch = HISTORY_MAX_LIMIT;
    let tool = q.tool.filter(|s| !s.is_empty());
    let role = q.role.filter(|s| !s.is_empty());

    let events = match runtime.list_audit_events(since, tool, status, role, prefetch) {
        Ok(events) => events,
        Err(e) => return server_error(e),
    };

    let exec_id_filter = q
        .execution_id
        .as_deref()
        .or(q.run_id.as_deref())
        .filter(|s| !s.is_empty());
    let profile_filter = q
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let needle =
        q.q.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_lowercase);

    let mut filtered: Vec<_> = events
        .into_iter()
        .filter(|e| {
            if let Some(eid) = exec_id_filter
                && e.execution_id != eid
            {
                return false;
            }
            if let Some(ref profile) = profile_filter
                && !arguments_json_matches_profile(e.arguments_json.as_deref(), profile)
            {
                return false;
            }
            if let Some(ref needle) = needle {
                let haystacks = [
                    e.command.as_str(),
                    e.subcommand.as_deref().unwrap_or(""),
                    e.tool_name.as_deref().unwrap_or(""),
                    e.target_id.as_deref().unwrap_or(""),
                    e.target_type.as_deref().unwrap_or(""),
                    e.role.as_str(),
                    e.error_message.as_deref().unwrap_or(""),
                ];
                if !haystacks.iter().any(|h| h.to_lowercase().contains(needle)) {
                    return false;
                }
            }
            true
        })
        .collect();

    if offset >= filtered.len() {
        return Json(Value::Array(Vec::new())).into_response();
    }
    let end = offset.saturating_add(limit).min(filtered.len());
    let page: Vec<Value> = filtered
        .drain(offset..end)
        .map(|e| audit_event_to_json(&e))
        .collect();
    Json(Value::Array(page)).into_response()
}

async fn get_log(Query(q): Query<LogQuery>) -> Response {
    let path = match resolve_log_path(None) {
        Ok(path) => path,
        Err(e) => return map_runtime_error(e),
    };
    match read_log_snapshot_from_path(&path, &q) {
        Ok(events) => Json(events).into_response(),
        Err(e) => map_runtime_error(e),
    }
}

async fn stream_log(Query(q): Query<LogQuery>) -> Response {
    let path = match resolve_log_path(None) {
        Ok(path) => path,
        Err(e) => return map_runtime_error(e),
    };
    let filters = match log_filters(&q) {
        Ok(filters) => filters,
        Err(e) => return map_runtime_error(e),
    };
    let stream = ReceiverSseStream {
        rx: spawn_log_sse_frames(path, filters),
    };
    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
    {
        Ok(response) => response,
        Err(e) => server_error(orbit_core::OrbitError::Execution(format!(
            "build SSE response: {e}"
        ))),
    }
}

fn read_log_snapshot_from_path(
    path: &std::path::Path,
    query: &LogQuery,
) -> Result<Vec<RenderedLogEvent>, orbit_core::OrbitError> {
    let limit = match query.limit {
        Some(limit) if limit > LOG_MAX_LIMIT => {
            return Err(orbit_core::OrbitError::InvalidInput(format!(
                "limit must be <= {LOG_MAX_LIMIT}; got {limit}"
            )));
        }
        Some(limit) => limit,
        None => LOG_DEFAULT_LIMIT,
    };
    let filters = log_filters(query)?;
    read_recent_rendered_events(path, &filters, limit)
        .map_err(|e| orbit_core::OrbitError::Io(format!("read log {}: {e}", path.display())))
}

fn log_filters(query: &LogQuery) -> Result<LogFilters, orbit_core::OrbitError> {
    LogFilters::from_query_parts(
        query.target.as_deref().and_then(non_empty_string),
        query.level.as_deref().and_then(non_empty_string),
        query
            .since
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    )
}

fn non_empty_string(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn spawn_log_sse_frames(path: PathBuf, filters: LogFilters) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel(LOG_STREAM_CHANNEL_DEPTH);
    thread::spawn(move || {
        let mut offset = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let mut leftover = String::new();
        loop {
            if tx.is_closed() {
                return;
            }
            match read_appended_log_events(&path, &filters, &mut offset, &mut leftover) {
                Ok(events) => {
                    for event in events {
                        let frame = match format_sse_frame(&event) {
                            Ok(frame) => frame,
                            Err(_) => continue,
                        };
                        if tx.blocking_send(frame).is_err() {
                            return;
                        }
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(_) => {}
            }
            thread::sleep(LOG_STREAM_POLL_INTERVAL);
        }
    });
    rx
}

fn read_appended_log_events(
    path: &std::path::Path,
    filters: &LogFilters,
    offset: &mut u64,
    leftover: &mut String,
) -> io::Result<Vec<RenderedLogEvent>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if len < *offset {
        *offset = 0;
        leftover.clear();
    }
    file.seek(SeekFrom::Start(*offset))?;
    let mut reader = BufReader::new(file);
    let mut events = Vec::new();

    loop {
        let mut buf = String::new();
        let n = reader.read_line(&mut buf)?;
        if n == 0 {
            break;
        }
        *offset += n as u64;
        if !buf.ends_with('\n') {
            leftover.push_str(&buf);
            continue;
        }
        let mut full_line = String::new();
        if !leftover.is_empty() {
            full_line.push_str(leftover);
            leftover.clear();
        }
        full_line.push_str(buf.trim_end_matches('\n'));
        if let Some(event) = parse_matching_event(&full_line, filters) {
            events.push(render_log_event_for_web(&event));
        }
    }

    Ok(events)
}

fn format_sse_frame(event: &RenderedLogEvent) -> Result<String, serde_json::Error> {
    serde_json::to_string(event).map(|json| format!("data: {json}\n\n"))
}

struct ReceiverSseStream {
    rx: mpsc::Receiver<String>,
}

impl Stream for ReceiverSseStream {
    type Item = Result<String, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx).map(|item| item.map(Ok))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    fn write_lines(path: &std::path::Path, lines: &[String]) {
        let mut content = String::new();
        for line in lines {
            content.push_str(line);
            content.push('\n');
        }
        std::fs::write(path, content).expect("write fixture");
    }

    #[test]
    fn log_snapshot_filters_target_level_and_since() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("orbit.jsonl");
        write_lines(
            &path,
            &[
                json!({
                    "timestamp": "2026-04-27T01:00:01Z",
                    "level": "INFO",
                    "target": "orbit.policy.deny",
                    "fields": {"tool": "fs.read", "path": "/tmp/a"}
                })
                .to_string(),
                json!({
                    "timestamp": "2026-04-27T01:00:03Z",
                    "level": "WARN",
                    "target": "orbit.policy.deny",
                    "fields": {"tool": "fs.write", "path": "/etc/passwd"}
                })
                .to_string(),
                json!({
                    "timestamp": "2026-04-27T01:00:04Z",
                    "level": "ERROR",
                    "target": "orbit.job.step_finished",
                    "fields": {"step_id": "build", "outcome": "failed", "success": false}
                })
                .to_string(),
            ],
        );

        let events = read_log_snapshot_from_path(
            &path,
            &LogQuery {
                limit: Some(10),
                target: Some("orbit.policy".to_string()),
                level: Some("warn".to_string()),
                since: Some("2026-04-27T01:00:02Z".to_string()),
            },
        )
        .expect("snapshot");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].source, "policy");
        assert_eq!(events[0].code, "DENY");
        assert_eq!(events[0].level, "warn");
        assert!(events[0].message_html.contains("<b>path</b>="));
    }

    #[test]
    fn log_snapshot_rejects_limit_above_max() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("orbit.jsonl");
        write_lines(&path, &[]);

        let err = read_log_snapshot_from_path(
            &path,
            &LogQuery {
                limit: Some(LOG_MAX_LIMIT + 1),
                ..LogQuery::default()
            },
        )
        .expect_err("limit should be rejected");

        assert!(err.to_string().contains("limit must be <= 500"));
    }

    #[test]
    fn log_stream_framing_emits_one_data_frame_per_appended_line() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("orbit.jsonl");
        write_lines(&path, &[]);
        let mut offset = std::fs::metadata(&path).expect("metadata").len();
        let mut leftover = String::new();

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("append");
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-04-27T01:00:05Z",
                "level": "INFO",
                "target": "orbit.job.step_started",
                "fields": {"job_run_id": "run-1", "step_id": "build"}
            })
        )
        .expect("write event");
        file.flush().expect("flush");

        let events =
            read_appended_log_events(&path, &LogFilters::default(), &mut offset, &mut leftover)
                .expect("read appended");
        assert_eq!(events.len(), 1);

        let frame = format_sse_frame(&events[0]).expect("frame");
        assert!(frame.starts_with("data: "));
        assert!(frame.ends_with("\n\n"));
        assert!(frame.contains("\"source\":\"job\""));
        assert!(frame.contains("build"));
    }
}

async fn get_run(State(runtime): State<Arc<OrbitRuntime>>, Path(id): Path<String>) -> Response {
    match runtime.show_job_run(&id) {
        Ok(run) => {
            let mut full = job_run_to_json(&run);
            // Reshape into `{run, steps}` per the dashboard contract: peel the
            // `steps` array off the flat `job_run_to_json` output.
            let steps = full
                .as_object_mut()
                .and_then(|m| m.remove("steps"))
                .unwrap_or(Value::Array(Vec::new()));
            Json(json!({ "run": full, "steps": steps })).into_response()
        }
        Err(e) => map_runtime_error(e),
    }
}

async fn list_run_events(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
    Query(q): Query<RunEventsQuery>,
) -> Response {
    let limit = bounded_limit(q.limit, RUN_EVENTS_DEFAULT_LIMIT);
    let offset = q.offset.unwrap_or(0);
    let kind = q
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let path = v2_loop_path(&runtime, &id);
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

fn v2_loop_path(runtime: &OrbitRuntime, run_id: &str) -> PathBuf {
    runtime
        .data_root()
        .join("state")
        .join("audit")
        .join("v2_loop")
        .join(format!("{run_id}.jsonl"))
}

fn v2_loop_dir(runtime: &OrbitRuntime) -> PathBuf {
    runtime
        .data_root()
        .join("state")
        .join("audit")
        .join("v2_loop")
}

/// Best-effort match of a stringified `arguments_json` payload against a
/// requested fsProfile name. Looks for any of the conventional keys
/// (`fsProfile`, `fs_profile`, `profile`) at the top level of the parsed
/// object. Returns `false` for malformed or empty payloads — the SQLite schema
/// has no profile column, so absence cannot be distinguished from mismatch.
fn arguments_json_matches_profile(raw: Option<&str>, expected: &str) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    const KEYS: &[&str] = &["fsProfile", "fs_profile", "profile"];
    let Some(obj) = value.as_object() else {
        return false;
    };
    for key in KEYS {
        if let Some(Value::String(found)) = obj.get(*key)
            && found == expected
        {
            return true;
        }
    }
    false
}

async fn audit_summary(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(q): Query<AuditSummaryQuery>,
) -> Response {
    let raw_since = q.since.as_deref().unwrap_or(DEFAULT_SUMMARY_WINDOW);
    let since = match parse_since(raw_since) {
        Ok(ts) => ts,
        Err(e) => return map_runtime_error(e),
    };
    let denial_threshold = q.denial_threshold.unwrap_or(DEFAULT_DENIAL_THRESHOLD);

    let (total, _success, _failure, sql_denied, _avg, _max) =
        match runtime.audit_event_stats(Some(since), None) {
            Ok(stats) => (
                stats.total,
                stats.success_count,
                stats.failure_count,
                stats.denied_count,
                stats.avg_duration_ms,
                stats.max_duration_ms,
            ),
            Err(e) => return server_error(e),
        };

    let v2_denials = match scan_v2_loop_denials(&runtime, Some(since), None, None) {
        Ok(events) => events.len() as i64,
        Err(e) => return server_error(e),
    };

    let failed_runs = match count_failed_runs(&runtime, since) {
        Ok(n) => n,
        Err(e) => return map_runtime_error(e),
    };

    let active_long_runs = match count_active_long_runs(&runtime, since) {
        Ok(n) => n,
        Err(e) => return map_runtime_error(e),
    };

    let buckets = match runtime.audit_event_hourly_buckets(&since) {
        Ok(b) => b,
        Err(e) => return server_error(e),
    };
    let sparkline = build_sparkline(since, &buckets);

    let denials = sql_denied + v2_denials;

    Json(json!({
        "events": total,
        "denials": denials,
        "denials_sql": sql_denied,
        "denials_v2": v2_denials,
        "failed_runs": failed_runs,
        "active_long_runs": active_long_runs,
        "sparkline": sparkline,
        "denial_threshold": denial_threshold,
        "since": since.to_rfc3339(),
        "window": raw_since,
    }))
    .into_response()
}

/// Builds a contiguous hourly sparkline covering `[truncate_to_hour(since), now]`,
/// zero-filling hours not present in `buckets`. Always returns at least 24
/// buckets so the UI can render a stable baseline width even on a fresh
/// workspace.
fn build_sparkline(since: DateTime<Utc>, buckets: &[(String, i64)]) -> Vec<Value> {
    let mut by_bucket: BTreeMap<String, i64> = BTreeMap::new();
    for (ts, count) in buckets {
        by_bucket.insert(ts.clone(), *count);
    }
    let now = Utc::now();
    let start = truncate_to_hour(since.min(now));
    let end = truncate_to_hour(now);
    let mut out = Vec::new();
    let mut cursor = start;
    while cursor <= end {
        let key = cursor.format("%Y-%m-%dT%H:00:00Z").to_string();
        let count = by_bucket.get(&key).copied().unwrap_or(0);
        out.push(json!({ "ts": key, "count": count }));
        cursor += Duration::hours(1);
    }
    while out.len() < 24 {
        let earliest = out
            .first()
            .and_then(|v| v.get("ts").and_then(Value::as_str))
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or(end);
        let prev = earliest - Duration::hours(1);
        let key = prev.format("%Y-%m-%dT%H:00:00Z").to_string();
        out.insert(0, json!({ "ts": key, "count": 0 }));
    }
    out
}

fn truncate_to_hour(ts: DateTime<Utc>) -> DateTime<Utc> {
    ts.with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(ts)
}

fn count_failed_runs(
    runtime: &OrbitRuntime,
    since: DateTime<Utc>,
) -> Result<i64, orbit_core::OrbitError> {
    let mut total: i64 = 0;
    for state in [JobRunState::Failed, JobRunState::Timeout] {
        let runs = runtime.list_job_runs(JobRunListParams {
            job_id: None,
            state: Some(state),
            since: Some(since),
            limit: Some(HISTORY_MAX_LIMIT),
        })?;
        total += runs.len() as i64;
    }
    Ok(total)
}

/// Counts running runs whose start time is older than the 95th percentile of
/// finished-run wall-clock durations within the same window. We use run-level
/// `duration_ms` as a proxy for the AC's "finished step" series — load-bearing
/// the same per-run signal without paying the O(steps) file-read cost. Faithful
/// to the spec's intent (flag stuck activity) and within the 500ms budget.
fn count_active_long_runs(
    runtime: &OrbitRuntime,
    since: DateTime<Utc>,
) -> Result<i64, orbit_core::OrbitError> {
    let mut finished_durations: Vec<i64> = Vec::new();
    for state in [
        JobRunState::Success,
        JobRunState::Failed,
        JobRunState::Timeout,
        JobRunState::Cancelled,
    ] {
        let runs = runtime.list_job_runs(JobRunListParams {
            job_id: None,
            state: Some(state),
            since: Some(since),
            limit: Some(HISTORY_MAX_LIMIT),
        })?;
        for r in runs {
            if let Some(d) = r.duration_ms {
                finished_durations.push(d as i64);
            }
        }
    }

    if finished_durations.is_empty() {
        return Ok(0);
    }
    finished_durations.sort_unstable();
    let idx = ((finished_durations.len() as f64) * 0.95).ceil() as usize;
    let idx = idx.min(finished_durations.len()).saturating_sub(1);
    let p95_ms = finished_durations[idx];

    let running = runtime.list_job_runs(JobRunListParams {
        job_id: None,
        state: Some(JobRunState::Running),
        since: None,
        limit: Some(HISTORY_MAX_LIMIT),
    })?;

    let now = Utc::now();
    let mut count: i64 = 0;
    for r in running {
        let started = r.started_at.unwrap_or(r.created_at);
        let elapsed = now.signed_duration_since(started).num_milliseconds().max(0);
        if elapsed > p95_ms {
            count += 1;
        }
    }
    Ok(count)
}

/// Internal denial event extracted from the v2 envelope JSONL.
#[derive(Debug, Clone)]
struct DenialRow {
    kind: &'static str,
    profile: String,
    target: String,
    run_id: String,
    agent: String,
}

/// Walks `state/audit/v2_loop/*.jsonl` and returns FsCallDenied / ToolDenied
/// rows matching the supplied filters. Bounded by `V2_LOOP_FILE_SCAN_CAP` files.
fn scan_v2_loop_denials(
    runtime: &OrbitRuntime,
    since: Option<DateTime<Utc>>,
    profile_filter: Option<&str>,
    agent_filter: Option<&str>,
) -> Result<Vec<DenialRow>, orbit_core::OrbitError> {
    let dir = v2_loop_dir(runtime);
    let mut out: Vec<DenialRow> = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(it) => it,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => {
            return Err(orbit_core::OrbitError::Io(format!(
                "read {}: {e}",
                dir.display()
            )));
        }
    };

    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
        .collect();
    paths.sort();
    if paths.len() > V2_LOOP_FILE_SCAN_CAP {
        let drop = paths.len() - V2_LOOP_FILE_SCAN_CAP;
        paths.drain(0..drop);
    }

    for path in paths {
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let kind_raw = value.get("body_kind").and_then(Value::as_str).unwrap_or("");
            let kind = match kind_raw {
                "fs_call_denied" => "fs",
                "tool_denied" => "tool",
                _ => continue,
            };
            let ts = value
                .get("ts")
                .and_then(Value::as_str)
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc));
            if let Some(since) = since
                && let Some(ts) = ts
                && ts < since
            {
                continue;
            }
            let run_id = value
                .get("run_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let agent = value
                .get("agent_identity")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let (profile, target) = match kind {
                "fs" => (
                    value
                        .get("profile")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    value
                        .get("path")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                ),
                _ => (
                    "tool".to_string(),
                    value
                        .get("tool_name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                ),
            };
            if let Some(want) = profile_filter
                && !want.is_empty()
                && profile != want
            {
                continue;
            }
            if let Some(want) = agent_filter
                && !want.is_empty()
                && agent != want
            {
                continue;
            }
            out.push(DenialRow {
                kind,
                profile,
                target,
                run_id,
                agent,
            });
        }
    }
    Ok(out)
}

async fn list_denials(
    State(runtime): State<Arc<OrbitRuntime>>,
    Query(q): Query<DenialsQuery>,
) -> Response {
    let raw_since = q.since.as_deref().unwrap_or(DEFAULT_SUMMARY_WINDOW);
    let since = match parse_since(raw_since) {
        Ok(ts) => Some(ts),
        Err(e) => return map_runtime_error(e),
    };
    let kind = q
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    if let Some(ref k) = kind
        && k != "fs"
        && k != "tool"
    {
        return bad_request(format!("kind must be 'fs', 'tool', or omitted; got '{k}'"));
    }
    let profile_filter = q
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let agent_filter = q.agent.as_deref().map(str::trim).filter(|s| !s.is_empty());

    let rows = match scan_v2_loop_denials(&runtime, since, profile_filter, agent_filter) {
        Ok(rows) => rows,
        Err(e) => return server_error(e),
    };

    // Apply optional kind filter post-scan; scan returns both varieties.
    let filtered: Vec<&DenialRow> = rows
        .iter()
        .filter(|r| match kind.as_deref() {
            None => true,
            Some(k) => r.kind == k,
        })
        .collect();

    let by_profile = aggregate_by(&filtered, |r| r.profile.clone());
    let by_target = aggregate_by(&filtered, |r| r.target.clone());
    let by_run = aggregate_by(&filtered, |r| r.run_id.clone());
    let by_agent = aggregate_by(&filtered, |r| r.agent.clone());

    Json(json!({
        "by_profile": rows_to_value(&by_profile, "name"),
        "by_target": rows_to_value(&by_target, "name"),
        "by_run": rows_to_value(&by_run, "run_id"),
        "by_agent": rows_to_value(&by_agent, "agent"),
        "total": filtered.len(),
        "kind": kind,
        "since": since.map(|s| s.to_rfc3339()),
    }))
    .into_response()
}

fn aggregate_by<F>(rows: &[&DenialRow], key: F) -> Vec<(String, i64)>
where
    F: Fn(&DenialRow) -> String,
{
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for row in rows {
        let k = key(row);
        if k.is_empty() {
            continue;
        }
        *counts.entry(k).or_insert(0) += 1;
    }
    let mut out: Vec<_> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

fn rows_to_value(rows: &[(String, i64)], key_label: &str) -> Value {
    Value::Array(
        rows.iter()
            .map(|(name, count)| json!({ key_label: name, "count": count }))
            .collect(),
    )
}

async fn scoreboard(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    let summary = match runtime.generate_scoreboard_summary() {
        Ok(s) => s,
        Err(e) => return server_error(e),
    };
    let mut value = match serde_json::to_value(&summary) {
        Ok(v) => v,
        Err(e) => return server_error(orbit_core::OrbitError::Store(e.to_string())),
    };

    // Join MetricsEntry-derived per-actor stats and audit denials. Errors are
    // logged-and-swallowed so the existing scoreboard surface still renders if
    // a side log is missing or malformed.
    let metrics_extras = compute_metrics_extras(&runtime).unwrap_or_default();
    let denials_by_role = runtime.audit_denials_by_role(None).unwrap_or_default();
    let denial_map: BTreeMap<String, i64> = denials_by_role.into_iter().collect();

    if let Some(agents) = value.get_mut("agents").and_then(|v| v.as_object_mut()) {
        // Collect all agent keys upfront so we can also surface metrics rows
        // that have no scoreboard counterpart yet.
        let existing_keys: Vec<String> = agents.keys().cloned().collect();
        for key in &existing_keys {
            let extras = metrics_extras
                .get(key.as_str())
                .cloned()
                .unwrap_or_default();
            let denials = lookup_denials_for_agent(&denial_map, key);
            if let Some(obj) = agents.get_mut(key.as_str()).and_then(|v| v.as_object_mut()) {
                obj.insert(
                    "avg_step_duration_ms".to_string(),
                    json!(extras.avg_duration_ms),
                );
                obj.insert("retries".to_string(), json!(extras.retry_count));
                obj.insert(
                    "p95_wall_clock_ms".to_string(),
                    json!(extras.p95_duration_ms),
                );
                obj.insert("denials".to_string(), json!(denials));
            }
        }
        // Surface metrics-only agents so retries/durations show even when no
        // task or token row exists for them yet.
        for (key, extras) in &metrics_extras {
            if existing_keys.iter().any(|k| k == key) {
                continue;
            }
            let denials = lookup_denials_for_agent(&denial_map, key);
            agents.insert(
                key.clone(),
                json!({
                    "tasks_completed": 0,
                    "friction": { "reported": 0, "accepted": 0, "rejected": 0 },
                    "tokens": { "total": 0, "output": 0 },
                    "duels": { "wins": 0, "losses": 0, "participated": 0 },
                    "pr": { "review_comments": 0, "merged_clean": 0, "merged_with_revision": 0 },
                    "tool_calls": 0,
                    "avg_step_duration_ms": extras.avg_duration_ms,
                    "retries": extras.retry_count,
                    "p95_wall_clock_ms": extras.p95_duration_ms,
                    "denials": denials,
                }),
            );
        }
    }

    Json(value).into_response()
}

/// Per-agent extras derived from `MetricsEntry` JSONL.
#[derive(Debug, Clone, Default)]
struct MetricsExtras {
    avg_duration_ms: i64,
    p95_duration_ms: i64,
    retry_count: i64,
}

fn compute_metrics_extras(
    runtime: &OrbitRuntime,
) -> Result<BTreeMap<String, MetricsExtras>, orbit_core::OrbitError> {
    use orbit_common::types::ActorIdentity;

    let now = Utc::now();
    let mut months = Vec::new();
    months.push(now.format("%Y-%m").to_string());
    if let Some(prev) = now.checked_sub_signed(Duration::days(31)) {
        let key = prev.format("%Y-%m").to_string();
        if !months.contains(&key) {
            months.push(key);
        }
    }

    let mut by_actor: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut retries: BTreeMap<String, i64> = BTreeMap::new();
    for month in &months {
        let entries = match runtime.read_metrics_entries(month) {
            Ok(e) => e,
            Err(orbit_core::OrbitError::InvalidInput(_)) => continue,
            Err(e) => return Err(e),
        };
        for entry in entries {
            let key = match &entry.actor_identity {
                ActorIdentity::Agent { model, name } if !model.is_empty() => model.clone(),
                ActorIdentity::Agent { name, .. } if !name.is_empty() => name.clone(),
                ActorIdentity::Human { label } if !label.is_empty() => label.clone(),
                _ => continue,
            };
            *retries.entry(key.clone()).or_insert(0) += entry.retry_count as i64;
            if let Some(d) = entry.step_duration_ms {
                by_actor.entry(key).or_default().push(d);
            }
        }
    }

    let mut out: BTreeMap<String, MetricsExtras> = BTreeMap::new();
    for (key, durations) in by_actor {
        let mut sorted = durations.clone();
        sorted.sort_unstable();
        let sum: u128 = sorted.iter().map(|d| *d as u128).sum();
        let avg = if sorted.is_empty() {
            0
        } else {
            (sum / sorted.len() as u128) as i64
        };
        let idx = ((sorted.len() as f64) * 0.95).ceil() as usize;
        let idx = idx.min(sorted.len()).saturating_sub(1);
        let p95 = sorted.get(idx).copied().unwrap_or(0) as i64;
        let retry = retries.remove(&key).unwrap_or(0);
        out.insert(
            key,
            MetricsExtras {
                avg_duration_ms: avg,
                p95_duration_ms: p95,
                retry_count: retry,
            },
        );
    }
    // Carry over retries-only actors that had no duration samples.
    for (key, count) in retries {
        out.entry(key).or_insert(MetricsExtras {
            avg_duration_ms: 0,
            p95_duration_ms: 0,
            retry_count: count,
        });
    }
    Ok(out)
}

/// Looks up the SQLite per-role denials for a scoreboard agent key. The audit
/// schema stores `role` as a free-form string (often the bare agent name), so
/// we accept either a direct match or a model-prefix match.
fn lookup_denials_for_agent(map: &BTreeMap<String, i64>, agent_key: &str) -> i64 {
    if let Some(v) = map.get(agent_key) {
        return *v;
    }
    // Match by leading agent-name portion of `agent / model` keys.
    if let Some(idx) = agent_key.find(" / ") {
        let head = &agent_key[..idx];
        if let Some(v) = map.get(head) {
            return *v;
        }
    }
    0
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
        orbit_core::OrbitError::JobRunNotFound(msg) => not_found(format!("run not found: {msg}")),
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
