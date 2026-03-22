use orbit_types::{OrbitError, TaskStatus};
use serde_json::{Value, json};

use crate::context::TaskHost;

use super::input::{input_string_field, required_input_string};

pub(super) fn update_task<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let status = required_input_string(input, "status")?
        .parse::<TaskStatus>()
        .map_err(|error| OrbitError::InvalidInput(format!("invalid input.status: {error}")))?;

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
