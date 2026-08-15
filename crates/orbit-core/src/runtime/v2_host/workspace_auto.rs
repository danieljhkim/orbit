use std::collections::{BTreeMap, BTreeSet};

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

pub(super) fn list_epic_descendants(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<Value, DispatchError> {
    let epic_task_id = input
        .get("epic_task_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: "missing `epic_task_id`".to_string(),
        })?;
    let all_tasks = runtime.stores().tasks().list_tasks().map_err(|error| {
        DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("list tasks: {error}"),
        }
    })?;
    let task_lookup = all_tasks
        .iter()
        .cloned()
        .map(|task| (task.id.clone(), task))
        .collect::<BTreeMap<_, _>>();
    let epic =
        task_lookup
            .get(epic_task_id)
            .ok_or_else(|| DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!("epic task `{epic_task_id}` was not found"),
            })?;
    if !epic.tags.iter().any(|tag| tag == "epic") {
        return Err(DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("task `{epic_task_id}` is not tagged `epic`"),
        });
    }

    let mut remaining = all_tasks
        .into_iter()
        .filter(|task| {
            is_descendant_of(task, epic_task_id, &task_lookup)
                && !matches!(
                    task.status,
                    TaskStatus::Done | TaskStatus::Rejected | TaskStatus::Archived
                )
        })
        .map(|task| (task.id.clone(), task))
        .collect::<BTreeMap<_, _>>();
    let mut ordered = Vec::with_capacity(remaining.len());

    while !remaining.is_empty() {
        let remaining_ids = remaining.keys().cloned().collect::<BTreeSet<_>>();
        let mut ready = remaining
            .values()
            .filter(|task| {
                task.dependencies()
                    .iter()
                    .all(|dependency_id| !remaining_ids.contains(dependency_id))
            })
            .cloned()
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Err(DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!(
                    "epic `{epic_task_id}` has a dependency cycle among unfinished descendants: {}",
                    remaining_ids.into_iter().collect::<Vec<_>>().join(", ")
                ),
            });
        }
        sort_tasks_by_priority_age(&mut ready);
        for task in ready {
            remaining.remove(&task.id);
            ordered.push(task.id);
        }
    }

    Ok(json!({
        "epic_task_id": epic_task_id,
        "task_count": ordered.len(),
        "task_ids": ordered,
        "empty": ordered.is_empty(),
    }))
}

fn is_descendant_of(task: &Task, ancestor_id: &str, task_lookup: &BTreeMap<String, Task>) -> bool {
    let mut visited = BTreeSet::from([task.id.clone()]);
    let mut next_parent_id = task.parent_id();
    for _ in 0..32 {
        let Some(parent_id) = next_parent_id else {
            return false;
        };
        if parent_id == ancestor_id {
            return true;
        }
        if !visited.insert(parent_id.to_string()) {
            return false;
        }
        let Some(parent) = task_lookup.get(parent_id) else {
            return false;
        };
        next_parent_id = parent.parent_id();
    }
    false
}
