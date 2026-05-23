//! Task CRUD and lifecycle handlers.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderName, HeaderValue, header};
use axum::response::{IntoResponse, Json, Response};
use orbit_common::types::validate_relative_artifact_path;
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
use orbit_core::{
    ExternalRef, OrbitRuntime, Task, TaskComplexity, TaskPriority, TaskStatus, TaskType,
};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};

use super::{bad_request, map_runtime_error, server_error, validate_id};
use crate::projections::{task_locks_json, task_to_json_with_sidecars};

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
    tags: Vec<String>,
    #[serde(default)]
    plan: String,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    context_files: Vec<String>,
    #[serde(default)]
    external_refs: Vec<ExternalRef>,
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
    #[serde(default)]
    crew: Option<String>,
}

fn default_priority() -> TaskPriority {
    TaskPriority::Medium
}

/// Partial-update body for `PATCH /tasks/:id`. Each field is `Option<...>`;
/// fields absent from the JSON body remain unchanged.
///
/// Note: `pr_status` and `job_run_id` are intentionally omitted from this v1
/// surface. They use `Option<Option<String>>` in `TaskUpdateParams` to
/// distinguish absent vs. clear; the dashboard does not currently need to set
/// them. Add them via a `deserialize_with` adapter when a UI use case appears.
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
    task_type: Option<TaskType>,
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

pub(super) async fn list_tasks(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    match list_dashboard_tasks(&runtime) {
        Ok(tasks) => match dashboard_status_index(&runtime) {
            Ok(status_by_id) => {
                let values = match tasks
                    .iter()
                    .map(|task| task_to_json_with_sidecars(&runtime, task, &status_by_id))
                    .collect::<Result<Vec<_>, _>>()
                {
                    Ok(values) => values,
                    Err(error) => return server_error(error),
                };
                Json(Value::Array(values)).into_response()
            }
            Err(e) => server_error(e),
        },
        Err(e) => server_error(e),
    }
}

pub(super) async fn list_task_locks(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    match task_locks_json(&runtime) {
        Ok(value) => Json(value).into_response(),
        Err(e) => server_error(e),
    }
}

fn list_dashboard_tasks(runtime: &OrbitRuntime) -> Result<Vec<Task>, orbit_core::OrbitError> {
    let mut tasks = Vec::new();
    for status in DASHBOARD_TASK_STATUSES {
        tasks.extend(runtime.list_tasks_filtered(Some(*status), None, None, None, None, None)?);
    }
    Ok(tasks)
}

fn dashboard_status_index(
    runtime: &OrbitRuntime,
) -> Result<std::collections::BTreeMap<String, TaskStatus>, orbit_core::OrbitError> {
    Ok(orbit_core::build_task_status_index(&runtime.list_tasks()?))
}

pub(super) async fn get_task(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
) -> Response {
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
    State(runtime): State<Arc<OrbitRuntime>>,
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

pub(super) async fn create_task_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Json(body): Json<CreateTaskBody>,
) -> Response {
    let params = TaskAddParams {
        parent_id: body.parent_id,
        title: body.title,
        description: body.description,
        acceptance_criteria: body.acceptance_criteria,
        dependencies: body.dependencies,
        relations: Vec::new(),
        tags: body.tags,
        plan: body.plan,
        comment: body.comment,
        context_files: body.context_files,
        workspace_path: body.workspace_path,
        priority: body.priority,
        complexity: body.complexity,
        task_type: body.task_type,
        status: body.status,
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
    State(runtime): State<Arc<OrbitRuntime>>,
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
        relations: None,
        tags: body.tags,
        plan: body.plan,
        execution_summary: body.execution_summary,
        comment: body.comment,
        status: body.status,
        task_type: body.task_type,
        source_task_id: None,
        planned_by: None,
        implemented_by: None,
        pr_status: None,
        job_run_id: None,
        crew: body.crew,
        context_files: body.context_files,
        upsert_artifacts: Vec::new(),
        append_review_threads: Vec::new(),
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
    State(runtime): State<Arc<OrbitRuntime>>,
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
    State(runtime): State<Arc<OrbitRuntime>>,
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

pub(super) async fn archive_task_action(
    State(runtime): State<Arc<OrbitRuntime>>,
    Path(id): Path<String>,
) -> Response {
    let id = match validate_id(&id) {
        Ok(id) => id,
        Err(message) => return bad_request(message),
    };
    match runtime.archive_task(id) {
        Ok(()) => Json(json!({ "ok": true, "id": id })).into_response(),
        Err(e) => map_runtime_error(e),
    }
}
