use orbit_common::types::{ExternalRef, NO_DIFF_EXPECTED_TAG, OrbitError, TaskStatus};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::super::super::input::{input_string_field, required_input_string};
use super::super::handoff::{FailedHandoffPhase, load_handoff_context, record_failed_handoff};
use super::attribution::pr_review_attribution;

pub(in crate::executor::automation) fn pr_promote<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let context = load_handoff_context(host, input, "pr_promote")?;
    let no_diff_expected = input
        .get("no_diff_expected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let pr_number = if no_diff_expected {
        if context
            .tasks
            .iter()
            .any(|task| !task.tags.iter().any(|tag| tag == NO_DIFF_EXPECTED_TAG))
        {
            return Err(OrbitError::Execution(
                "pr_promote: no_diff_expected requires every task to carry the no-diff-expected tag"
                    .to_string(),
            ));
        }
        None
    } else {
        Some(required_input_string(input, "pr_number")?.to_string())
    };

    let mut performed_task_ids = Vec::new();
    let mut reused_task_ids = Vec::new();
    for task in &context.tasks {
        let has_pr = pr_number
            .as_deref()
            .is_none_or(|number| task.github_pr_number() == Some(number));
        if matches!(task.status, TaskStatus::Review | TaskStatus::Done) && has_pr {
            reused_task_ids.push(task.id.clone());
            continue;
        }

        let model = pr_review_attribution(host, task, &context.batch_id)?;
        let external_refs = match pr_number.as_deref() {
            Some(number) => vec![ExternalRef::github_pr(number.to_string())?],
            None => Vec::new(),
        };
        let update = TaskAutomationUpdate {
            status: (task.status != TaskStatus::Done).then_some(TaskStatus::Review),
            external_refs,
            model,
            ..TaskAutomationUpdate::default()
        };
        if let Err(error) = host.apply_task_automation_update(&task.id, update) {
            record_failed_handoff(host, &context, input, FailedHandoffPhase::Promote, &error)?;
            return Err(error);
        }
        performed_task_ids.push(task.id.clone());
    }

    let decision = if performed_task_ids.is_empty() {
        "reused"
    } else {
        "performed"
    };
    Ok(json!({
        "phase": "promote",
        "decision": decision,
        "performed_task_ids": performed_task_ids,
        "reused_task_ids": reused_task_ids,
        "pr_number": pr_number,
        "pr_url": input_string_field(input, "pr_url"),
        "no_diff_expected": no_diff_expected,
    }))
}
