use std::cmp::Reverse;
use std::collections::HashSet;

use chrono::Utc;
use orbit_common::types::{
    OrbitError, Task, TaskComment, TaskComplexity, TaskPriority, TaskStatus,
};
use orbit_common::utility::selector::shared_anchor_prefix_depth;
use serde_json::{Value, json};

use crate::context::{TaskAutomationUpdate, TaskHost};

use super::super::input::required_input_string;
use super::parallel::{parse_parallelism, tasks_conflict};

const SYSTEM_ACTOR_LABEL: &str = "system";

pub(in crate::executor::automation) fn dispatch_batch<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let run_id = required_input_string(input, "run_id")?;
    let parallelism = parse_parallelism(input)?;
    let in_progress =
        host.list_tasks_filtered(Some(TaskStatus::InProgress), None, None, None, None, None)?;
    let occupied = collect_occupied_contexts(&in_progress);
    let selected = match parse_task_ids(input)? {
        Some(task_ids) => load_explicit_batch(host, &task_ids, parallelism, &occupied)?,
        None => select_backlog_batch(host, parallelism, &occupied)?,
    };
    let rationale = batch_rationale(input.get("task_ids").is_some(), &selected);
    let selected = claim_selected_tasks(host, &selected, run_id, rationale)?;

    Ok(json!({
        "batch_id": run_id,
        "task_ids": selected.iter().map(|task| task.id.to_string()).collect::<Vec<_>>(),
        "batch_size": selected.len(),
    }))
}

fn parse_task_ids(input: &Value) -> Result<Option<Vec<String>>, OrbitError> {
    let Some(raw_ids) = input.get("task_ids") else {
        return Ok(None);
    };
    let ids = raw_ids.as_array().ok_or_else(|| {
        OrbitError::InvalidInput("task_ids must be an array of task IDs".to_string())
    })?;
    if ids.is_empty() {
        return Err(OrbitError::InvalidInput(
            "task_ids must not be empty when provided".to_string(),
        ));
    }

    let mut seen = HashSet::new();
    let mut parsed = Vec::with_capacity(ids.len());
    for value in ids {
        let task_id = value
            .as_str()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                OrbitError::InvalidInput("task_ids entries must be non-empty strings".to_string())
            })?;
        if !seen.insert(task_id.to_string()) {
            return Err(OrbitError::InvalidInput(format!(
                "duplicate task id '{task_id}' in input.task_ids"
            )));
        }
        parsed.push(task_id.to_string());
    }
    Ok(Some(parsed))
}

fn load_explicit_batch<H: TaskHost + ?Sized>(
    host: &H,
    task_ids: &[String],
    parallelism: usize,
    occupied: &[String],
) -> Result<Vec<Task>, OrbitError> {
    if task_ids.len() > parallelism {
        return Err(OrbitError::InvalidInput(format!(
            "explicit task_ids batch of {} exceeds parallelism {}",
            task_ids.len(),
            parallelism
        )));
    }

    let mut tasks = Vec::with_capacity(task_ids.len());
    for task_id in task_ids {
        let task = host.get_task(task_id)?;
        ensure_backlog_status(&task)?;
        ensure_unbatched(&task)?;
        ensure_not_occupied(&task, occupied)?;
        tasks.push(task);
    }
    ensure_batch_is_conflict_free(&tasks)?;
    Ok(tasks)
}

fn select_backlog_batch<H: TaskHost + ?Sized>(
    host: &H,
    parallelism: usize,
    occupied: &[String],
) -> Result<Vec<Task>, OrbitError> {
    let mut backlog = host
        .list_tasks_filtered(Some(TaskStatus::Backlog), None, None, None, None, None)?
        .into_iter()
        .filter(|task| task_batch_id(task).is_none())
        .filter(|task| !task_is_occupied(task, occupied))
        .collect::<Vec<_>>();
    if backlog.is_empty() || parallelism == 0 {
        return Ok(Vec::new());
    }

    backlog.sort_by_key(|task| {
        (
            Reverse(priority_rank(task.priority)),
            task.created_at,
            task.id.to_string(),
        )
    });

    let seed = backlog.remove(0);
    let mut selected = vec![seed.clone()];
    if parallelism == 1 || task_prefers_single_batch(&seed) {
        return Ok(selected);
    }

    backlog.sort_by_key(|task| {
        (
            Reverse(relatedness_score(&seed, task)),
            complexity_score(task),
            task.created_at,
            task.id.to_string(),
        )
    });

    for candidate in backlog {
        if selected.len() >= parallelism {
            break;
        }
        if relatedness_score(&seed, &candidate) == 0 {
            continue;
        }
        if selected
            .iter()
            .any(|existing| tasks_conflict(&existing.context_files, &candidate.context_files))
        {
            continue;
        }
        selected.push(candidate);
    }

    Ok(selected)
}

fn claim_selected_tasks<H: TaskHost + ?Sized>(
    host: &H,
    selected: &[Task],
    run_id: &str,
    rationale: &str,
) -> Result<Vec<Task>, OrbitError> {
    let mut claimed = Vec::with_capacity(selected.len());
    for task in selected {
        let refreshed = host.get_task(task.id.as_ref())?;
        ensure_backlog_status(&refreshed)?;
        ensure_unbatched(&refreshed)?;

        host.start_task(task.id.as_ref(), None, None)
            .map_err(|error| {
                OrbitError::Execution(format!(
                    "failed to claim task '{}' for batch '{}': {error}",
                    task.id, run_id
                ))
            })?;
        tag_task(host, &refreshed, run_id, rationale)?;
        claimed.push(host.get_task(task.id.as_ref())?);
    }
    Ok(claimed)
}

fn ensure_backlog_status(task: &Task) -> Result<(), OrbitError> {
    if task.status == TaskStatus::Backlog {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "task '{}' must be in backlog status, found '{}'",
        task.id, task.status
    )))
}

fn ensure_batch_is_conflict_free(tasks: &[Task]) -> Result<(), OrbitError> {
    for (index, left) in tasks.iter().enumerate() {
        for right in &tasks[index + 1..] {
            if tasks_conflict(&left.context_files, &right.context_files) {
                return Err(OrbitError::InvalidInput(format!(
                    "dispatch_batch selected conflicting tasks '{}' and '{}'",
                    left.id, right.id
                )));
            }
        }
    }
    Ok(())
}

fn ensure_unbatched(task: &Task) -> Result<(), OrbitError> {
    if task_batch_id(task).is_none() {
        return Ok(());
    }
    Err(OrbitError::InvalidInput(format!(
        "task '{}' is already assigned to batch '{}'",
        task.id,
        task_batch_id(task).unwrap_or_default()
    )))
}

fn task_batch_id(task: &Task) -> Option<&str> {
    task.batch_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn ensure_not_occupied(task: &Task, occupied: &[String]) -> Result<(), OrbitError> {
    if task_is_occupied(task, occupied) {
        return Err(OrbitError::InvalidInput(format!(
            "task '{}' conflicts with an in-progress task and cannot join a concurrent batch",
            task.id
        )));
    }
    Ok(())
}

fn tag_task<H: TaskHost + ?Sized>(
    host: &H,
    task: &Task,
    run_id: &str,
    rationale: &str,
) -> Result<(), OrbitError> {
    host.apply_task_automation_update(
        task.id.as_ref(),
        TaskAutomationUpdate {
            batch_id: Some(run_id.to_string()),
            append_comments: vec![TaskComment {
                at: Utc::now(),
                by: SYSTEM_ACTOR_LABEL.to_string(),
                message: format!("Batch dispatched: {rationale}"),
            }],
            ..TaskAutomationUpdate::default()
        },
    )
}

fn batch_rationale(explicit_ids: bool, selected: &[Task]) -> &'static str {
    if explicit_ids {
        "explicit task_ids input honored"
    } else if selected.len() <= 1 {
        "single focused task selected by deterministic batch heuristics"
    } else {
        "related non-conflicting backlog tasks grouped for planning"
    }
}

fn collect_occupied_contexts(tasks: &[Task]) -> Vec<String> {
    tasks
        .iter()
        .flat_map(|task| task.context_files.iter().cloned())
        .collect()
}

fn task_is_occupied(task: &Task, occupied: &[String]) -> bool {
    !occupied.is_empty() && tasks_conflict(&task.context_files, occupied)
}

fn task_prefers_single_batch(task: &Task) -> bool {
    matches!(task.complexity, Some(TaskComplexity::Hard))
        || task.context_files.len() >= 4
        || task.acceptance_criteria.len() >= 4
        || complexity_score(task) >= 6
}

fn complexity_score(task: &Task) -> usize {
    task.context_files.len().saturating_mul(2) + task.acceptance_criteria.len()
}

fn relatedness_score(seed: &Task, candidate: &Task) -> usize {
    let parent_bonus =
        usize::from(seed.parent_id.is_some() && seed.parent_id == candidate.parent_id);
    let path_bonus = shared_path_prefix_score(seed, candidate);
    parent_bonus.saturating_mul(10) + path_bonus
}

fn shared_path_prefix_score(left: &Task, right: &Task) -> usize {
    left.context_files
        .iter()
        .map(String::as_str)
        .flat_map(|left_path| {
            right
                .context_files
                .iter()
                .map(String::as_str)
                .map(move |right_path| shared_prefix_depth(left_path, right_path))
        })
        .max()
        .unwrap_or(0)
}

fn shared_prefix_depth(left: &str, right: &str) -> usize {
    shared_anchor_prefix_depth(left, right)
}

fn priority_rank(priority: TaskPriority) -> u8 {
    match priority {
        TaskPriority::Low => 0,
        TaskPriority::Medium => 1,
        TaskPriority::High => 2,
        TaskPriority::Critical => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::shared_prefix_depth;

    #[test]
    fn shared_prefix_depth_uses_selector_anchors() {
        assert_eq!(
            shared_prefix_depth("symbol:src/lib.rs#run:function", "dir:src"),
            1
        );
        assert_eq!(
            shared_prefix_depth("file:src/a.rs", "file:src/nested/b.rs"),
            1
        );
        assert_eq!(shared_prefix_depth("file:src/a.rs", "file:tests/a.rs"), 0);
    }
}
