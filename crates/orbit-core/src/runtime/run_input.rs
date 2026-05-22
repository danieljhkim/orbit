use serde_json::Value;

/// Extract the singular task id from run/activity input shapes that are meant
/// to identify exactly one task.
pub(crate) fn singular_task_id_from_input(input: &Value) -> Option<&str> {
    input
        .get("task_id")
        .and_then(Value::as_str)
        .and_then(non_empty)
        .or_else(|| {
            input
                .get("task")
                .and_then(|task| task.get("id"))
                .and_then(Value::as_str)
                .and_then(non_empty)
        })
        .or_else(|| {
            let items = input.get("task_ids")?.as_array()?;
            if items.len() == 1 {
                items.first()?.as_str().and_then(non_empty)
            } else {
                None
            }
        })
}

pub(crate) fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}
