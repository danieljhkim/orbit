use std::collections::BTreeMap;
use std::path::PathBuf;

use orbit_common::types::{Task, TaskStatus, TaskType, prune_missing_context_files};
use orbit_common::utility::path::workspace_relative_paths_overlap;
use orbit_common::utility::selector::canonical_selector_in_workspace;
use orbit_engine::activity_job::DispatchError;
use serde::Serialize;
use serde_json::Value;

use crate::OrbitRuntime;

const MAX_TASK_PARENT_CHAIN_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct BacklogTaskExclusion {
    id: String,
    reason: BacklogTaskExclusionReason,
    conflicts: Vec<BacklogTaskConflict>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum BacklogTaskExclusionReason {
    ContextLockConflict,
    GroupMemberConflict,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct BacklogTaskConflict {
    requested_file: String,
    locking_task_id: String,
}

fn active_task_lock_holders<'a>(
    tasks: impl IntoIterator<Item = &'a Task>,
) -> BTreeMap<String, Vec<String>> {
    let mut holders: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for task in tasks {
        if matches!(task.status, TaskStatus::InProgress | TaskStatus::Review) {
            for file in existing_lock_context_files(task) {
                holders.entry(file).or_default().push(task.id.clone());
            }
        }
    }
    for locking_task_ids in holders.values_mut() {
        locking_task_ids.sort();
        locking_task_ids.dedup();
    }
    holders
}

fn is_epic_terminal_status(status: TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Done | TaskStatus::Blocked | TaskStatus::Archived | TaskStatus::Rejected
    )
}

fn epic_state_for_task_status(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Done | TaskStatus::Archived => "done",
        TaskStatus::Blocked | TaskStatus::Rejected => "blocked",
        TaskStatus::InProgress | TaskStatus::Review => "in_flight",
        TaskStatus::Proposed | TaskStatus::Friction | TaskStatus::Backlog | TaskStatus::Someday => {
            "pending"
        }
    }
}

fn task_overlap_conflicts(
    task: &Task,
    holders: &BTreeMap<String, Vec<String>>,
) -> Vec<BacklogTaskConflict> {
    let mut conflicts = Vec::new();
    for requested_file in existing_lock_context_files(task) {
        for (held_file, locking_task_ids) in holders {
            if workspace_relative_paths_overlap(&requested_file, held_file) {
                for locking_task_id in locking_task_ids {
                    conflicts.push(BacklogTaskConflict {
                        requested_file: requested_file.clone(),
                        locking_task_id: locking_task_id.clone(),
                    });
                }
            }
        }
    }
    conflicts.sort();
    conflicts.dedup();
    conflicts
}

fn existing_lock_context_files(task: &Task) -> Vec<String> {
    let workspace_root = task
        .workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let canonical = task
        .context_files
        .iter()
        .filter_map(|entry| canonical_selector_in_workspace(entry, &workspace_root).ok())
        .collect::<Vec<_>>();
    let (kept, _dropped) = prune_missing_context_files(&workspace_root, canonical);
    kept
}

pub(super) fn list_backlog_tasks(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<Value, DispatchError> {
    let max_tasks = input
        .get("max_tasks")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .min(500) as usize;
    let explicit_task_ids: Vec<String> = input
        .get("task_ids")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let (mut tasks, excluded_entries) = if explicit_task_ids.is_empty() {
        let all_tasks = runtime.stores().tasks().list().map_err(|err| {
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
        let lock_holders = active_task_lock_holders(task_lookup.values());
        let mut backlog: Vec<Task> = all_tasks
            .into_iter()
            .filter(|task| task.status == TaskStatus::Backlog)
            .collect();
        backlog.sort_by(|a, b| {
            let rank = |p: orbit_common::types::TaskPriority| match p {
                orbit_common::types::TaskPriority::Critical => 0,
                orbit_common::types::TaskPriority::High => 1,
                orbit_common::types::TaskPriority::Medium => 2,
                orbit_common::types::TaskPriority::Low => 3,
            };
            rank(a.priority)
                .cmp(&rank(b.priority))
                .then(a.created_at.cmp(&b.created_at))
        });
        let mut excluded = Vec::new();
        if !lock_holders.is_empty() {
            let direct_conflicts: BTreeMap<String, Vec<BacklogTaskConflict>> = backlog
                .iter()
                .filter_map(|task| {
                    let conflicts = task_overlap_conflicts(task, &lock_holders);
                    (!conflicts.is_empty()).then(|| (task.id.clone(), conflicts))
                })
                .collect();
            let mut root_trigger: BTreeMap<String, Vec<BacklogTaskConflict>> = BTreeMap::new();
            for task in &backlog {
                if let Some(conflicts) = direct_conflicts.get(&task.id) {
                    let root_id = task_root_id(task, &task_lookup);
                    // Backlog is already priority/age sorted; the first direct
                    // conflict in that order supplies group-member attribution.
                    root_trigger
                        .entry(root_id)
                        .or_insert_with(|| conflicts.clone());
                }
            }
            if !root_trigger.is_empty() {
                let mut kept = Vec::new();
                for task in backlog {
                    let root_id = task_root_id(&task, &task_lookup);
                    if let Some(trigger_conflicts) = root_trigger.get(&root_id) {
                        if let Some(conflicts) = direct_conflicts.get(&task.id) {
                            excluded.push(BacklogTaskExclusion {
                                id: task.id.clone(),
                                reason: BacklogTaskExclusionReason::ContextLockConflict,
                                conflicts: conflicts.clone(),
                            });
                        } else {
                            excluded.push(BacklogTaskExclusion {
                                id: task.id.clone(),
                                reason: BacklogTaskExclusionReason::GroupMemberConflict,
                                conflicts: trigger_conflicts.clone(),
                            });
                        }
                    } else {
                        kept.push(task);
                    }
                }
                excluded.sort_by(|a, b| a.id.cmp(&b.id));
                backlog = kept;
            }
        }
        (backlog, Some(excluded))
    } else {
        let tasks = explicit_task_ids
            .iter()
            .map(|task_id| {
                runtime
                    .get_task(task_id)
                    .map_err(|err| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: format!("load task {task_id}: {err}"),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        (tasks, None)
    };
    tasks.truncate(max_tasks);
    let ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
    let bundles: Vec<Vec<String>> = ids.iter().map(|task_id| vec![task_id.clone()]).collect();
    let task_objs: Vec<Value> = tasks
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "title": t.title,
                "type": t.task_type.to_string(),
                "priority": t.priority.to_string(),
                "context_files": t.context_files,
                "parent_id": t.parent_id,
            })
        })
        .collect();
    let mut payload = serde_json::Map::new();
    payload.insert("task_count".to_string(), Value::from(task_objs.len()));
    payload.insert("task_ids".to_string(), serde_json::json!(ids));
    payload.insert("tasks".to_string(), serde_json::json!(task_objs));
    payload.insert("bundles".to_string(), serde_json::json!(bundles));
    // Keep this Rust serialization contract in sync with
    // crates/orbit-core/assets/activities/list_backlog_tasks.yaml.
    if let Some(excluded) = excluded_entries {
        payload.insert(
            "excluded".to_string(),
            serde_json::to_value(excluded).map_err(|err| {
                DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: format!("serialize excluded backlog tasks: {err}"),
                }
            })?,
        );
    }
    Ok(Value::Object(payload))
}

pub(super) fn load_epic(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<Value, DispatchError> {
    let epic_id = input
        .get("epic_task_id")
        .and_then(Value::as_str)
        .ok_or_else(|| DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: "missing `epic_task_id`".to_string(),
        })?;
    let epic =
        runtime
            .get_task(epic_id)
            .map_err(|err| DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!("load epic {epic_id}: {err}"),
            })?;
    if epic.task_type != TaskType::Epic {
        return Err(DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!(
                "task `{epic_id}` has type `{}`; expected `epic`",
                epic.task_type
            ),
        });
    }
    let subtasks = runtime
        .list_tasks_filtered(None, None, Some(epic_id), None, None, None)
        .map_err(|err| DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("list subtasks of {epic_id}: {err}"),
        })?;
    let final_subtasks = subtasks
        .iter()
        .map(|t| {
            (
                t.id.clone(),
                serde_json::json!({
                    "state": epic_state_for_task_status(t.status),
                    "status": t.status.to_string(),
                    "title": t.title,
                }),
            )
        })
        .collect::<serde_json::Map<String, Value>>();
    let open_subtasks = subtasks
        .iter()
        .filter(|task| !is_epic_terminal_status(task.status))
        .collect::<Vec<_>>();
    let subtask_payload: Vec<Value> = open_subtasks
        .iter()
        .filter(|t| !matches!(t.status, TaskStatus::Done | TaskStatus::Archived))
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "title": t.title,
                "description": t.description,
                "type": t.task_type.to_string(),
                "status": t.status.to_string(),
                "context_files": t.context_files,
            })
        })
        .collect();
    Ok(serde_json::json!({
        "epic": {
            "id": epic.id,
            "title": epic.title,
            "description": epic.description,
            "type": epic.task_type.to_string(),
            "status": epic.status.to_string(),
        },
        "subtasks": subtask_payload,
        "all_terminal": open_subtasks.is_empty(),
        "final_state": {
            "epic_id": epic.id.clone(),
            "subtasks": final_subtasks,
        },
    }))
}

pub(super) fn summarize_epic(input: &Value) -> Result<Value, DispatchError> {
    let state = input.get("state").cloned().unwrap_or(Value::Null);
    let subtasks_map = state
        .get("subtasks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut done = 0u64;
    let mut failed = 0u64;
    let mut blocked = 0u64;
    let mut in_flight = 0u64;
    let mut unfinished_ids: Vec<String> = Vec::new();
    for (id, entry) in subtasks_map.iter() {
        let entry_state = entry
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        match entry_state {
            "done" => done += 1,
            "blocked" => {
                blocked += 1;
                unfinished_ids.push(id.clone());
            }
            "failed" => {
                failed += 1;
                unfinished_ids.push(id.clone());
            }
            "in_flight" | "pending" => {
                in_flight += 1;
                unfinished_ids.push(id.clone());
            }
            _ => {
                unfinished_ids.push(id.clone());
            }
        }
    }
    let total = subtasks_map.len() as u64;
    let message = if total == 0 {
        "epic had no subtasks".to_string()
    } else if unfinished_ids.is_empty() {
        format!("all {total} subtasks done")
    } else {
        format!(
            "{done}/{total} done; {failed} failed, {blocked} blocked, \
             {in_flight} in flight/pending"
        )
    };
    Ok(serde_json::json!({
        "total": total,
        "done": done,
        "failed": failed,
        "blocked": blocked,
        "in_flight": in_flight,
        "unfinished_ids": unfinished_ids,
        "message": message,
    }))
}

fn task_root_id(task: &Task, task_lookup: &BTreeMap<String, Task>) -> String {
    let mut path = vec![task.id.clone()];
    let mut root_id = task.id.clone();
    let mut next_parent_id = task.parent_id.clone();

    for _ in 0..MAX_TASK_PARENT_CHAIN_DEPTH {
        let Some(parent_id) = next_parent_id else {
            return root_id;
        };

        if let Some(cycle_start) = path.iter().position(|task_id| task_id == &parent_id) {
            return path[cycle_start..].iter().min().cloned().unwrap_or(root_id);
        }

        let Some(parent) = task_lookup.get(&parent_id) else {
            return root_id;
        };

        root_id = parent.id.clone();
        path.push(parent.id.clone());
        next_parent_id = parent.parent_id.clone();
    }

    root_id
}

#[cfg(test)]
#[path = "backlog_exclusion_tests.rs"]
mod tests;
