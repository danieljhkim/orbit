use chrono::Utc;
use orbit_types::{OrbitError, TaskComment};

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

pub(super) fn effective_actor_label(
    default_label: &str,
    agent: Option<&str>,
    model: Option<&str>,
) -> String {
    match (agent, model) {
        (_, Some(model)) => model.to_string(),
        (Some(agent), None) => agent.to_string(),
        (None, None) => default_label.to_string(),
    }
}

pub(super) fn authored_role_value(content: &str, actor_label: &str) -> Option<String> {
    if content.trim().is_empty() {
        None
    } else {
        Some(actor_label.to_string())
    }
}
