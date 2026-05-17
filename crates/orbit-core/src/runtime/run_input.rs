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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::singular_task_id_from_input;

    #[test]
    fn singular_task_id_accepts_single_entry_task_ids() {
        let input = json!({ "task_ids": [" ORB-00073 "] });

        assert_eq!(singular_task_id_from_input(&input), Some("ORB-00073"));
    }

    #[test]
    fn singular_task_id_rejects_multi_task_input() {
        let input = json!({ "task_ids": ["ORB-00073", "ORB-00078"] });

        assert_eq!(singular_task_id_from_input(&input), None);
    }
}
