use std::collections::BTreeSet;

use orbit_common::types::{
    OrbitError, TaskPriority, build_task_status_index, optional_csv_or_string_list_alias,
    optional_raw_string, optional_string, optional_string_alias, optional_string_list_alias,
    required_string, strip_retired_task_add_input_fields, task_dependencies_ready,
};
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::command::task::{TaskAddParams, TaskUpdateParams, compute_task_add_warnings};

use super::input::{
    empty_string_to_none, optional_bool_alias, parse_artifacts, parse_relations,
    parse_task_complexity, parse_task_priority, parse_task_status, parse_task_type,
};
use super::json::{serialize_task, serialize_task_lint_report, task_fields_to_json, task_to_json};

pub(super) fn add(
    runtime: &OrbitRuntime,
    mut input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let ignored_fields = strip_retired_task_add_input_fields(&mut input);
    if !ignored_fields.is_empty() {
        tracing::warn!(
            target: "orbit.core.task.add",
            ignored_fields = ?ignored_fields,
            "ignored retired orbit.task.add fields"
        );
    }

    let title = required_string(&input, &["title"], "title")?;
    let description = required_string(&input, &["description"], "description")?;
    let workspace = required_string(&input, &["workspace"], "workspace")?;
    let raw_context_files =
        optional_csv_or_string_list_alias(&input, &["context_files"])?.unwrap_or_default();
    let task = runtime.add_task_with_identity(
        TaskAddParams {
            parent_id: None,
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
            dependencies: Vec::new(),
            relations: parse_relations(&input)?.unwrap_or_default(),
            tags: optional_csv_or_string_list_alias(&input, &["tags", "tag"])?.unwrap_or_default(),
            plan: String::new(),
            comment: None,
            context_files: raw_context_files.clone(),
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
            status: None,
            system_created: false,
            external_refs: Vec::new(),
            source_task_id: None,
            crew: None,
        },
        agent,
        model,
    )?;
    let mut response = serialize_task(runtime, &task)?;
    let warnings = compute_task_add_warnings(&raw_context_files, task.task_type);
    if !warnings.is_empty()
        && let Some(obj) = response.as_object_mut()
    {
        obj.insert("warnings".to_string(), json!(warnings));
    }
    Ok(response)
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
    let force = optional_bool_alias(&input, &["force"])?.unwrap_or(false);
    runtime.delete_task_guarded(&id, force)?;
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
    let job_run_id = optional_string(&input, "job_run_id")?;
    let tags = optional_csv_or_string_list_alias(&input, &["tags", "tag"])?.unwrap_or_default();
    let ready = optional_bool_alias(&input, &["ready"])?;
    let path = optional_string(&input, "path")?;
    let all_tasks = runtime.list_tasks_filtered(
        status,
        None,
        parent_id.as_deref(),
        job_run_id.as_deref(),
        None,
        None,
    )?;
    let status_by_id = build_task_status_index(&runtime.list_tasks()?);
    let tasks = all_tasks
        .into_iter()
        .filter(|task| orbit_common::types::task_matches_tags(task, &tags))
        .filter(|task| ready != Some(true) || task_dependencies_ready(task, &status_by_id))
        .filter(|task| {
            path.as_deref()
                .is_none_or(|p| crate::task_selectors_contain_path(&task.context_files, p))
        })
        .collect::<Vec<_>>();
    Ok(Value::Array(
        tasks
            .into_iter()
            .filter(|task| task_type.is_none_or(|kind| task.task_type == kind))
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
    let with_context =
        optional_bool_alias(&input, &["with_context", "withContext", "with-context"])?
            .unwrap_or(false);
    let max_docs = optional_usize_alias(&input, &["max_docs", "maxDocs", "max-docs"])?;
    if let Some(fields) = fields {
        if with_context {
            return Err(OrbitError::InvalidInput(
                "`with_context` cannot be combined with `fields`".to_string(),
            ));
        }
        task_fields_to_json(runtime, &task, &fields)
    } else if with_context {
        let mut value = serialize_task(runtime, &task)?;
        let object = value.as_object_mut().ok_or_else(|| {
            OrbitError::Execution("task JSON projection did not produce an object".to_string())
        })?;
        object.insert(
            "related_docs".to_string(),
            serde_json::to_value(runtime.related_docs_for_task(&task, max_docs)?).map_err(
                |error| OrbitError::Execution(format!("serialize related docs: {error}")),
            )?,
        );
        Ok(value)
    } else {
        serialize_task(runtime, &task)
    }
}

fn optional_usize_alias(input: &Value, names: &[&str]) -> Result<Option<usize>, OrbitError> {
    for name in names {
        let Some(value) = input.get(*name) else {
            continue;
        };
        return match value {
            Value::Number(number) => number
                .as_u64()
                .ok_or_else(|| {
                    OrbitError::InvalidInput(format!("`{name}` must be an unsigned integer"))
                })
                .and_then(|value| {
                    usize::try_from(value)
                        .map(Some)
                        .map_err(|_| OrbitError::InvalidInput(format!("`{name}` is too large")))
                }),
            Value::String(raw) => raw.trim().parse::<usize>().map(Some).map_err(|error| {
                OrbitError::InvalidInput(format!("`{name}` must be an unsigned integer: {error}"))
            }),
            _ => Err(OrbitError::InvalidInput(format!(
                "`{name}` must be an unsigned integer"
            ))),
        };
    }
    Ok(None)
}

pub(super) fn start(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let id = required_string(&input, &["id"], "id")?;
    let task = runtime.start_task_with_identity_and_crew(
        &id,
        optional_string(&input, "note")?,
        optional_string(&input, "comment")?,
        agent,
        model,
        optional_string(&input, "crew")?,
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
            relations: parse_relations(&input)?,
            tags: optional_csv_or_string_list_alias(&input, &["tags", "tag"])?,
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
            task_type: optional_string_alias(&input, &["type", "task_type", "taskType"])?
                .map(|value| parse_task_type("type", &value))
                .transpose()?,
            source_task_id: optional_raw_string_alias(
                &input,
                &["source_task_id", "source_task", "sourceTaskId"],
            )?
            .map(empty_string_to_none),
            planned_by: optional_raw_string(&input, "planned_by")?.map(empty_string_to_none),
            implemented_by: optional_raw_string(&input, "implemented_by")?
                .map(empty_string_to_none),
            pr_status: optional_raw_string(&input, "pr_status")?.map(empty_string_to_none),
            job_run_id: optional_raw_string(&input, "job_run_id")?.map(empty_string_to_none),
            crew: optional_raw_string(&input, "crew")?.map(empty_string_to_none),
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

fn optional_raw_string_alias(input: &Value, keys: &[&str]) -> Result<Option<String>, OrbitError> {
    for key in keys {
        if let Some(value) = input.get(*key) {
            return match value {
                Value::Null => Ok(None),
                Value::String(raw) => Ok(Some(raw.to_string())),
                _ => Err(OrbitError::InvalidInput(format!(
                    "`{key}` must be a string"
                ))),
            };
        }
    }
    Ok(None)
}
