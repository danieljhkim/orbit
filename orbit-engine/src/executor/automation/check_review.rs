use orbit_types::OrbitError;
use serde_json::{Value, json};

use super::input::required_input_string;
use super::review::normalize_review_decision;
use crate::context::TaskHost;

pub(super) fn check_review_decision<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let task = host.get_task(task_id)?;

    let pr_status = task.pr_status.as_deref().unwrap_or("none");
    let normalized = normalize_review_decision(pr_status);
    if normalized == "APPROVED" {
        Ok(json!({ "review_decision": normalized }))
    } else {
        let pr_number = task.pr_number.as_deref().unwrap_or("unknown");
        Err(OrbitError::Execution(format!(
            "pull request '{pr_number}' is not approved (pr_status={pr_status})"
        )))
    }
}
