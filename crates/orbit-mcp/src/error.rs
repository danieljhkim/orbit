use orbit_common::types::{NotFoundKind, OrbitError};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};

/// Map an [`OrbitError`] from tool execution into an `isError: true` MCP
/// [`CallToolResult`] with a structured payload.
///
/// The payload always carries:
/// - `code`: a short, stable machine-readable classifier (e.g. `"not_found"`,
///   `"invalid_input"`). Callers match on this rather than the free-form text.
/// - `message`: the human-readable error message (the `Display` of the error).
pub(crate) fn tool_error_result(err: &OrbitError) -> CallToolResult {
    CallToolResult::structured_error(error_payload(err))
}

fn error_payload(err: &OrbitError) -> Value {
    let mut payload = json!({
        "code": error_code(err),
        "message": err.to_string(),
    });
    if let Some(did_you_mean) = err.did_you_mean()
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("did_you_mean".to_string(), json!(did_you_mean));
    }
    payload
}

fn error_code(err: &OrbitError) -> &'static str {
    match err {
        OrbitError::NotFound { kind, .. } => match kind {
            NotFoundKind::Tool => "tool_not_found",
            NotFoundKind::Task
            | NotFoundKind::Skill
            | NotFoundKind::Job
            | NotFoundKind::JobRun
            | NotFoundKind::Activity
            | NotFoundKind::Adr
            | NotFoundKind::DesignFeature
            | NotFoundKind::Learning
            | NotFoundKind::LearningComment
            | NotFoundKind::AgentSession
            | NotFoundKind::Workspace => "not_found",
        },
        OrbitError::CompanionNotInstalled(_) => "companion_not_installed",
        OrbitError::PolicyDenied(_) => "policy_denied",
        OrbitError::TaskApprovalRequired(_) => "approval_required",
        OrbitError::InvalidInput(_) | OrbitError::InvalidInputDiagnostic { .. } => "invalid_input",
        OrbitError::SkillValidation(_) | OrbitError::JobValidation(_) => "validation_failed",
        OrbitError::TaskStatusTransition(_)
        | OrbitError::JobRunStateTransition(_)
        | OrbitError::AdrInvalidTransition(_) => "invalid_transition",
        OrbitError::AgentProtocolViolation(_) => "agent_protocol_violation",
        OrbitError::UnsupportedAgentProvider(_) => "unsupported_provider",
        OrbitError::Execution(_) => "execution_failed",
        OrbitError::Store(_) => "store_error",
        OrbitError::WorkspaceError(_) => "workspace_error",
        OrbitError::Io(_) => "io_error",
        OrbitError::Migration(_) => "migration_failed",
    }
}
