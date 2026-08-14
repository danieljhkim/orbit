use std::collections::BTreeMap;

use orbit_common::types::{Task, TaskStatus, task_dependencies_ready};
use orbit_engine::DispatchError;
use serde_json::{Value, json};

use crate::OrbitRuntime;

use super::backlog_exclusion::{
    EpicFamilyMembership, epic_family_membership, list_backlog_tasks, sort_tasks_by_priority_age,
};

pub(super) fn classify_workspace_auto_tasks(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<Value, DispatchError> {
    let all_tasks = runtime.stores().tasks().list_tasks().map_err(|err| {
        DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("list tasks: {err}"),
        }
    })?;
    let task_lookup: BTreeMap<String, Task> = all_tasks
        .iter()
        .cloned()
        .map(|task| (task.id.clone(), task))
        .collect();

    let mut in_progress_epics = all_tasks
        .iter()
        .filter(|task| {
            task.status == TaskStatus::InProgress
                && epic_family_membership(task, &task_lookup) == Some(EpicFamilyMembership::Root)
        })
        .cloned()
        .collect::<Vec<_>>();
    sort_tasks_by_priority_age(&mut in_progress_epics);
    if let Some(epic) = in_progress_epics.first() {
        return Ok(json!({
            "decision": "hold",
            "loose_task_ids": [],
            "epic_task_id": epic.id,
        }));
    }

    let backlog = list_backlog_tasks(runtime, action, input)?;
    let loose_task_ids = backlog
        .get("task_ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !loose_task_ids.is_empty() {
        return Ok(json!({
            "decision": "ship",
            "loose_task_ids": loose_task_ids,
            "epic_task_id": null,
        }));
    }

    let status_by_id =
        runtime
            .task_status_index()
            .map_err(|err| DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!("load global task status projection: {err}"),
            })?;
    let mut backlog_epics = all_tasks
        .into_iter()
        .filter(|task| {
            task.status == TaskStatus::Backlog
                && epic_family_membership(task, &task_lookup) == Some(EpicFamilyMembership::Root)
                && task_dependencies_ready(task, &status_by_id)
        })
        .collect::<Vec<_>>();
    sort_tasks_by_priority_age(&mut backlog_epics);
    if let Some(epic) = backlog_epics.first() {
        return Ok(json!({
            "decision": "epic",
            "loose_task_ids": [],
            "epic_task_id": epic.id,
        }));
    }

    Ok(json!({
        "decision": "empty",
        "loose_task_ids": [],
        "epic_task_id": null,
    }))
}
