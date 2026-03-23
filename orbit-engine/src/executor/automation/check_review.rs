use orbit_types::OrbitError;
use serde_json::{Value, json};

use super::input::{canonicalize_existing_dir, required_input_string};
use super::review::resolve_review_decision;
use crate::context::TaskHost;

pub(super) fn check_review_decision<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let task = host.get_task(task_id)?;
    let repo_root = canonicalize_existing_dir(
        task.repo_root
            .as_deref()
            .or(task.workspace_path.as_deref())
            .ok_or_else(|| {
                OrbitError::InvalidInput(
                    "check_review_decision requires task.repo_root or task.workspace_path"
                        .to_string(),
                )
            })?,
        "repo_root",
    )?;
    let pr_number = task.pr_number.as_deref().ok_or_else(|| {
        OrbitError::InvalidInput("check_review_decision requires task.pr_number".to_string())
    })?;

    let decision = resolve_review_decision(&repo_root, pr_number)?;
    if decision == "APPROVED" {
        Ok(json!({ "review_decision": decision }))
    } else {
        Err(OrbitError::Execution(format!(
            "pull request '{pr_number}' is not approved (review_decision={decision})"
        )))
    }
}
