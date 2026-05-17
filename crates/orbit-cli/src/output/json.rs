use orbit_core::{NotFoundKind, OrbitError};
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
    let mut payload = json!({
        "error": error.to_string(),
        "code": error_code(error),
    });
    if let Some(did_you_mean) = error.did_you_mean()
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("did_you_mean".to_string(), json!(did_you_mean));
    }
    payload
}

fn error_code(error: &OrbitError) -> &'static str {
    match error {
        OrbitError::PolicyDenied(_) => "policy_denied",
        OrbitError::NotFound { kind, .. } => match kind {
            NotFoundKind::Tool => "tool_not_found",
            NotFoundKind::Task => "task_not_found",
            NotFoundKind::Skill => "skill_not_found",
            NotFoundKind::Job => "job_not_found",
            NotFoundKind::JobRun => "job_run_not_found",
            NotFoundKind::Activity => "activity_not_found",
            NotFoundKind::Adr => "adr_not_found",
            NotFoundKind::DesignFeature => "design_feature_not_found",
            NotFoundKind::Learning => "learning_not_found",
            NotFoundKind::LearningComment => "learning_comment_not_found",
            NotFoundKind::AgentSession => "agent_session_not_found",
            NotFoundKind::Workspace => "workspace_not_found",
        },
        OrbitError::TaskApprovalRequired(_) => "task_approval_required",
        OrbitError::CompanionNotInstalled(_) => "companion_not_installed",
        OrbitError::InvalidInput(_) | OrbitError::InvalidInputDiagnostic { .. } => "invalid_input",
        OrbitError::SkillValidation(_) => "skill_validation_failed",
        OrbitError::JobValidation(_) => "job_validation_failed",
        OrbitError::AgentProtocolViolation(_) => "agent_protocol_violation",
        OrbitError::UnsupportedAgentProvider(_) => "unsupported_agent_provider",
        OrbitError::Execution(_) => "execution_failed",
        OrbitError::Store(_) => "store_error",
        OrbitError::TaskStatusTransition(_) => "task_status_transition",
        OrbitError::JobRunStateTransition(_) => "job_run_state_transition",
        OrbitError::WorkspaceError(_) => "workspace_error",
        OrbitError::Io(_) => "io_error",
        OrbitError::AdrInvalidTransition(_) => "adr_invalid_transition",
        OrbitError::Migration(_) => "migration_failed",
    }
}
