use std::collections::BTreeSet;

use orbit_common::types::{
    OrbitError, TaskPriority, build_task_status_index, optional_csv_or_string_list_alias,
    optional_raw_string, optional_string, optional_string_alias, optional_string_list_alias,
    required_string, task_dependencies_ready,
};
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::command::task::{TaskAddParams, TaskUpdateParams};

use super::input::{
    empty_string_to_none, optional_bool_alias, parse_artifacts, parse_external_refs,
    parse_task_complexity, parse_task_priority, parse_task_status, parse_task_type,
};
use super::json::{serialize_task, serialize_task_lint_report, task_fields_to_json, task_to_json};

pub(super) fn add(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let title = required_string(&input, &["title"], "title")?;
    let description = required_string(&input, &["description"], "description")?;
    let workspace = required_string(&input, &["workspace"], "workspace")?;
    let plan = match input.get("plan") {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Null) | None => String::new(),
        Some(_) => {
            return Err(OrbitError::InvalidInput(
                "`plan` must be a string".to_string(),
            ));
        }
    };
    let task = runtime.add_task_with_identity(
        TaskAddParams {
            parent_id: optional_string_alias(&input, &["parent_id", "parent", "parentId"])?,
            title,
            description,
            acceptance_criteria: optional_string_list_alias(
                &input,
                &[
                    "acceptance_criteria",
                    "acceptanceCriteria",
                    "acceptance-criteria",
                ],
            )?
            .unwrap_or_default(),
            dependencies: optional_csv_or_string_list_alias(&input, &["dependencies"])?
                .unwrap_or_default(),
            plan,
            comment: optional_string(&input, "comment")?,
            context_files: optional_csv_or_string_list_alias(
                &input,
                &["context_files", "context"],
            )?
            .unwrap_or_default(),
            workspace_path: Some(workspace),
            priority: optional_string(&input, "priority")?
                .map(|value| parse_task_priority("priority", &value))
                .transpose()?
                .unwrap_or(TaskPriority::Medium),
            complexity: optional_string(&input, "complexity")?
                .map(|value| parse_task_complexity("complexity", &value))
                .transpose()?,
            task_type: optional_string_alias(&input, &["type", "task_type", "taskType"])?
                .map(|value| parse_task_type("type", &value))
                .transpose()?,
            status: optional_string(&input, "status")?
                .map(|value| parse_task_status("status", &value))
                .transpose()?,
            system_created: false,
            external_refs: parse_external_refs(&input)?,
            source_task_id: optional_string_alias(
                &input,
                &["source_task_id", "source_task", "sourceTaskId"],
            )?,
        },
        agent,
        model,
    )?;
    serialize_task(runtime, &task)
}

pub(super) fn approve(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    let task = runtime.approve_task_with_identity(
        &id,
        optional_string(&input, "note")?,
        optional_string(&input, "comment")?,
        agent,
        model,
    )?;
    serialize_task(runtime, &task)
}

pub(super) fn delete(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    runtime.delete_task(&id)?;
    Ok(json!({ "id": id, "deleted": true }))
}

pub(super) fn lint(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    serialize_task_lint_report(&runtime.lint_task(&id)?)
}

pub(super) fn list(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let status = optional_string(&input, "status")?
        .map(|value| parse_task_status("status", &value))
        .transpose()?;
    let task_type = optional_string_alias(&input, &["type", "task_type", "taskType"])?
        .map(|value| parse_task_type("type", &value))
        .transpose()?;
    let parent_id = optional_string_alias(&input, &["parent_id", "parent", "parentId"])?;
    let batch_id = optional_string(&input, "batch_id")?;
    let ready = optional_bool_alias(&input, &["ready"])?;
    let all_tasks = runtime.list_tasks()?;
    let status_by_id = build_task_status_index(&all_tasks);
    let tasks = all_tasks
        .into_iter()
        .filter(|task| status.is_none_or(|value| task.status == value))
        .filter(|task| {
            parent_id
                .as_deref()
                .is_none_or(|value| task.parent_id.as_deref() == Some(value))
        })
        .filter(|task| {
            batch_id
                .as_deref()
                .is_none_or(|value| task.batch_id.as_deref() == Some(value))
        })
        .filter(|task| ready != Some(true) || task_dependencies_ready(task, &status_by_id))
        .collect::<Vec<_>>();
    Ok(Value::Array(
        tasks
            .into_iter()
            .filter(|task| task_type.is_none_or(|kind| task.task_type == kind))
            .map(|task| task_to_json(&task, &status_by_id))
            .collect::<Vec<_>>(),
    ))
}

pub(super) fn search(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let query = required_string(&input, &["query"], "query")?;
    let status_by_id = build_task_status_index(&runtime.list_tasks()?);
    let tasks = runtime.search_tasks(&query)?;
    Ok(Value::Array(
        tasks
            .into_iter()
            .map(|task| task_to_json(&task, &status_by_id))
            .collect::<Vec<_>>(),
    ))
}

pub(super) fn reject(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    let note = required_string(&input, &["note"], "note")?;
    let task = runtime.reject_task_with_identity(
        &id,
        note,
        optional_string(&input, "comment")?,
        agent,
        model,
    )?;
    serialize_task(runtime, &task)
}

pub(super) fn show(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    let task = runtime.get_task(&id)?;
    let fields = optional_csv_or_string_list_alias(&input, &["fields", "field"])?;
    if let Some(fields) = fields {
        task_fields_to_json(runtime, &task, &fields)
    } else {
        serialize_task(runtime, &task)
    }
}

pub(super) fn start(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    let task = runtime.start_task_with_identity(
        &id,
        optional_string(&input, "note")?,
        optional_string(&input, "comment")?,
        agent,
        model,
    )?;
    serialize_task(runtime, &task)
}

pub(super) fn update(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    let task = runtime.update_task_with_identity(
        &id,
        TaskUpdateParams {
            title: optional_string(&input, "title")?,
            description: input
                .get("description")
                .map(|value| {
                    value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                        OrbitError::InvalidInput("`description` must be a string".to_string())
                    })
                })
                .transpose()?,
            acceptance_criteria: optional_string_list_alias(
                &input,
                &[
                    "acceptance_criteria",
                    "acceptanceCriteria",
                    "acceptance-criteria",
                ],
            )?,
            dependencies: optional_csv_or_string_list_alias(&input, &["dependencies"])?,
            plan: input
                .get("plan")
                .map(|value| {
                    value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                        OrbitError::InvalidInput("`plan` must be a string".to_string())
                    })
                })
                .transpose()?,
            execution_summary: optional_raw_string(&input, "execution_summary")?,
            comment: optional_string(&input, "comment")?,
            status: optional_string(&input, "status")?
                .map(|value| parse_task_status("status", &value))
                .transpose()?,
            planned_by: optional_raw_string(&input, "planned_by")?.map(empty_string_to_none),
            implemented_by: optional_raw_string(&input, "implemented_by")?
                .map(empty_string_to_none),
            pr_status: optional_raw_string(&input, "pr_status")?.map(empty_string_to_none),
            batch_id: optional_raw_string(&input, "batch_id")?.map(empty_string_to_none),
            context_files: optional_csv_or_string_list_alias(
                &input,
                &["context_files", "context"],
            )?,
            upsert_artifacts: parse_artifacts(&input)?,
            ..Default::default()
        },
        agent,
        model,
    )?;
    serialize_task(runtime, &task)
}

pub(crate) fn parse_task_ids(input: &Value) -> Result<Vec<String>, OrbitError> {
    let task_ids = optional_string_list_alias(input, &["task_ids", "taskIds", "task-ids"])?
        .ok_or_else(|| OrbitError::InvalidInput("missing `task_ids`".to_string()))?;
    parse_task_id_list(task_ids)
}

fn parse_task_id_list(task_ids: Vec<String>) -> Result<Vec<String>, OrbitError> {
    let deduped = task_ids.into_iter().collect::<BTreeSet<_>>();
    if deduped.is_empty() {
        return Err(OrbitError::InvalidInput(
            "`task_ids` must contain at least one task ID".to_string(),
        ));
    }
    Ok(deduped.into_iter().collect())
}
