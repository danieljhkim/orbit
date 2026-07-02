//! Cross-task review-thread surface: list, reply, resolve, and re-open.
//!
//! The list endpoint walks all tasks and projects threads into a flat shape
//! tagged with task id and author identity so the dashboard threads panel
//! does not have to fan out across the per-task endpoints. Write paths
//! delegate to the same `OrbitRuntime` methods the MCP tools use.

use crate::state::Ws;
use axum::extract::{Path, Query};
use axum::response::{IntoResponse, Json, Response};
use chrono::{DateTime, Utc};
use orbit_common::types::{ReviewMessage, ReviewThread, ReviewThreadStatus, all_agent_families};
use orbit_core::TaskStatus;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{bad_request, map_runtime_error, server_error, validate_id};

#[derive(Deserialize, Default)]
pub(super) struct ListQuery {
    #[serde(default)]
    pub(super) status: Option<String>,
    /// `human`, `agent`, or `both`. Anything else is treated as `both`.
    #[serde(default)]
    pub(super) author_kind: Option<String>,
    #[serde(default)]
    pub(super) task_id: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ReplyBody {
    pub(super) body: String,
}

pub(super) async fn list_review_threads(Ws(runtime): Ws, Query(q): Query<ListQuery>) -> Response {
    let status_filter = match parse_status_filter(q.status.as_deref()) {
        Ok(value) => value,
        Err(message) => return bad_request(message),
    };
    let author_kind = parse_author_kind(q.author_kind.as_deref());

    let tasks = match q.task_id.as_deref() {
        Some(raw) => {
            let id = match validate_id(raw) {
                Ok(id) => id,
                Err(message) => return bad_request(message),
            };
            match runtime.get_task(id) {
                Ok(task) => vec![task],
                Err(e) => return map_runtime_error(e),
            }
        }
        None => match runtime.list_tasks() {
            Ok(list) => list,
            Err(e) => return server_error(e),
        },
    };

    let mut rows: Vec<Value> = Vec::new();
    let mut open_count: usize = 0;
    let mut resolved_count: usize = 0;
    let mut human_count: usize = 0;
    let mut agent_count: usize = 0;
    for task in &tasks {
        if !is_workable_task_status(task.status) {
            continue;
        }
        let threads = match runtime.get_task_review_threads(&task.id) {
            Ok(t) => t,
            Err(e) => return server_error(e),
        };
        for thread in threads {
            let projection = project_thread(&task.id, task.title.as_str(), task.status, &thread);
            match thread.status {
                ReviewThreadStatus::Open => open_count += 1,
                ReviewThreadStatus::Resolved => resolved_count += 1,
            }
            if projection.has_agent_message {
                agent_count += 1;
            }
            if projection.has_human_message {
                human_count += 1;
            }
            if status_filter.is_some_and(|want| thread.status != want) {
                continue;
            }
            if !matches_author_kind(&projection, author_kind) {
                continue;
            }
            rows.push(projection.value);
        }
    }

    rows.sort_by(|a, b| {
        let a_ts = a
            .get("last_activity_at")
            .and_then(Value::as_str)
            .unwrap_or("");
        let b_ts = b
            .get("last_activity_at")
            .and_then(Value::as_str)
            .unwrap_or("");
        b_ts.cmp(a_ts)
    });

    Json(json!({
        "items": rows,
        "stats": {
            "open": open_count,
            "resolved": resolved_count,
            "total": open_count + resolved_count,
            "agent_authored": agent_count,
            "human_authored": human_count,
        },
    }))
    .into_response()
}

pub(super) async fn reply_review_thread_action(
    Ws(runtime): Ws,
    Path((task_id, thread_id)): Path<(String, String)>,
    Json(body): Json<ReplyBody>,
) -> Response {
    let validated_id = match validate_id(&task_id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    if thread_id.trim().is_empty() {
        return bad_request("thread_id must not be empty".to_string());
    }
    let reply_body = body.body.trim().to_string();
    if reply_body.is_empty() {
        return bad_request("reply body must not be empty".to_string());
    }
    let task_status = match runtime.get_task(validated_id) {
        Ok(t) => t.status,
        Err(e) => return map_runtime_error(e),
    };
    match runtime.reply_review_thread(validated_id, &thread_id, reply_body, None, None) {
        Ok(thread) => {
            Json(project_thread(validated_id, "", task_status, &thread).value).into_response()
        }
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn resolve_review_thread_action(
    Ws(runtime): Ws,
    Path((task_id, thread_id)): Path<(String, String)>,
) -> Response {
    let validated_id = match validate_id(&task_id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    if thread_id.trim().is_empty() {
        return bad_request("thread_id must not be empty".to_string());
    }
    let task_status = match runtime.get_task(validated_id) {
        Ok(t) => t.status,
        Err(e) => return map_runtime_error(e),
    };
    match runtime.resolve_review_thread(validated_id, &thread_id, None, None) {
        Ok(thread) => {
            Json(project_thread(validated_id, "", task_status, &thread).value).into_response()
        }
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn reopen_review_thread_action(
    Ws(runtime): Ws,
    Path((task_id, thread_id)): Path<(String, String)>,
) -> Response {
    let validated_id = match validate_id(&task_id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    if thread_id.trim().is_empty() {
        return bad_request("thread_id must not be empty".to_string());
    }
    let task_status = match runtime.get_task(validated_id) {
        Ok(t) => t.status,
        Err(e) => return map_runtime_error(e),
    };
    match runtime.reopen_review_thread(validated_id, &thread_id, None, None) {
        Ok(thread) => {
            Json(project_thread(validated_id, "", task_status, &thread).value).into_response()
        }
        Err(e) => map_runtime_error(e),
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum AuthorKindFilter {
    Both,
    Human,
    Agent,
}

// `pub(super)` widened so the sibling test module
// (`src/api/tests/review_threads.rs`) can exercise the helper. Used only
// by handlers in this file plus tests.
pub(super) fn parse_status_filter(raw: Option<&str>) -> Result<Option<ReviewThreadStatus>, String> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if raw.eq_ignore_ascii_case("all") {
        return Ok(None);
    }
    match raw.to_ascii_lowercase().as_str() {
        "open" => Ok(Some(ReviewThreadStatus::Open)),
        "resolved" => Ok(Some(ReviewThreadStatus::Resolved)),
        other => Err(format!(
            "status must be 'open', 'resolved', or 'all'; got '{other}'"
        )),
    }
}

fn parse_author_kind(raw: Option<&str>) -> AuthorKindFilter {
    match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("human") => AuthorKindFilter::Human,
        Some("agent") => AuthorKindFilter::Agent,
        _ => AuthorKindFilter::Both,
    }
}

/// Returns true for task statuses whose review threads are actionable in the
/// dashboard (the only ones surfaced by the cross-task list endpoint).
fn is_workable_task_status(status: TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Backlog | TaskStatus::InProgress | TaskStatus::Review
    )
}

struct ThreadProjection {
    value: Value,
    has_agent_message: bool,
    has_human_message: bool,
}

fn matches_author_kind(projection: &ThreadProjection, filter: AuthorKindFilter) -> bool {
    match filter {
        AuthorKindFilter::Both => true,
        AuthorKindFilter::Human => projection.has_human_message,
        AuthorKindFilter::Agent => projection.has_agent_message,
    }
}

fn project_thread(
    task_id: &str,
    task_title: &str,
    task_status: TaskStatus,
    thread: &ReviewThread,
) -> ThreadProjection {
    let messages_json: Vec<Value> = thread
        .messages
        .iter()
        .map(|message| {
            let kind = classify_author(&message.by);
            json!({
                "message_id": message.message_id,
                "at": message.at.to_rfc3339(),
                "by": message.by,
                "body": message.body,
                "author_kind": kind.tag(),
                "agent_family": kind.agent_family(),
            })
        })
        .collect();

    let has_agent_message = thread
        .messages
        .iter()
        .any(|m| matches!(classify_author(&m.by), AuthorKind::Agent { .. }));
    let has_human_message = thread
        .messages
        .iter()
        .any(|m| matches!(classify_author(&m.by), AuthorKind::Human));

    let last_message: Option<&ReviewMessage> = thread.messages.last();
    let first_message: Option<&ReviewMessage> = thread.messages.first();
    let last_activity_at: Option<DateTime<Utc>> = last_message.map(|m| m.at);
    let last_kind = last_message
        .map(|m| classify_author(&m.by))
        .unwrap_or(AuthorKind::Human);
    let anchor = if thread.path.is_some() && thread.line.is_some() {
        json!({
            "kind": "inline",
            "path": thread.path,
            "line": thread.line,
        })
    } else {
        json!({ "kind": "task_level" })
    };
    let body_preview = first_message
        .map(|m| preview_body(&m.body))
        .unwrap_or_default();

    let value = json!({
        "task_id": task_id,
        "task_title": task_title,
        "task_status": task_status.cli_name(),
        "thread_id": thread.thread_id,
        "status": thread.status.to_string(),
        "path": thread.path,
        "line": thread.line,
        "anchor": anchor,
        "body_preview": body_preview,
        "message_count": thread.messages.len(),
        "last_activity_at": last_activity_at.map(|ts| ts.to_rfc3339()),
        "last_author_kind": last_kind.tag(),
        "last_author_family": last_kind.agent_family(),
        "messages": messages_json,
    });

    ThreadProjection {
        value,
        has_agent_message,
        has_human_message,
    }
}

// `pub(super)` widened for sibling test access.
pub(super) fn preview_body(body: &str) -> String {
    const PREVIEW_LIMIT: usize = 160;
    let collapsed: String = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() <= PREVIEW_LIMIT {
        collapsed
    } else {
        let truncated: String = collapsed.chars().take(PREVIEW_LIMIT).collect();
        format!("{truncated}\u{2026}")
    }
}

// `pub(super)` widened for sibling test access.
#[derive(Debug, Clone)]
pub(super) enum AuthorKind {
    Human,
    Agent { family: Option<String> },
}

impl AuthorKind {
    fn tag(&self) -> &'static str {
        match self {
            AuthorKind::Human => "human",
            AuthorKind::Agent { .. } => "agent",
        }
    }

    fn agent_family(&self) -> Option<String> {
        match self {
            AuthorKind::Agent { family } => family.clone(),
            AuthorKind::Human => None,
        }
    }
}

// `pub(super)` widened for sibling test access.
pub(super) fn classify_author(label: &str) -> AuthorKind {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return AuthorKind::Human;
    }
    let lower = trimmed.to_ascii_lowercase();
    for family in all_agent_families() {
        if lower == family {
            return AuthorKind::Agent {
                family: Some(family.to_string()),
            };
        }
    }
    if let Some(family) = orbit_common::types::infer_agent_family_from_model(trimmed) {
        return AuthorKind::Agent {
            family: Some(family),
        };
    }
    AuthorKind::Human
}
