use orbit_core::OrbitError;
use serde_json::{Value, json};

pub fn print(value: &Value) -> Result<(), OrbitError> {
    print_with_format(value, false)
}

pub fn print_pretty(value: &Value) -> Result<(), OrbitError> {
    print_with_format(value, true)
}

pub fn print_with_format(value: &Value, pretty: bool) -> Result<(), OrbitError> {
    println!("{}", render(value, pretty)?);
    Ok(())
}

pub fn render(value: &Value, pretty: bool) -> Result<String, OrbitError> {
    if pretty {
        serde_json::to_string_pretty(value).map_err(|e| OrbitError::Execution(e.to_string()))
    } else {
        serde_json::to_string(value).map_err(|e| OrbitError::Execution(e.to_string()))
    }
}

pub fn error_payload(error: &OrbitError) -> Value {
    json!({
        "error": error.to_string(),
        "code": error_code(error),
    })
}

fn error_code(error: &OrbitError) -> &'static str {
    match error {
        OrbitError::PolicyDenied(_) => "policy_denied",
        OrbitError::ToolNotFound(_) => "tool_not_found",
        OrbitError::TaskNotFound(_) => "task_not_found",
        OrbitError::TaskApprovalRequired(_) => "task_approval_required",
        OrbitError::SkillNotFound(_) => "skill_not_found",
        OrbitError::JobNotFound(_) => "job_not_found",
        OrbitError::JobRunNotFound(_) => "job_run_not_found",
        OrbitError::ActivityNotFound(_) => "activity_not_found",
        OrbitError::AgentSessionNotFound(_) => "agent_session_not_found",
        OrbitError::CompanionNotInstalled(_) => "companion_not_installed",
        OrbitError::InvalidInput(_) => "invalid_input",
        OrbitError::SkillValidation(_) => "skill_validation_failed",
        OrbitError::JobValidation(_) => "job_validation_failed",
        OrbitError::AgentProtocolViolation(_) => "agent_protocol_violation",
        OrbitError::UnsupportedAgentProvider(_) => "unsupported_agent_provider",
        OrbitError::Execution(_) => "execution_failed",
        OrbitError::Store(_) => "store_error",
        OrbitError::TaskStatusTransition(_) => "task_status_transition",
        OrbitError::JobRunStateTransition(_) => "job_run_state_transition",
        OrbitError::WorkspaceNotFound(_) => "workspace_not_found",
        OrbitError::WorkspaceError(_) => "workspace_error",
        OrbitError::Io(_) => "io_error",
        OrbitError::AdrNotFound(_) => "adr_not_found",
        OrbitError::AdrInvalidTransition(_) => "adr_invalid_transition",
        OrbitError::LearningNotFound(_) => "learning_not_found",
    }
}
