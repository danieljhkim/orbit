use orbit_common::types::{OrbitError, TaskStatus};
use serde_json::{Value, json};

use crate::context::TaskHost;

use super::input::{input_string_field, required_input_string};

pub(super) fn update_task<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;

    // Tolerate missing status: when the upstream agent persisted status directly
    // to the task (via orbit.task.update) but crashed before returning structured
    // output, piped input will lack `status`. In that case, treat as a no-op
    // since the task already has the correct status.
    let status = match input_string_field(input, "status") {
        Some(raw) => raw
            .parse::<TaskStatus>()
            .map_err(|error| OrbitError::InvalidInput(format!("invalid input.status: {error}")))?,
        None => return Ok(json!({})),
    };

    // Idempotent: if task is already at the target status, skip the update.
    let task = host.get_task(task_id)?;
    if task.status == status {
        return Ok(json!({}));
    }

    let note = input_string_field(input, "note")
        .or_else(|| Some(format!("automation: update_task → {status}")));
    host.update_task_from_activity(task_id, status, None, None, note)?;
    Ok(json!({}))
}
