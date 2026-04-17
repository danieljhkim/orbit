use orbit_types::{OrbitError, Task, TaskComment, TaskHistoryEntry, TaskStatus};
use serde_json::{Map, Value, json};

use crate::OrbitRuntime;
use crate::command::task::TaskLintReport;

pub(super) fn task_to_json(task: Task) -> Value {
    json!({
        "id": task.id,
        "parent_id": task.parent_id,
        "title": task.title,
        "description": task.description,
        "acceptance_criteria": task.acceptance_criteria,
        "plan": task.plan,
        "execution_summary": task.execution_summary,
        "context_files": task.context_files,
        "workspace_path": task.workspace_path,
        "repo_root": task.repo_root,
        "created_by": task.created_by,
        "planned_by": task.planned_by,
        "implemented_by": task.implemented_by,
        "status": task.status.to_string(),
        "priority": task.priority.to_string(),
        "complexity": task.complexity.map(|value| value.to_string()),
        "type": task.task_type.to_string(),
        "pr_number": task.pr_number,
        "pr_status": task.pr_status,
        "source_task_id": task.source_task_id,
        "comments": task.comments,
        "history": task.history,
        "review_threads": task.review_threads,
        "created_at": task.created_at.to_rfc3339(),
        "updated_at": task.updated_at.to_rfc3339(),
    })
}

pub(super) fn serialize_task(task: &Task) -> Result<Value, OrbitError> {
    Ok(task_to_json(task.clone()))
}

pub(super) fn task_lock_to_json(task: &Task) -> Value {
    json!({
        "id": task.id,
        "title": task.title,
        "status": task.status.to_string(),
        "batch_id": task.batch_id,
        "context_files": task.context_files,
    })
}

pub(super) fn serialize_task_lint_report(report: &TaskLintReport) -> Result<Value, OrbitError> {
    serde_json::to_value(report).map_err(serialize_error("serialize task lint report"))
}

pub(super) fn task_fields_to_json(
    runtime: &OrbitRuntime,
    task: &Task,
    fields: &[String],
) -> Result<Value, OrbitError> {
    if fields.len() == 1 {
        return task_field_to_json(runtime, task, &fields[0]);
    }

    let mut object = Map::new();
    for field in fields {
        object.insert(field.clone(), task_field_to_json(runtime, task, field)?);
    }
    Ok(Value::Object(object))
}

fn task_field_to_json(
    runtime: &OrbitRuntime,
    task: &Task,
    field: &str,
) -> Result<Value, OrbitError> {
    match field {
        "comments" => serialize_comments(&task.comments),
        "plan" => Ok(Value::String(task.plan.clone())),
        "execution_summary" => Ok(Value::String(task.execution_summary.clone())),
        "description" => Ok(Value::String(task.description.clone())),
        "acceptance_criteria" => serde_json::to_value(&task.acceptance_criteria)
            .map_err(serialize_error("serialize acceptance criteria")),
        "history" => serialize_history(&task.history),
        "context_files" => serde_json::to_value(&task.context_files)
            .map_err(serialize_error("serialize context files")),
        "artifacts" => serde_json::to_value(runtime.get_task_artifacts(&task.id)?)
            .map_err(serialize_error("serialize task artifacts")),
        other => Err(OrbitError::InvalidInput(format!(
            "unknown field selector `{other}`. Valid values: comments, plan, execution_summary, description, acceptance_criteria, history, context_files, artifacts"
        ))),
    }
}

fn serialize_comments(comments: &[TaskComment]) -> Result<Value, OrbitError> {
    serde_json::to_value(comments).map_err(serialize_error("serialize comments"))
}

fn serialize_history(history: &[TaskHistoryEntry]) -> Result<Value, OrbitError> {
    serde_json::to_value(history).map_err(serialize_error("serialize history"))
}

pub(super) fn task_lock_status_rank(status: TaskStatus) -> u8 {
    match status {
        TaskStatus::InProgress => 0,
        TaskStatus::Review => 1,
        _ => 2,
    }
}

pub(super) fn serialize_error(label: &'static str) -> impl FnOnce(serde_json::Error) -> OrbitError {
    move |error| OrbitError::Execution(format!("{label}: {error}"))
}
