use std::path::Path;

use orbit_common::fs::task_io::prune_missing_context_files;
use orbit_engine::{DispatchError, WORKFLOW_RUN_FAILED_EVENT};
use orbit_types::task::{Task, TaskHistoryEntry, TaskStatus};
use serde_json::Value;

use crate::OrbitRuntime;
use crate::application::task::{canonicalize_context_files_for_read, context_workspace_root};
use crate::runtime::run_input::singular_task_id_from_input;

pub(crate) fn associated_task_ids(input: &Value) -> Vec<String> {
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

pub(crate) fn task_context_for_agent_input(
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
    let task_history = runtime.get_task_history(task_id).map_err(|err| {
        DispatchError::CliInvocationFailed(format!(
            "load task `{task_id}` history for agent envelope: {err}"
        ))
    })?;
    Ok(Some(agent_task_context_json(
        &task,
        &task_history,
        input,
        &runtime.paths().repo_root,
    )))
}

fn agent_task_context_json(
    task: &Task,
    task_history: &[TaskHistoryEntry],
    input: &Value,
    fallback_repo_root: &Path,
) -> Value {
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

    // `json!` with a braced literal always yields `Value::Object`; the fallback
    // arm keeps this total so the agent context never panics on a malformed
    // literal, and takes the map by value instead of cloning it.
    let mut context = match serde_json::json!({
        "id": task.id.clone(),
        "status": task.status.cli_name(),
        "terminal": refuses_implementer_writes(task.status),
        "title": task.title.clone(),
        "description": task.description.clone(),
        "acceptance_criteria": task.acceptance_criteria.clone(),
        "plan": task.plan.clone(),
        "context_files": kept_context_files,
        "tags": task.tags.clone(),
        "required_tools": task.required_tools.clone(),
        "external_refs": task.external_refs.clone(),
        "workspace_path": workspace_path,
        "repo_root": repo_root,
    }) {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };

    if !task.execution_summary.trim().is_empty() {
        context.insert(
            "execution_summary".to_string(),
            Value::String(task.execution_summary.clone()),
        );
    }
    if let Some(status_note) = workflow_failure_status_note(task_history) {
        context.insert(
            "status_note".to_string(),
            Value::String(status_note.to_string()),
        );
    }

    Value::Object(context)
}

fn workflow_failure_status_note(task_history: &[TaskHistoryEntry]) -> Option<&str> {
    task_history.iter().rev().find_map(|entry| {
        (entry.event == WORKFLOW_RUN_FAILED_EVENT)
            .then_some(entry.note.as_deref())
            .flatten()
            .filter(|note| !note.trim().is_empty())
    })
}

/// Whether the task record refuses the writes an implementer must make.
///
/// Mirrors the `update_task` gate in `command::task::update` and its
/// `orbit.task.update` tool-host twin: `Done` rejects every non-comment
/// mutation, and `Archived` rejects everything except the bare restore to
/// backlog. Neither admits an `execution_summary`, so an implement invocation
/// dispatched against one of these can never persist what it produces.
///
/// An implement invocation is not guaranteed to be the only actor
/// on its task. The executor re-dispatches a failed `agent_implement` step once
/// after its `recovery_activity` succeeds, and a task can be promoted through
/// the review/approve surface while an attempt is still running. Naming the
/// condition in the envelope lets an invocation that has nothing left to do
/// exit up front, instead of discovering it at its final persist call. See
/// The envelope is a dispatch-time snapshot, so `agent_implement` also
/// re-checks status mid-run.
fn refuses_implementer_writes(status: TaskStatus) -> bool {
    matches!(status, TaskStatus::Done | TaskStatus::Archived)
}

fn push_unique_task_id(task_ids: &mut Vec<String>, task_id: &str) {
    let task_id = task_id.trim();
    if !task_id.is_empty() && !task_ids.iter().any(|existing| existing == task_id) {
        task_ids.push(task_id.to_string());
    }
}
