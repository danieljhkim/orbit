use chrono::Utc;
use orbit_common::types::{
    OrbitError, Task, TaskComment, TaskHistoryEntry, normalize_attribution_label,
    normalize_optional_attribution_label,
};

pub(crate) const SYSTEM_ACTOR_LABEL: &str = "system";

pub(super) fn build_task_comments(
    message: Option<String>,
    by: &str,
) -> Result<Vec<TaskComment>, OrbitError> {
    let Some(message) = message else {
        return Ok(Vec::new());
    };
    let message = message.trim();
    if message.is_empty() {
        return Err(OrbitError::InvalidInput(
            "task comment must not be empty".to_string(),
        ));
    }
    let by = by.trim();
    if by.is_empty() {
        return Err(OrbitError::InvalidInput(
            "task comment author must not be empty".to_string(),
        ));
    }

    Ok(vec![TaskComment {
        at: Utc::now(),
        by: by.to_string(),
        message: message.to_string(),
    }])
}

pub(super) fn task_comment_history_entries(comments: &[TaskComment]) -> Vec<TaskHistoryEntry> {
    comments
        .iter()
        .map(|comment| TaskHistoryEntry {
            at: comment.at,
            by: comment.by.clone(),
            event: "commented".to_string(),
            note: None,
            from_status: None,
            to_status: None,
        })
        .collect()
}

pub(super) fn effective_actor_label(
    default_label: &str,
    agent: Option<&str>,
    model: Option<&str>,
) -> String {
    let label = match (agent, model) {
        (_, Some(model)) => model.to_string(),
        (Some(agent), None) => agent.to_string(),
        (None, None) => default_label.to_string(),
    };
    normalize_attribution_label(&label, model)
}

pub(super) fn implementation_label(
    task: &Task,
    actor_label: &str,
    explicit_model: Option<&str>,
) -> Option<String> {
    if let Some(existing) = task.implemented_by.as_deref() {
        return normalize_optional_attribution_label(Some(existing), None);
    }

    normalize_optional_attribution_label(
        explicit_model.or((!actor_label.trim().is_empty()).then_some(actor_label)),
        explicit_model,
    )
}

pub(super) fn authored_role_value(content: &str, actor_label: &str) -> Option<String> {
    if content.trim().is_empty() {
        None
    } else {
        Some(actor_label.to_string())
    }
}
