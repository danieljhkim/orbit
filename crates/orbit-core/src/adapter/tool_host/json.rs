use std::collections::BTreeMap;

use orbit_common::OrbitError;
use orbit_types::task::{
    Task, TaskArtifact, TaskComment, TaskHistoryEntry, TaskStatus, resolve_task_dependencies,
    resolve_task_relations, task_show_record_field_json, unknown_task_show_field_message,
};
use serde_json::{Map, Value, json};

use crate::OrbitRuntime;
use crate::TaskCrewRead;
use crate::application::task::TaskLintReport;

pub(super) fn task_to_json(task: &Task, status_by_id: &BTreeMap<String, TaskStatus>) -> Value {
    json!({
        "id": task.id,
        "parent_id": task.parent_id(),
        "title": task.title,
        "description": task.description,
        "acceptance_criteria": task.acceptance_criteria,
        "dependencies": task.dependencies(),
        "resolved_dependencies": resolve_task_dependencies(task, status_by_id)
            .into_iter()
            .map(|dependency| dependency.label())
            .collect::<Vec<_>>(),
        "tags": task.tags,
        "plan": task.plan,
        "execution_summary": task.execution_summary,
        "context_files": task.context_files,
        "created_by": task.created_by,
        "planned_by": task.planned_by,
        "implemented_by": task.implemented_by,
        "status": task.status.to_string(),
        "priority": task.priority.to_string(),
        "complexity": task.complexity.map(|value| value.to_string()),
        "type": task.task_type.to_string(),
        "pr_status": task.pr_status,
        "external_refs": task.external_refs,
        "relations": resolve_task_relations(task, status_by_id),
        "source_task_id": task.source_task_id(),
        "job_run_id": task.job_run_id,
        "crew": task.crew,
        "orchestrator": task.orchestrator,
        "created_at": task.created_at.to_rfc3339(),
        "updated_at": task.updated_at.to_rfc3339(),
    })
}

pub(super) fn serialize_task(runtime: &OrbitRuntime, task: &Task) -> Result<Value, OrbitError> {
    let status_by_id = runtime.task_status_index()?;
    let mut value = task_to_json(task, &status_by_id);
    let object = value.as_object_mut().ok_or_else(|| {
        OrbitError::Execution("task JSON projection did not produce an object".to_string())
    })?;
    object.insert(
        "comments".to_string(),
        serialize_comments(&runtime.get_task_comments(&task.id)?)?,
    );
    object.insert(
        "history".to_string(),
        serialize_history(&runtime.get_task_history(&task.id)?)?,
    );
    insert_resolved_crew(runtime, task, object);
    Ok(value)
}

/// Enrich a task projection with its resolved crew, when this host can resolve
/// one.
///
/// `crew` (the stored value) is part of the record; `resolved_crew` /
/// `crew_model` only annotate it. A host whose `[crews.*]` table has no entry
/// for the stored crew — a task authored elsewhere, or a config edited since —
/// still owes the caller the task, so an unresolvable crew is reported as
/// `crew_unresolved` rather than failing the whole readout (ORB-10968). The
/// tolerance lives in `OrbitRuntime::task_crew_read`, shared with the CLI.
fn insert_resolved_crew(runtime: &OrbitRuntime, task: &Task, object: &mut Map<String, Value>) {
    match runtime.task_crew_read(task) {
        TaskCrewRead::Absent => {}
        TaskCrewRead::Resolved(projection) => {
            object.insert("resolved_crew".to_string(), Value::String(projection.name));
            object.insert("crew_model".to_string(), Value::String(projection.model));
        }
        TaskCrewRead::Unresolved { reason } => {
            object.insert("crew_unresolved".to_string(), Value::String(reason));
        }
    }
}

pub(super) fn serialize_task_lint_report(report: &TaskLintReport) -> Result<Value, OrbitError> {
    serde_json::to_value(report).map_err(serialize_error("serialize task lint report"))
}

pub(super) fn task_fields_to_json(
    runtime: &OrbitRuntime,
    task: &Task,
    fields: &[String],
) -> Result<Value, OrbitError> {
    let status_by_id = if fields.iter().any(|field| field == "resolved_dependencies") {
        Some(runtime.task_status_index()?)
    } else {
        None
    };

    if fields.len() == 1 {
        return task_field_to_json(runtime, task, &fields[0], status_by_id.as_ref());
    }

    let mut object = Map::new();
    for field in fields {
        object.insert(
            field.clone(),
            task_field_to_json(runtime, task, field, status_by_id.as_ref())?,
        );
    }
    Ok(Value::Object(object))
}

fn task_field_to_json(
    runtime: &OrbitRuntime,
    task: &Task,
    field: &str,
    status_by_id: Option<&BTreeMap<String, TaskStatus>>,
) -> Result<Value, OrbitError> {
    match field {
        "comments" => serialize_comments(&runtime.get_task_comments(&task.id)?),
        "plan" => Ok(Value::String(task.plan.clone())),
        "execution_summary" => Ok(Value::String(task.execution_summary.clone())),
        "description" => Ok(Value::String(task.description.clone())),
        "acceptance_criteria" => serde_json::to_value(&task.acceptance_criteria)
            .map_err(serialize_error("serialize acceptance criteria")),
        "dependencies" => serde_json::to_value(task.dependencies())
            .map_err(serialize_error("serialize dependencies")),
        "tags" => serde_json::to_value(&task.tags).map_err(serialize_error("serialize tags")),
        "resolved_dependencies" => serde_json::to_value(
            resolve_task_dependencies(
                task,
                status_by_id.ok_or_else(|| {
                    OrbitError::Execution(
                        "missing dependency status index for resolved_dependencies".to_string(),
                    )
                })?,
            )
            .into_iter()
            .map(|dependency| dependency.label())
            .collect::<Vec<_>>(),
        )
        .map_err(serialize_error("serialize resolved dependencies")),
        "history" => serialize_history(&runtime.get_task_history(&task.id)?),
        "context_files" => serde_json::to_value(&task.context_files)
            .map_err(serialize_error("serialize context files")),
        "crew" => serde_json::to_value(&task.crew).map_err(serialize_error("serialize crew")),
        "orchestrator" => serde_json::to_value(&task.orchestrator)
            .map_err(serialize_error("serialize orchestrator")),
        "artifacts" => Ok(serialize_task_artifacts(
            &runtime.get_task_artifacts(&task.id)?,
        )),
        other => task_show_record_field_json(task, other)
            .ok_or_else(|| OrbitError::InvalidInput(unknown_task_show_field_message(other))),
    }
}

pub(super) fn serialize_task_artifacts(artifacts: &[TaskArtifact]) -> Value {
    Value::Array(
        artifacts
            .iter()
            .map(|artifact| {
                let mut object = Map::new();
                object.insert("path".to_string(), Value::String(artifact.path.clone()));
                object.insert(
                    "media_type".to_string(),
                    Value::String(artifact.media_type.clone()),
                );
                if let Some(created_by) = &artifact.created_by {
                    object.insert("created_by".to_string(), Value::String(created_by.clone()));
                }
                object.insert(
                    "size".to_string(),
                    Value::Number(serde_json::Number::from(artifact.content.len())),
                );
                if let Some(content) = artifact.text_content() {
                    object.insert("content".to_string(), Value::String(content.to_string()));
                }
                Value::Object(object)
            })
            .collect(),
    )
}

fn serialize_comments(comments: &[TaskComment]) -> Result<Value, OrbitError> {
    serde_json::to_value(comments).map_err(serialize_error("serialize comments"))
}

fn serialize_history(history: &[TaskHistoryEntry]) -> Result<Value, OrbitError> {
    serde_json::to_value(history).map_err(serialize_error("serialize history"))
}

pub(super) fn serialize_error(label: &'static str) -> impl FnOnce(serde_json::Error) -> OrbitError {
    move |error| OrbitError::Execution(format!("{label}: {error}"))
}
