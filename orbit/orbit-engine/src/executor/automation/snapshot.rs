use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::context::TaskHost;

use super::input::required_input_string;

pub(super) fn snapshot_batch_state<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let run_id = required_input_string(input, "run_id")?;
    let tasks = host.list_tasks_filtered(None, None, None, Some(run_id))?;

    let task_ids: Vec<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
    let task_objects: Vec<Value> = tasks
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "title": t.title,
                "status": t.status,
                "context_files": t.context_files,
            })
        })
        .collect();

    Ok(json!({
        "task_ids": task_ids,
        "task_count": tasks.len(),
        "tasks": task_objects,
    }))
}
