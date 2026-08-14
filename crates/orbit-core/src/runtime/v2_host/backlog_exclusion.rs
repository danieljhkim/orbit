use std::collections::BTreeMap;
use std::path::Path;

use orbit_common::types::{Task, TaskStatus, prune_missing_context_files, task_dependencies_ready};
use orbit_common::utility::path::workspace_relative_paths_overlap;
use orbit_common::utility::selector::canonical_selector_in_workspace;
use orbit_engine::DispatchError;
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
    EpicChild,
    EpicRoot,
    GroupMemberConflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EpicFamilyMembership {
    Child,
    Root,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct BacklogTaskConflict {
    requested_file: String,
    locking_task_id: String,
}

fn active_task_lock_holders<'a>(
    tasks: impl IntoIterator<Item = &'a Task>,
    workspace_root: &Path,
) -> BTreeMap<String, Vec<String>> {
    let mut holders: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for task in tasks {
        if matches!(task.status, TaskStatus::InProgress | TaskStatus::Review) {
            for file in existing_lock_context_files(task, workspace_root) {
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

fn task_overlap_conflicts(
    task: &Task,
    holders: &BTreeMap<String, Vec<String>>,
    workspace_root: &Path,
) -> Vec<BacklogTaskConflict> {
    let mut conflicts = Vec::new();
    for requested_file in existing_lock_context_files(task, workspace_root) {
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

fn existing_lock_context_files(task: &Task, workspace_root: &Path) -> Vec<String> {
    let canonical = task
        .context_files
        .iter()
        .filter_map(|entry| canonical_selector_in_workspace(entry, workspace_root).ok())
        .collect::<Vec<_>>();
    let (kept, _dropped) = prune_missing_context_files(workspace_root, canonical);
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
        let status_by_id = runtime.task_status_index().map_err(|err| {
            DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!("load global task status projection: {err}"),
            }
        })?;
        let workspace_root = runtime.paths().repo_root.as_path();
        let lock_holders = active_task_lock_holders(task_lookup.values(), workspace_root);
        let mut backlog: Vec<Task> = all_tasks
            .into_iter()
            .filter(|task| {
                task.status == TaskStatus::Backlog && task_dependencies_ready(task, &status_by_id)
            })
            .collect();
        sort_tasks_by_priority_age(&mut backlog);
        let mut excluded = Vec::new();
        backlog.retain(|task| {
            let Some(membership) = epic_family_membership(task, &task_lookup) else {
                return true;
            };
            excluded.push(BacklogTaskExclusion {
                id: task.id.clone(),
                reason: match membership {
                    EpicFamilyMembership::Root => BacklogTaskExclusionReason::EpicRoot,
                    EpicFamilyMembership::Child => BacklogTaskExclusionReason::EpicChild,
                },
                conflicts: Vec::new(),
            });
            false
        });
        if !lock_holders.is_empty() {
            let direct_conflicts: BTreeMap<String, Vec<BacklogTaskConflict>> = backlog
                .iter()
                .filter_map(|task| {
                    let conflicts = task_overlap_conflicts(task, &lock_holders, workspace_root);
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
                backlog = kept;
            }
        }
        excluded.sort_by(|a, b| a.id.cmp(&b.id));
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
                "parent_id": t.parent_id(),
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

pub(super) fn sort_tasks_by_priority_age(tasks: &mut [Task]) {
    let rank = |priority: orbit_common::types::TaskPriority| match priority {
        orbit_common::types::TaskPriority::Critical => 0,
        orbit_common::types::TaskPriority::High => 1,
        orbit_common::types::TaskPriority::Medium => 2,
        orbit_common::types::TaskPriority::Low => 3,
    };
    tasks.sort_by(|left, right| {
        rank(left.priority)
            .cmp(&rank(right.priority))
            .then(left.created_at.cmp(&right.created_at))
    });
}

pub(super) fn epic_family_membership(
    task: &Task,
    task_lookup: &BTreeMap<String, Task>,
) -> Option<EpicFamilyMembership> {
    if task.tags.iter().any(|tag| tag == "epic") {
        return Some(EpicFamilyMembership::Root);
    }

    let mut visited = vec![task.id.clone()];
    let mut next_parent_id = task.parent_id().map(ToOwned::to_owned);
    for _ in 0..MAX_TASK_PARENT_CHAIN_DEPTH {
        let parent_id = next_parent_id?;
        if visited.iter().any(|task_id| task_id == &parent_id) {
            return None;
        }
        let parent = task_lookup.get(&parent_id)?;
        if parent.tags.iter().any(|tag| tag == "epic") {
            return Some(EpicFamilyMembership::Child);
        }
        visited.push(parent.id.clone());
        next_parent_id = parent.parent_id().map(ToOwned::to_owned);
    }
    None
}

fn task_root_id(task: &Task, task_lookup: &BTreeMap<String, Task>) -> String {
    let mut path = vec![task.id.clone()];
    let mut root_id = task.id.clone();
    let mut next_parent_id = task.parent_id().map(ToOwned::to_owned);

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
        next_parent_id = parent.parent_id().map(ToOwned::to_owned);
    }

    root_id
}
