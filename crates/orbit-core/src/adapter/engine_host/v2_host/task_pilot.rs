//! Deterministic support actions for the task-pilot workflow [ORB-10510].
//!
//! The agent leg only proposes task metadata. These actions own discovery,
//! partitioning and canonical selector validation. Ordinarily the sole write
//! is replacing `context_files` on the exact tasks prepared for the run. A
//! CI-failure sweep may additionally request explicit admission: after the
//! selectors and every recommendation validate, this boundary promotes only a
//! current, warning-free repair from `proposed` to `backlog`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use orbit_common::fs::selector::{
    anchor_path, canonical_selector_in_workspace, exists_in_workspace,
};
use orbit_engine::DispatchError;
use orbit_store::contracts::JobRunQuery;
use orbit_types::task::{Task, TaskStatus};
use orbit_types::workflow::JobRunState;
use serde_json::{Value, json};

use crate::OrbitRuntime;

mod apply;
mod source;

pub(super) use apply::apply;
use source::{GitPathKind, SourceSnapshot, requested_base_branch, resolve_source_snapshot};

const DEFAULT_MAX_PARTITION_SIZE: usize = 5;
const HARD_MAX_PARTITION_SIZE: usize = 5;
const DEFAULT_MAX_TASKS: usize = 50;
const HARD_MAX_TASKS: usize = 500;
const NO_DIFF_TAGS: [&str; 2] = ["no-diff-needed", "no-diff-expected"];
const TASK_PILOT_JOB_ID: &str = "task_pilot_pipeline";

pub(super) fn prepare(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
) -> Result<Value, DispatchError> {
    let workspace_root = requested_workspace_root(runtime, action, input)?;
    let source = resolve_source_snapshot(runtime, action, input, &workspace_root)?;
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
    let active_preparations = active_task_pilot_preparations(runtime, action, &workspace_root)?;
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
                }).and_then(|task| {
                    if let Some(run_ids) = active_preparations.get(task_id) {
                        Err(action_failed(
                            action,
                            format!(
                                "task {task_id} is already prepared by active task-pilot run(s) {}; inspect or resume that durable run instead of starting duplicate pilot work",
                                run_ids.iter().cloned().collect::<Vec<_>>().join(", ")
                            ),
                        ))
                    } else {
                        Ok(task)
                    }
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
            } else if let Some(run_ids) = active_preparations.get(&task.id) {
                excluded.push(json!({
                    "task_id": task.id,
                    "reason": "active_pilot_prepared",
                    "prepared_by_run_ids": run_ids,
                }));
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
        "source": source.as_ref().map(SourceSnapshot::to_json).unwrap_or_else(|| {
            json!({
                "base_branch": requested_base_branch(runtime, input),
                "source_ref": Value::Null,
                "source_revision": Value::Null,
                "fast_forwarded": false,
            })
        }),
        "task_count": selected.len(),
        "task_ids": selected.iter().map(|task| task.id.clone()).collect::<Vec<_>>(),
        "tasks": task_snapshots,
        "partition_size": max_partition_size,
        "partition_count": partitions.len(),
        "partitions": partitions,
        "excluded": excluded,
    }))
}

fn active_task_pilot_preparations(
    runtime: &OrbitRuntime,
    action: &str,
    workspace_root: &Path,
) -> Result<BTreeMap<String, BTreeSet<String>>, DispatchError> {
    let mut prepared_by_task = BTreeMap::<String, BTreeSet<String>>::new();
    for state in [
        JobRunState::Pending,
        JobRunState::Running,
        JobRunState::Retrying,
    ] {
        let runs = runtime
            .stores()
            .jobs()
            .list_job_runs_filtered(&JobRunQuery {
                job_id: Some(TASK_PILOT_JOB_ID.to_string()),
                state: Some(state),
                terminal_only: false,
                created_since: None,
                limit: None,
                ..Default::default()
            })
            .map_err(|error| {
                action_failed(action, format!("list active task-pilot runs: {error}"))
            })?;
        for run in runs {
            let run = runtime.show_job_run(&run.run_id).map_err(|error| {
                action_failed(
                    action,
                    format!("reconcile active task-pilot run {}: {error}", run.run_id),
                )
            })?;
            if run.state.is_terminal() {
                continue;
            }
            let Some(state) = runtime.read_run_state(&run.run_id).map_err(|error| {
                action_failed(
                    action,
                    format!("read active task-pilot run {} state: {error}", run.run_id),
                )
            })?
            else {
                continue;
            };
            let Some(task_ids) = state.step_outputs.iter().find_map(|(step_index, output)| {
                (state.step_states.get(step_index) == Some(&JobRunState::Success))
                    .then(|| prepared_task_ids(output, workspace_root))
                    .flatten()
            }) else {
                continue;
            };
            for task_id in task_ids {
                prepared_by_task
                    .entry(task_id)
                    .or_default()
                    .insert(run.run_id.clone());
            }
        }
    }
    Ok(prepared_by_task)
}

fn prepared_task_ids(output: &Value, workspace_root: &Path) -> Option<Vec<String>> {
    let object = output.as_object()?;
    let prepared_workspace = object.get("workspace_path")?.as_str()?;
    if Path::new(prepared_workspace) != workspace_root
        || !object.get("partitions")?.is_array()
        || !object.get("tasks")?.is_array()
        || !matches!(object.get("mode")?.as_str(), Some("automatic" | "explicit"))
    {
        return None;
    }
    object
        .get("task_ids")?
        .as_array()?
        .iter()
        .map(|task_id| task_id.as_str().map(ToOwned::to_owned))
        .collect()
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
    source: Option<&SourceSnapshot>,
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
        if let Some(source) = source {
            validate_selector_at_source(action, task_id, &canonical, workspace_root, source)?;
        } else {
            if !exists_in_workspace(&canonical, workspace_root) {
                return Err(action_failed(
                    action,
                    format!(
                        "task {task_id} selector {selector:?} does not resolve to an existing in-workspace target"
                    ),
                ));
            }
            validate_selector_target_kind(action, task_id, &canonical, workspace_root)?;
        }
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

fn validate_selector_at_source(
    action: &str,
    task_id: &str,
    selector: &str,
    workspace_root: &Path,
    source: &SourceSnapshot,
) -> Result<(), DispatchError> {
    let anchor = anchor_path(selector).map_err(|error| {
        action_failed(
            action,
            format!("task {task_id} selector {selector:?} has no filesystem anchor: {error}"),
        )
    })?;
    let kind = source.path_kind(action, workspace_root, &anchor)?;
    let expected_dir = selector.starts_with("dir:");
    match (kind, expected_dir) {
        (GitPathKind::Tree, true) | (GitPathKind::Blob, false) => Ok(()),
        (GitPathKind::Missing, _) => Err(action_failed(
            action,
            format!(
                "task {task_id} selector {selector:?} does not resolve to an existing in-workspace target at source revision {} ({})",
                source.source_revision, source.source_ref
            ),
        )),
        _ => Err(action_failed(
            action,
            format!(
                "task {task_id} selector {selector:?} does not match the target's file/directory kind at source revision {} ({})",
                source.source_revision, source.source_ref
            ),
        )),
    }
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
