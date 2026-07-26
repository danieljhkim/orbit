use std::path::Path;

use orbit_common::types::{Task, prune_missing_context_files};
use orbit_engine::DispatchError;
use serde_json::Value;

use crate::OrbitRuntime;
use crate::command::task::{canonicalize_context_files_for_read, context_workspace_root};
use crate::runtime::run_input::singular_task_id_from_input;

pub(super) fn associated_task_ids(input: &Value) -> Vec<String> {
    let mut task_ids = Vec::new();
    if let Some(task_id) = input.get("task_id").and_then(Value::as_str) {
        push_unique_task_id(&mut task_ids, task_id);
    }
    if let Some(items) = input.get("task_ids").and_then(Value::as_array) {
        for item in items {
            if let Some(task_id) = item.as_str() {
                push_unique_task_id(&mut task_ids, task_id);
            }
        }
    }
    if let Some(items) = input.get("tasks").and_then(Value::as_array) {
        for item in items {
            if let Some(task_id) = item.as_str() {
                push_unique_task_id(&mut task_ids, task_id);
                continue;
            }
            if let Some(task_id) = item
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| item.get("task_id").and_then(Value::as_str))
            {
                push_unique_task_id(&mut task_ids, task_id);
            }
        }
    }
    task_ids
}

pub(super) fn task_context_for_agent_input(
    runtime: &OrbitRuntime,
    input: &Value,
) -> Result<Option<Value>, DispatchError> {
    let Some(task_id) = singular_task_id_from_input(input) else {
        return Ok(None);
    };
    let task = runtime.get_task(task_id).map_err(|err| {
        DispatchError::CliInvocationFailed(format!(
            "load task `{task_id}` for agent envelope: {err}"
        ))
    })?;
    Ok(Some(agent_task_context_json(
        &task,
        input,
        &runtime.paths().repo_root,
    )))
}

fn agent_task_context_json(task: &Task, input: &Value, fallback_repo_root: &Path) -> Value {
    let workspace_path = input
        .get("workspace_path")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let repo_root = input
        .get("repo_root")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let prune_root = context_workspace_root(fallback_repo_root, workspace_path.as_deref());
    let canonical_context_files =
        canonicalize_context_files_for_read(&task.context_files, &prune_root);
    let (kept_context_files, _dropped) =
        prune_missing_context_files(&prune_root, canonical_context_files);

    serde_json::json!({
        "id": task.id.clone(),
        "title": task.title.clone(),
        "description": task.description.clone(),
        "acceptance_criteria": task.acceptance_criteria.clone(),
        "plan": task.plan.clone(),
        "context_files": kept_context_files,
        "tags": task.tags.clone(),
        "external_refs": task.external_refs.clone(),
        "workspace_path": workspace_path,
        "repo_root": repo_root,
    })
}

fn push_unique_task_id(task_ids: &mut Vec<String>, task_id: &str) {
    let task_id = task_id.trim();
    if !task_id.is_empty() && !task_ids.iter().any(|existing| existing == task_id) {
        task_ids.push(task_id.to_string());
    }
}
