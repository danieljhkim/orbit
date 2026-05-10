//! Task CRUD and lifecycle handlers.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Json, Response};
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
use orbit_core::{
    ExternalRef, OrbitRuntime, Task, TaskComplexity, TaskPriority, TaskStatus, TaskType,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{bad_request, map_runtime_error, server_error, validate_id};
use crate::command::task::output::task_to_json;

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
}

fn default_priority() -> TaskPriority {
    TaskPriority::Medium
}

/// Partial-update body for `PATCH /tasks/:id`. Each field is `Option<...>`;
/// fields absent from the JSON body remain unchanged.
///
/// Note: `pr_status` and `batch_id` are intentionally omitted from this v1
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
    context_files: Option<Vec<String>>,
}

pub(super) async fn list_tasks(State(runtime): State<Arc<OrbitRuntime>>) -> Response {
    match list_dashboard_tasks(&runtime) {
        Ok(tasks) => match dashboard_status_index(&runtime) {
            Ok(status_by_id) => {
                let values: Vec<Value> = tasks
                    .iter()
                    .map(|task| task_to_json(task, &status_by_id))
                    .collect();
                Json(Value::Array(values)).into_response()
            }
            Err(e) => server_error(e),
        },
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
            Ok(status_by_id) => Json(task_to_json(&task, &status_by_id)).into_response(),
            Err(e) => server_error(e),
        },
        Err(e) => map_runtime_error(e),
    }
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
    };
    match runtime.add_task_with_identity(params, None, None) {
        Ok(task) => match dashboard_status_index(&runtime) {
            Ok(status_by_id) => Json(task_to_json(&task, &status_by_id)).into_response(),
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
        tags: body.tags,
        plan: body.plan,
        execution_summary: body.execution_summary,
        comment: body.comment,
        status: body.status,
        planned_by: None,
        implemented_by: None,
        pr_status: None,
        batch_id: None,
        context_files: body.context_files,
        upsert_artifacts: Vec::new(),
        append_review_threads: Vec::new(),
    };
    match runtime.update_task_with_identity(id, params, None, None) {
        Ok(task) => match dashboard_status_index(&runtime) {
            Ok(status_by_id) => Json(task_to_json(&task, &status_by_id)).into_response(),
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
            Ok(status_by_id) => Json(task_to_json(&task, &status_by_id)).into_response(),
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
            Ok(status_by_id) => Json(task_to_json(&task, &status_by_id)).into_response(),
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
