use orbit_types::OrbitError;
use serde_json::{Value, json};

use super::review::normalize_review_decision;
use crate::context::TaskHost;

pub(super) fn check_batch_review_decision<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = input
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            OrbitError::InvalidInput(
                "check_batch_review_decision requires input.run_id".to_string(),
            )
        })?;

    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id))?;

    for task in &batch_tasks {
        if task.pr_number.is_none() {
            continue;
        }

        let pr_status = task.pr_status.as_deref().unwrap_or("none");
        let normalized = normalize_review_decision(pr_status);
        if normalized != "APPROVED" {
            return Err(OrbitError::Execution(format!(
                "task '{}' is not approved (pr_status={pr_status})",
                task.id
            )));
        }
    }

    Ok(json!({ "review_decision": "APPROVED", "loop_exit": true }))
}
