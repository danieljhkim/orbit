use std::cmp::Reverse;

use orbit_types::{OrbitError, Task, TaskPriority, TaskStatus};
use serde_json::{Value, json};

use crate::context::TaskReadHost;

use super::input::input_string_field;

pub(super) fn select_duel_task<H: TaskReadHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    if let Some(task_id) = input_string_field(input, "task_id") {
        return Ok(json!({
            "task_id": task_id.clone(),
            "task_ids": [task_id],
        }));
    }

    let mut tasks = host.list_tasks_filtered(Some(TaskStatus::Backlog), None, None, None)?;
    tasks.retain(|task| duel_batch_id(task).is_none());
    tasks.sort_by(|left, right| {
        (
            Reverse(duel_priority_rank(left.priority)),
            left.created_at,
            left.id.clone(),
        )
            .cmp(&(
                Reverse(duel_priority_rank(right.priority)),
                right.created_at,
                right.id.clone(),
            ))
    });

    let task_id = tasks.first().map(|task| task.id.clone()).ok_or_else(|| {
        OrbitError::InvalidInput("no duel-eligible backlog tasks found for auto-selection".into())
    })?;

    Ok(json!({
        "task_id": task_id.clone(),
        "task_ids": [task_id],
    }))
}

fn duel_batch_id(task: &Task) -> Option<&str> {
    task.batch_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn duel_priority_rank(priority: TaskPriority) -> u8 {
    match priority {
        TaskPriority::Low => 0,
        TaskPriority::Medium => 1,
        TaskPriority::High => 2,
        TaskPriority::Critical => 3,
    }
}
