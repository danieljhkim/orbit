//! Task CRUD and lifecycle handlers.

use crate::state::Ws;
use axum::body::Body;
use axum::extract::{Path, RawQuery};
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::{IntoResponse, Json, Response};
use orbit_common::types::task_artifacts::TaskRelation;
use orbit_common::types::validate_relative_artifact_path;
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
use orbit_core::{
    DEFAULT_TASK_LIST_LIMIT, ExternalRef, OrbitRuntime, Task, TaskComplexity, TaskCreateStatus,
    TaskPriority, TaskStatus, TaskType,
};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

use super::{bad_request, map_runtime_error, server_error, validate_id};
use crate::projections::{task_locks_json, task_to_json_with_sidecars};

struct ArtifactResponsePolicy {
    content_type: &'static str,
    attachment: bool,
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
    relations: Vec<TaskRelation>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    plan: String,
    #[serde(default)]
    context_files: Vec<String>,
    #[serde(default)]
    external_refs: Vec<ExternalRef>,
    #[serde(default)]
    workspace_path: Option<String>,
    /// Trap field (ORB-00042): `workspace` is a *workspace selector* and this
    /// endpoint takes it as the `?workspace=<id>` query parameter, never as a
    /// body field. Historically bridge sent `{"workspace": <path>}` here and
    /// serde silently dropped the unknown key, so every task landed in the
    /// default workspace. Deserializing the key just to reject it makes the
    /// mistake a loud 400. A `#[serde(alias = "workspace")]` on
    /// `workspace_path` would be wrong semantics (a selector is not a sub-path
    /// hint), and `#[serde(deny_unknown_fields)]` would break every tolerant
    /// caller that sends extra keys.
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default = "default_priority")]
    priority: TaskPriority,
    #[serde(default)]
    complexity: Option<TaskComplexity>,
    #[serde(default)]
    task_type: Option<TaskType>,
    #[serde(default)]
    status: Option<TaskCreateStatus>,
    #[serde(default)]
    parent_id: Option<String>,
    #[serde(default)]
    source_task_id: Option<String>,
    #[serde(default)]
    crew: Option<String>,
}

fn default_priority() -> TaskPriority {
    TaskPriority::Medium
}

/// Partial-update body for `PATCH /tasks/:id`. Each field is `Option<...>`;
/// fields absent from the JSON body remain unchanged.
///
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
    relations: Option<Vec<TaskRelation>>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    plan: Option<String>,
    #[serde(default)]
    execution_summary: Option<String>,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    status: Option<TaskStatus>,
    #[serde(default)]
    complexity: Option<TaskComplexity>,
    #[serde(default)]
    task_type: Option<TaskType>,
    #[serde(default, deserialize_with = "deserialize_nullable_string_patch_field")]
    pr_status: Option<Option<String>>,
    #[serde(default)]
    context_files: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_nullable_string_patch_field")]
    crew: Option<Option<String>>,
}

fn deserialize_nullable_string_patch_field<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// `GET /api/tasks` — the request workspace's tasks, filtered server-side.
///
/// ## Query parameters
///
/// - `status` — repeatable and/or comma-separated (`?status=proposed,backlog`).
///   OR semantics across values; omitted means every lifecycle status.
/// - `tag` (alias `tags`) — repeatable and/or comma-separated. AND semantics:
///   a task must carry every requested tag. Values go through Orbit's own
///   [`normalize_task_tags`](orbit_common::types::normalize_task_tags) /
///   `task_matches_tags`, so a colon is ordinary tag content and
///   `auto-task:qa-sweep` matches that whole tag rather than being split.
/// - `type` (alias `task_type`) — a single task type (`feature`/`bug`/
///   `refactor`/`chore`).
/// - `limit` — positive integer, defaulting to [`DEFAULT_TASK_LIST_LIMIT`].
///
/// Unknown keys (including the `?workspace=<id>` selector consumed by the
/// [`Ws`] extractor) are ignored; an unparseable value is a 400 rather than a
/// silently-dropped filter.
///
/// ## Response contract (ORB-10400, consumed by bridge ORB-10398)
///
/// ```json
/// { "items": [ /* task objects, newest first */ ],
///   "total": 137, "limit": 50, "truncated": true }
/// ```
///
/// **Every predicate is applied before the limit**, so `items` holds the newest
/// *matching* tasks — a match older than the newest `limit` unfiltered tasks is
/// still reachable by passing its filters. `total` is the pre-limit match count
/// and `truncated` is `total > items.len()`, which is what lets a client tell a
/// genuinely empty result (`total: 0`) from a filter whose matches fell outside
/// the window (previously indistinguishable, because the handler answered a bare
/// truncated array with no metadata).
///
/// The cross-workspace `/api/tasks/all` aggregate still answers a bare array; it
/// takes no filters and is bounded by the same default limit.
pub(super) async fn list_tasks(Ws(runtime): Ws, RawQuery(query): RawQuery) -> Response {
    let query = match TaskListQuery::parse(query.as_deref()) {
        Ok(query) => query,
        Err(message) => return bad_request(message),
    };
    match task_list_page_json(&runtime, &query) {
        Ok(value) => Json(value).into_response(),
        Err(e) => server_error(e),
    }
}

/// Server-side filters for `GET /api/tasks`, mirroring `orbit task list`
/// semantics (ORB-10400).
struct TaskListQuery {
    /// Accepted statuses; empty means status-neutral (every lifecycle status).
    statuses: Vec<TaskStatus>,
    /// Required tags, AND-combined. Passed verbatim to `list_tasks_by_tags`,
    /// which normalizes them.
    tags: Vec<String>,
    task_type: Option<TaskType>,
    limit: usize,
}

impl Default for TaskListQuery {
    fn default() -> Self {
        Self {
            statuses: Vec::new(),
            tags: Vec::new(),
            task_type: None,
            limit: DEFAULT_TASK_LIST_LIMIT,
        }
    }
}

impl TaskListQuery {
    /// Parse the raw (still percent-encoded) query string.
    ///
    /// Repeated keys accumulate and each value may itself be comma-separated,
    /// matching the CLI's `--status a,b` / repeated `--tag`. Empty values are
    /// ignored so `?status=` behaves like an omitted filter, and unknown keys
    /// are skipped because this endpoint shares its query string with the `Ws`
    /// workspace selector.
    fn parse(raw_query: Option<&str>) -> Result<Self, String> {
        let mut parsed = Self::default();
        let Some(raw_query) = raw_query else {
            return Ok(parsed);
        };
        for (key, value) in url::form_urlencoded::parse(raw_query.as_bytes()) {
            match key.as_ref() {
                "status" => {
                    for raw in split_filter_values(&value) {
                        let status = raw
                            .to_ascii_lowercase()
                            .parse::<TaskStatus>()
                            .map_err(|error| format!("invalid `status` value `{raw}`: {error}"))?;
                        if !parsed.statuses.contains(&status) {
                            parsed.statuses.push(status);
                        }
                    }
                }
                // A tag is matched whole: only `,` separates values, so a
                // colon-bearing tag (`auto-task:qa-sweep`) stays intact.
                "tag" | "tags" => parsed
                    .tags
                    .extend(split_filter_values(&value).map(str::to_string)),
                "type" | "task_type" => {
                    for raw in split_filter_values(&value) {
                        parsed.task_type =
                            Some(raw.to_ascii_lowercase().parse::<TaskType>().map_err(
                                |error| format!("invalid `type` value `{raw}`: {error}"),
                            )?);
                    }
                }
                "limit" => {
                    if let Some(raw) = split_filter_values(&value).next_back() {
                        parsed.limit = parse_limit(raw)?;
                    }
                }
                _ => {}
            }
        }
        Ok(parsed)
    }
}

/// Split one query value into its comma-separated parts, trimming each and
/// dropping empties.
fn split_filter_values(value: &str) -> impl DoubleEndedIterator<Item = &str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
}

/// Parse `?limit=`, rejecting zero (which would return nothing) the same way the
/// CLI's `--limit` does.
fn parse_limit(raw: &str) -> Result<usize, String> {
    let value: usize = raw
        .parse()
        .map_err(|_| format!("invalid `limit` value `{raw}` (expected a positive integer)"))?;
    if value == 0 {
        return Err("`limit` must be at least 1".to_string());
    }
    Ok(value)
}

/// Build the `{ items, total, limit, truncated }` page for `GET /api/tasks`.
fn task_list_page_json(
    runtime: &OrbitRuntime,
    query: &TaskListQuery,
) -> Result<Value, orbit_core::OrbitError> {
    let (tasks, total) = filtered_dashboard_tasks(runtime, query)?;
    let items = task_values(runtime, &tasks)?;
    let truncated = total > items.len();
    Ok(json!({
        "items": items,
        "total": total,
        "limit": query.limit,
        "truncated": truncated,
    }))
}

/// Build the dashboard task list as JSON values for the cross-workspace
/// `/api/tasks/all` aggregate: unfiltered and bounded by the default limit,
/// exactly as `GET /api/tasks` was before it grew query filters.
pub(super) fn list_tasks_json(
    runtime: &OrbitRuntime,
) -> Result<Vec<Value>, orbit_core::OrbitError> {
    let (tasks, _total) = filtered_dashboard_tasks(runtime, &TaskListQuery::default())?;
    task_values(runtime, &tasks)
}

fn task_values(
    runtime: &OrbitRuntime,
    tasks: &[Task],
) -> Result<Vec<Value>, orbit_core::OrbitError> {
    let status_by_id = dashboard_status_index(runtime)?;
    tasks
        .iter()
        .map(|task| task_to_json_with_sidecars(runtime, task, &status_by_id))
        .collect()
}

pub(super) async fn list_task_locks(Ws(runtime): Ws) -> Response {
    match task_locks_json(&runtime) {
        Ok(value) => Json(value).into_response(),
        Err(e) => server_error(e),
    }
}

/// The dashboard task list: status-neutral by default, newest-first, and
/// bounded (ORB-10310). Returns the page plus the **pre-limit** match count.
///
/// `list_tasks_by_tags` orders by `created_at DESC` with task ID ascending for
/// ties and applies Orbit's own `normalize_task_tags`/`task_matches_tags`, so
/// tag matching here is identical to `orbit task list --tag` (a colon is
/// ordinary tag content). The remaining predicates preserve that order, and —
/// the point of ORB-10400 — every predicate runs *before* the truncation, so
/// the page holds the newest tasks matching the filter rather than whatever
/// survives a truncation of the unfiltered set. With no filters this is exactly
/// the previous behavior: the newest `DEFAULT_TASK_LIST_LIMIT` tasks of any
/// lifecycle status, `done`/`archived` included.
fn filtered_dashboard_tasks(
    runtime: &OrbitRuntime,
    query: &TaskListQuery,
) -> Result<(Vec<Task>, usize), orbit_core::OrbitError> {
    let mut matching = runtime
        .list_tasks_by_tags(&query.tags)?
        .into_iter()
        .filter(|task| query.statuses.is_empty() || query.statuses.contains(&task.status))
        .filter(|task| query.task_type.is_none_or(|kind| task.task_type == kind))
        .collect::<Vec<_>>();
    let total = matching.len();
    matching.truncate(query.limit);
    Ok((matching, total))
}

/// Dependency-status projection for dashboard task serialization.
///
/// Uses the coordination registry's global status index
/// ([`OrbitRuntime::task_status_index`]) rather than `runtime.list_tasks()`
/// (workspace-scoped), so a task depending on another registered workspace's
/// task resolves that dependency's real status instead of `[missing]`
/// (ORB-10291). Task *listing* stays workspace-scoped: this index is only
/// consulted to label dependencies, never to add tasks to the response body.
fn dashboard_status_index(
    runtime: &OrbitRuntime,
) -> Result<std::collections::BTreeMap<String, TaskStatus>, orbit_core::OrbitError> {
    runtime.task_status_index()
}

pub(super) async fn get_task(Ws(runtime): Ws, Path(id): Path<String>) -> Response {
    let id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    match runtime.get_task(id) {
        Ok(task) => match dashboard_status_index(&runtime) {
            Ok(status_by_id) => match task_to_json_with_sidecars(&runtime, &task, &status_by_id) {
                Ok(value) => Json(value).into_response(),
                Err(e) => server_error(e),
            },
            Err(e) => server_error(e),
        },
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn get_task_artifact(
    Ws(runtime): Ws,
    Path((id, path)): Path<(String, String)>,
) -> Response {
    let id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    let path = match validate_artifact_request_path(&path) {
        Ok(path) => path,
        Err(message) => return bad_request(message),
    };
    match runtime.get_task_artifact(id, &path) {
        Ok(Some(artifact)) => {
            let policy = artifact_response_policy(&artifact.media_type);
            let mut response = Response::new(Body::from(artifact.content));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(policy.content_type),
            );
            response.headers_mut().insert(
                HeaderName::from_static("x-content-type-options"),
                HeaderValue::from_static("nosniff"),
            );
            if policy.attachment {
                response.headers_mut().insert(
                    header::CONTENT_DISPOSITION,
                    HeaderValue::from_static("attachment"),
                );
            }
            response
        }
        Ok(None) => super::not_found(format!("artifact not found: {id}/{path}")),
        Err(e) => map_runtime_error(e),
    }
}

fn artifact_response_policy(media_type: &str) -> ArtifactResponsePolicy {
    let content_type = inline_safe_artifact_content_type(media_type);
    ArtifactResponsePolicy {
        content_type: content_type.unwrap_or("application/octet-stream"),
        attachment: content_type.is_none(),
    }
}

fn inline_safe_artifact_content_type(media_type: &str) -> Option<&'static str> {
    match normalized_media_type(media_type).as_deref() {
        Some("application/json") => Some("application/json"),
        Some("application/toml") => Some("application/toml"),
        Some("application/yaml") => Some("application/yaml"),
        Some("image/gif") => Some("image/gif"),
        Some("image/jpeg") => Some("image/jpeg"),
        Some("image/png") => Some("image/png"),
        Some("image/webp") => Some("image/webp"),
        Some("text/csv") => Some("text/csv"),
        Some("text/plain") => Some("text/plain"),
        _ => None,
    }
}

fn normalized_media_type(media_type: &str) -> Option<String> {
    let base = media_type
        .split_once(';')
        .map_or(media_type, |(base, _params)| base)
        .trim();
    if base.is_empty() {
        return None;
    }
    Some(base.to_ascii_lowercase())
}

fn validate_artifact_request_path(path: &str) -> Result<String, String> {
    validate_relative_artifact_path(path).map_err(|error| error.to_string())?;
    Ok(path.to_string())
}

/// `POST /tasks` — create a task in the request's workspace.
///
/// Workspace selection is the [`Ws`] extractor's: the `?workspace=<id>` query
/// parameter picks the target workspace; omitting it falls back to the
/// server's configured default workspace; an unknown id is a 404 and an
/// inactive (stale-path) one a 400 — never a silent fallback. The body's
/// `workspace_path` is *not* a selector: it is an optional sub-path hint
/// within the already-selected workspace. A stray `workspace` body key is
/// rejected with a 400 (see [`CreateTaskBody::workspace`], ORB-00042).
pub(super) async fn create_task_action(
    Ws(runtime): Ws,
    Json(body): Json<CreateTaskBody>,
) -> Response {
    if body.workspace.is_some() {
        return bad_request(
            "unsupported body field `workspace`: select the target workspace with the \
             `?workspace=<id>` query parameter (`workspace_path` is a sub-path hint \
             within the selected workspace, not a workspace selector)"
                .to_string(),
        );
    }
    let params = TaskAddParams {
        parent_id: body.parent_id,
        title: body.title,
        description: body.description,
        acceptance_criteria: body.acceptance_criteria,
        dependencies: body.dependencies,
        relations: body.relations,
        tags: body.tags,
        plan: body.plan,
        comment: None,
        context_files: body.context_files,
        workspace_path: body.workspace_path,
        priority: body.priority,
        complexity: body.complexity,
        task_type: body.task_type,
        status: body.status.map(Into::into),
        system_created: false,
        external_refs: body.external_refs,
        source_task_id: body.source_task_id,
        crew: body.crew,
    };
    match runtime.add_task_with_identity(params, None, None) {
        Ok(task) => match dashboard_status_index(&runtime) {
            Ok(status_by_id) => match task_to_json_with_sidecars(&runtime, &task, &status_by_id) {
                Ok(value) => Json(value).into_response(),
                Err(e) => server_error(e),
            },
            Err(e) => server_error(e),
        },
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn update_task_action(
    Ws(runtime): Ws,
    Path(id): Path<String>,
    Json(body): Json<UpdateTaskBody>,
) -> Response {
    let id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    let params = TaskUpdateParams {
        title: body.title,
        description: body.description,
        acceptance_criteria: body.acceptance_criteria,
        dependencies: body.dependencies,
        relations: body.relations,
        tags: body.tags,
        plan: body.plan,
        execution_summary: body.execution_summary,
        comment: body.comment,
        status: body.status,
        complexity: body.complexity,
        task_type: body.task_type,
        source_task_id: None,
        planned_by: None,
        implemented_by: None,
        pr_status: body.pr_status,
        job_run_id: None,
        crew: body.crew,
        context_files: body.context_files,
        upsert_artifacts: Vec::new(),
    };
    match runtime.update_task_with_identity(id, params, None, None) {
        Ok(task) => match dashboard_status_index(&runtime) {
            Ok(status_by_id) => match task_to_json_with_sidecars(&runtime, &task, &status_by_id) {
                Ok(value) => Json(value).into_response(),
                Err(e) => server_error(e),
            },
            Err(e) => server_error(e),
        },
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn approve_task_action(
    Ws(runtime): Ws,
    Path(id): Path<String>,
    body: Option<Json<ApproveBody>>,
) -> Response {
    let id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    let body = body.map(|Json(b)| b).unwrap_or_default();
    match runtime.approve_task(id, body.note, body.comment) {
        Ok(task) => match dashboard_status_index(&runtime) {
            Ok(status_by_id) => match task_to_json_with_sidecars(&runtime, &task, &status_by_id) {
                Ok(value) => Json(value).into_response(),
                Err(e) => server_error(e),
            },
            Err(e) => server_error(e),
        },
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn reject_task_action(
    Ws(runtime): Ws,
    Path(id): Path<String>,
    Json(body): Json<RejectBody>,
) -> Response {
    let id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    match runtime.reject_task(id, body.note, body.comment) {
        Ok(task) => match dashboard_status_index(&runtime) {
            Ok(status_by_id) => match task_to_json_with_sidecars(&runtime, &task, &status_by_id) {
                Ok(value) => Json(value).into_response(),
                Err(e) => server_error(e),
            },
            Err(e) => server_error(e),
        },
        Err(e) => map_runtime_error(e),
    }
}

pub(super) async fn archive_task_action(Ws(runtime): Ws, Path(id): Path<String>) -> Response {
    let id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    match runtime.archive_task(id) {
        Ok(()) => Json(json!({ "ok": true, "id": id })).into_response(),
        Err(e) => map_runtime_error(e),
    }
}
