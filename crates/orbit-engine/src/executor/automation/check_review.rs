use orbit_types::OrbitError;
use serde_json::{Value, json};

use super::input::required_batch_id;
use super::review::normalize_review_decision;
use crate::context::TaskHost;

pub(super) fn check_task_value<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let field = input
        .get("field")
        .and_then(Value::as_str)
        .ok_or_else(|| OrbitError::InvalidInput("check_task_value requires 'field'".into()))?;
    let expected = input
        .get("expected")
        .and_then(Value::as_str)
        .ok_or_else(|| OrbitError::InvalidInput("check_task_value requires 'expected'".into()))?;
    let scope = input.get("scope").and_then(Value::as_str).unwrap_or("all");
    let normalize = input
        .get("normalize")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fail_on_mismatch = input
        .get("fail_on_mismatch")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let matches = match scope {
        "input" => {
            // Read directly from the current input (duel case: reads arbiter decision)
            let raw = input.get(field).and_then(Value::as_str).unwrap_or("");
            let value = if normalize {
                normalize_review_decision(raw)
            } else {
                raw.to_string()
            };
            value.eq_ignore_ascii_case(expected)
        }
        "all" | "any" => {
            let batch_id = required_batch_id(input, "check_task_value")?;
            let tasks = host.list_tasks_filtered(None, None, None, Some(batch_id))?;
            if tasks.is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "no tasks found for batch_id '{batch_id}'"
                )));
            }
            let check_fn = |task: &orbit_types::Task| -> bool {
                let raw = get_task_field_value(task, field);
                let value = if normalize {
                    normalize_review_decision(&raw)
                } else {
                    raw
                };
                value.eq_ignore_ascii_case(expected)
            };
            if scope == "all" {
                tasks.iter().all(check_fn)
            } else {
                tasks.iter().any(check_fn)
            }
        }
        other => {
            return Err(OrbitError::InvalidInput(format!(
                "check_task_value: unknown scope '{other}'; expected input, all, or any"
            )));
        }
    };

    if fail_on_mismatch && !matches {
        return Err(OrbitError::Execution(format!(
            "check_task_value: expected '{field}' to match '{expected}' for scope '{scope}'"
        )));
    }

    Ok(json!({
        "match": matches,
        "loop_exit": matches,
    }))
}

fn get_task_field_value(task: &orbit_types::Task, field: &str) -> String {
    match field {
        "pr_status" => task.pr_status.as_deref().unwrap_or("").to_string(),
        "status" => format!("{:?}", task.status),
        _other => String::new(),
    }
}
