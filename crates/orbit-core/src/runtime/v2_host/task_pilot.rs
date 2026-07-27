//! Deterministic support actions for the task-pilot workflow [ORB-10510].
//!
//! The agent leg only proposes task metadata. These actions own discovery,
//! partitioning, canonical selector validation, and the sole permitted write:
//! replacing `context_files` on the exact tasks prepared for the run.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use orbit_common::types::{Task, TaskStatus};
use orbit_common::utility::selector::{
    anchor_path, canonical_selector_in_workspace, exists_in_workspace,
};
use orbit_engine::DispatchError;
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::command::task::TaskUpdateParams;

const DEFAULT_MAX_PARTITION_SIZE: usize = 5;
const HARD_MAX_PARTITION_SIZE: usize = 5;
const DEFAULT_MAX_TASKS: usize = 50;
const HARD_MAX_TASKS: usize = 500;
const NO_DIFF_TAGS: [&str; 2] = ["no-diff-needed", "no-diff-expected"];

pub(super) fn prepare(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<Value, DispatchError> {
    let workspace_root = requested_workspace_root(runtime, action, input)?;
    let max_partition_size = bounded_usize(
        action,
        input,
        "max_partition_size",
        DEFAULT_MAX_PARTITION_SIZE,
        HARD_MAX_PARTITION_SIZE,
    )?;
    let max_tasks = bounded_usize(
        action,
        input,
        "max_tasks",
        DEFAULT_MAX_TASKS,
        HARD_MAX_TASKS,
    )?;
    let explicit_task_ids = string_array(input, "task_ids", action)?;
    let explicit_mode = !explicit_task_ids.is_empty();
    let all_tasks = runtime
        .list_tasks()
        .map_err(|error| action_failed(action, format!("list workspace tasks: {error}")))?;
    let by_id = all_tasks
        .iter()
        .map(|task| (task.id.as_str(), task))
        .collect::<BTreeMap<_, _>>();

    let (mode, selected, excluded) = if explicit_mode {
        let mut seen = BTreeSet::new();
        let selected = explicit_task_ids
            .iter()
            .map(|task_id| {
                if !seen.insert(task_id.as_str()) {
                    return Err(action_failed(
                        action,
                        format!("explicit task_ids contains duplicate {task_id}"),
                    ));
                }
                by_id.get(task_id.as_str()).copied().ok_or_else(|| {
                    action_failed(
                        action,
                        format!("task {task_id} does not exist in the selected workspace"),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        ("explicit", selected, Vec::new())
    } else {
        let mut selected = Vec::new();
        let mut excluded = Vec::new();
        let mut candidates = all_tasks.iter().collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then(left.id.cmp(&right.id))
        });
        for task in candidates {
            let reason = automatic_exclusion_reason(task);
            if let Some(reason) = reason {
                excluded.push(json!({ "task_id": task.id, "reason": reason }));
            } else {
                selected.push(task);
            }
        }
        ("automatic", selected, excluded)
    };

    if selected.len() > max_tasks {
        return Err(action_failed(
            action,
            format!(
                "{mode} task-pilot selection contains {} tasks, exceeding max_tasks {max_tasks}",
                selected.len()
            ),
        ));
    }

    let task_snapshots = selected
        .iter()
        .map(|task| task_snapshot(task))
        .collect::<Vec<_>>();
    let partitions = selected
        .chunks(max_partition_size)
        .enumerate()
        .map(|(partition_index, tasks)| {
            json!({
                "partition_index": partition_index,
                "task_ids": tasks.iter().map(|task| task.id.clone()).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "mode": mode,
        "workspace_path": workspace_root,
        "task_count": selected.len(),
        "task_ids": selected.iter().map(|task| task.id.clone()).collect::<Vec<_>>(),
        "tasks": task_snapshots,
        "partition_size": max_partition_size,
        "partition_count": partitions.len(),
        "partitions": partitions,
        "excluded": excluded,
    }))
}

pub(super) fn apply(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<Value, DispatchError> {
    let workspace_root = requested_workspace_root(runtime, action, input)?;
    let prepared = input
        .get("prepared")
        .and_then(Value::as_object)
        .ok_or_else(|| action_failed(action, "`prepared` must be an object"))?;
    let prepared_workspace = prepared
        .get("workspace_path")
        .and_then(Value::as_str)
        .ok_or_else(|| action_failed(action, "prepared.workspace_path must be a string"))?;
    if Path::new(prepared_workspace) != workspace_root {
        return Err(action_failed(
            action,
            format!(
                "prepared workspace {prepared_workspace} does not match active workspace {}",
                workspace_root.display()
            ),
        ));
    }
    let mode = prepared
        .get("mode")
        .and_then(Value::as_str)
        .ok_or_else(|| action_failed(action, "prepared.mode must be a string"))?;
    let expected_partitions = prepared
        .get("partitions")
        .and_then(Value::as_array)
        .ok_or_else(|| action_failed(action, "prepared.partitions must be an array"))?;
    let prepared_tasks = prepared
        .get("tasks")
        .and_then(Value::as_array)
        .ok_or_else(|| action_failed(action, "prepared.tasks must be an array"))?;
    let results = input
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| action_failed(action, "`results` must be an array"))?;

    if results.len() != expected_partitions.len() {
        return Err(action_failed(
            action,
            format!(
                "expected {} task-pilot partition results, received {}",
                expected_partitions.len(),
                results.len()
            ),
        ));
    }

    let prepared_before = prepared_tasks
        .iter()
        .map(|entry| {
            let task_id = required_string(entry, "task_id", action)?;
            let before = required_string_array(entry, "context_files_before", action)?;
            Ok((task_id.to_string(), before))
        })
        .collect::<Result<BTreeMap<_, _>, DispatchError>>()?;

    // Validate every partition and every selector before the first mutation.
    // A malformed or failed partition therefore cannot partially apply the
    // otherwise-valid partitions that happened to precede it.
    let mut validated = Vec::with_capacity(prepared_before.len());
    let mut seen_task_ids = BTreeSet::new();
    for (position, (expected, result)) in expected_partitions.iter().zip(results).enumerate() {
        let expected_index = expected
            .get("partition_index")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                action_failed(
                    action,
                    format!("prepared.partitions[{position}].partition_index is invalid"),
                )
            })?;
        let expected_ids = required_string_array(expected, "task_ids", action)?;
        let result_object = result.as_object().ok_or_else(|| {
            action_failed(
                action,
                format!("partition {expected_index} failed or returned no structured result"),
            )
        })?;
        let result_index = result_object
            .get("partition_index")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                action_failed(
                    action,
                    format!("partition result {position} is missing partition_index"),
                )
            })?;
        if result_index != expected_index {
            return Err(action_failed(
                action,
                format!(
                    "partition result {position} reports index {result_index}, expected {expected_index}"
                ),
            ));
        }
        let result_ids = result_object
            .get("task_ids")
            .ok_or_else(|| action_failed(action, "`task_ids` must be an array"))
            .and_then(|value| string_array_value(value, "task_ids", action))?;
        if result_ids != expected_ids {
            return Err(action_failed(
                action,
                format!("partition {expected_index} task_ids do not match the prepared partition"),
            ));
        }
        let assessments = result_object
            .get("tasks")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                action_failed(
                    action,
                    format!("partition {expected_index}.tasks must be an array"),
                )
            })?;
        if assessments.len() != expected_ids.len() {
            return Err(action_failed(
                action,
                format!(
                    "partition {expected_index} returned {} task assessments for {} tasks",
                    assessments.len(),
                    expected_ids.len()
                ),
            ));
        }
        let assessments_by_id = assessments
            .iter()
            .map(|assessment| {
                let task_id = required_string(assessment, "task_id", action)?;
                Ok((task_id.to_string(), assessment))
            })
            .collect::<Result<BTreeMap<_, _>, DispatchError>>()?;
        if assessments_by_id.len() != assessments.len() {
            return Err(action_failed(
                action,
                format!("partition {expected_index} contains duplicate task assessments"),
            ));
        }

        for task_id in expected_ids {
            if !seen_task_ids.insert(task_id.clone()) {
                return Err(action_failed(
                    action,
                    format!("task {task_id} appears in more than one partition result"),
                ));
            }
            let assessment = assessments_by_id.get(&task_id).ok_or_else(|| {
                action_failed(
                    action,
                    format!("partition {expected_index} omitted task {task_id}"),
                )
            })?;
            let expected_before = prepared_before.get(&task_id).ok_or_else(|| {
                action_failed(action, format!("task {task_id} was not in prepared.tasks"))
            })?;
            let reported_before =
                required_string_array(assessment, "context_files_before", action)?;
            if &reported_before != expected_before {
                return Err(action_failed(
                    action,
                    format!(
                        "task {task_id} context_files_before does not match the prepared snapshot"
                    ),
                ));
            }
            let current = runtime.get_task(&task_id).map_err(|error| {
                action_failed(action, format!("reload task {task_id}: {error}"))
            })?;
            if current.context_files != *expected_before {
                return Err(action_failed(
                    action,
                    format!(
                        "task {task_id} context_files changed after preparation; refusing stale pilot output"
                    ),
                ));
            }
            let disposition = required_string(assessment, "disposition", action)?;
            let after = required_string_array(assessment, "context_files_after", action)?;
            validate_after_selectors(
                action,
                &task_id,
                disposition,
                assessment,
                &after,
                &workspace_root,
            )?;
            validate_recommendations(action, &task_id, assessment)?;
            validated.push((
                task_id,
                expected_before.clone(),
                after,
                (*assessment).clone(),
            ));
        }
    }

    if seen_task_ids.len() != prepared_before.len() {
        return Err(action_failed(
            action,
            "partition results did not cover every prepared task",
        ));
    }

    let mut task_results = Vec::with_capacity(validated.len());
    for (task_id, before, after, mut assessment) in validated {
        let changed = before != after;
        if changed {
            runtime
                .update_task(
                    &task_id,
                    TaskUpdateParams {
                        context_files: Some(after.clone()),
                        ..TaskUpdateParams::default()
                    },
                )
                .map_err(|error| {
                    action_failed(
                        action,
                        format!("apply validated context_files for task {task_id}: {error}"),
                    )
                })?;
        }
        if let Value::Object(fields) = &mut assessment {
            fields.insert("applied".to_string(), Value::Bool(changed));
        }
        task_results.push(assessment);
    }

    Ok(json!({
        "status": "success",
        "mode": mode,
        "crew": input.get("crew").cloned().unwrap_or(Value::Null),
        "workspace_path": workspace_root,
        "discovery": {
            "task_ids": prepared.get("task_ids").cloned().unwrap_or_else(|| json!([])),
            "excluded": prepared.get("excluded").cloned().unwrap_or_else(|| json!([])),
        },
        "partition_decisions": expected_partitions,
        "partition_count": expected_partitions.len(),
        "failed_partitions": [],
        "tasks": task_results,
    }))
}

fn automatic_exclusion_reason(task: &Task) -> Option<&'static str> {
    if !matches!(task.status, TaskStatus::Proposed | TaskStatus::Backlog) {
        return Some("status_not_eligible");
    }
    if !task.context_files.is_empty() {
        return Some("context_files_not_empty");
    }
    if task
        .tags
        .iter()
        .any(|tag| NO_DIFF_TAGS.contains(&tag.as_str()))
    {
        return Some("no_diff_task");
    }
    None
}

fn task_snapshot(task: &Task) -> Value {
    json!({
        "task_id": task.id,
        "title": task.title,
        "status": task.status,
        "tags": task.tags,
        "context_files_before": task.context_files,
    })
}

fn validate_after_selectors(
    action: &str,
    task_id: &str,
    disposition: &str,
    assessment: &Value,
    selectors: &[String],
    workspace_root: &Path,
) -> Result<(), DispatchError> {
    if selectors.is_empty() {
        if !matches!(disposition, "verified_no_diff" | "host_operational") {
            return Err(action_failed(
                action,
                format!(
                    "task {task_id} may keep empty context_files only with verified_no_diff or host_operational disposition"
                ),
            ));
        }
        required_string(assessment, "evidence", action)?;
        return Ok(());
    }
    if disposition != "selectors" {
        return Err(action_failed(
            action,
            format!(
                "task {task_id} has non-empty context_files_after but disposition is {disposition}"
            ),
        ));
    }

    let mut seen = BTreeSet::new();
    for selector in selectors {
        let trimmed = selector.trim();
        if !matches!(
            trimmed.split_once(':').map(|(kind, _)| kind),
            Some("file" | "dir" | "symbol")
        ) {
            return Err(action_failed(
                action,
                format!("task {task_id} selector {selector:?} must use file:, dir:, or symbol:"),
            ));
        }
        let canonical =
            canonical_selector_in_workspace(trimmed, workspace_root).map_err(|error| {
                action_failed(
                    action,
                    format!("task {task_id} selector {selector:?} is invalid: {error}"),
                )
            })?;
        if canonical != trimmed {
            return Err(action_failed(
                action,
                format!(
                    "task {task_id} selector {selector:?} is not canonical; expected {canonical:?}"
                ),
            ));
        }
        if !exists_in_workspace(&canonical, workspace_root) {
            return Err(action_failed(
                action,
                format!(
                    "task {task_id} selector {selector:?} does not resolve to an existing in-workspace target"
                ),
            ));
        }
        validate_selector_target_kind(action, task_id, &canonical, workspace_root)?;
        if !seen.insert(canonical) {
            return Err(action_failed(
                action,
                format!("task {task_id} repeats selector {selector:?}"),
            ));
        }
    }
    Ok(())
}

fn validate_recommendations(
    action: &str,
    task_id: &str,
    assessment: &Value,
) -> Result<(), DispatchError> {
    required_string(assessment, "recommended_crew", action)?;
    let complexity = required_string(assessment, "recommended_complexity", action)?;
    if !matches!(complexity, "low" | "medium" | "hard") {
        return Err(action_failed(
            action,
            format!("task {task_id} recommended_complexity must be low, medium, or hard"),
        ));
    }
    for field in [
        "blocked_by",
        "adr_conflicts",
        "utility_warnings",
        "surface_warnings",
    ] {
        string_array_value(
            assessment.get(field).ok_or_else(|| {
                action_failed(action, format!("task {task_id} is missing {field}"))
            })?,
            field,
            action,
        )?;
    }
    for field in ["duplicate_of", "already_landed"] {
        if assessment.get(field).is_none() {
            return Err(action_failed(
                action,
                format!("task {task_id} is missing {field} recommendation"),
            ));
        }
    }
    Ok(())
}

fn validate_selector_target_kind(
    action: &str,
    task_id: &str,
    selector: &str,
    workspace_root: &Path,
) -> Result<(), DispatchError> {
    let anchor = anchor_path(selector).map_err(|error| {
        action_failed(
            action,
            format!("task {task_id} selector {selector:?} has no filesystem anchor: {error}"),
        )
    })?;
    let resolved = workspace_root.join(anchor);
    let correct_kind = if selector.starts_with("dir:") {
        resolved.is_dir()
    } else {
        resolved.is_file()
    };
    if correct_kind {
        Ok(())
    } else {
        Err(action_failed(
            action,
            format!(
                "task {task_id} selector {selector:?} does not match the target's file/directory kind"
            ),
        ))
    }
}

fn requested_workspace_root(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<PathBuf, DispatchError> {
    let runtime_root = runtime.paths().repo_root.canonicalize().map_err(|error| {
        action_failed(action, format!("canonicalize runtime workspace: {error}"))
    })?;
    let requested = input
        .get("workspace_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_root.clone());
    let requested = if requested.is_absolute() {
        requested
    } else {
        runtime_root.join(requested)
    };
    let requested = requested.canonicalize().map_err(|error| {
        action_failed(
            action,
            format!(
                "canonicalize requested workspace {}: {error}",
                requested.display()
            ),
        )
    })?;
    if requested != runtime_root {
        return Err(action_failed(
            action,
            format!(
                "requested workspace {} does not match active workspace {}",
                requested.display(),
                runtime_root.display()
            ),
        ));
    }
    Ok(runtime_root)
}

fn bounded_usize(
    action: &str,
    input: &Value,
    field: &str,
    default: usize,
    max: usize,
) -> Result<usize, DispatchError> {
    let value = input
        .get(field)
        .and_then(Value::as_u64)
        .unwrap_or(default as u64);
    if value == 0 || value > max as u64 {
        return Err(action_failed(
            action,
            format!("`{field}` must be between 1 and {max}"),
        ));
    }
    Ok(value as usize)
}

fn string_array(input: &Value, field: &str, action: &str) -> Result<Vec<String>, DispatchError> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(value) => string_array_value(value, field, action),
    }
}

fn string_array_value(
    value: &Value,
    field: &str,
    action: &str,
) -> Result<Vec<String>, DispatchError> {
    value
        .as_array()
        .ok_or_else(|| action_failed(action, format!("`{field}` must be an array")))
        .and_then(|values| {
            values
                .iter()
                .map(|value| {
                    value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                        action_failed(action, format!("`{field}` must contain only strings"))
                    })
                })
                .collect()
        })
}

fn required_string_array(
    input: &Value,
    field: &str,
    action: &str,
) -> Result<Vec<String>, DispatchError> {
    input
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| action_failed(action, format!("`{field}` must be an array")))
        .and_then(|values| {
            values
                .iter()
                .map(|value| {
                    value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                        action_failed(action, format!("`{field}` must contain only strings"))
                    })
                })
                .collect()
        })
}

fn required_string<'a>(
    input: &'a Value,
    field: &str,
    action: &str,
) -> Result<&'a str, DispatchError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| action_failed(action, format!("`{field}` must be a non-empty string")))
}

fn action_failed(action: &str, message: impl Into<String>) -> DispatchError {
    DispatchError::DeterministicActionFailed {
        action: action.to_string(),
        message: message.into(),
    }
}
