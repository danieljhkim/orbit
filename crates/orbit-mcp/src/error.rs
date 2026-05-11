use orbit_common::types::OrbitError;
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
    json!({
        "code": error_code(err),
        "message": err.to_string(),
    })
}

fn error_code(err: &OrbitError) -> &'static str {
    match err {
        OrbitError::ToolNotFound(_) => "tool_not_found",
        OrbitError::TaskNotFound(_)
        | OrbitError::SkillNotFound(_)
        | OrbitError::JobNotFound(_)
        | OrbitError::JobRunNotFound(_)
        | OrbitError::ActivityNotFound(_)
        | OrbitError::AgentSessionNotFound(_)
        | OrbitError::WorkspaceNotFound(_)
        | OrbitError::AdrNotFound(_)
        | OrbitError::LearningNotFound(_) => "not_found",
        OrbitError::CompanionNotInstalled(_) => "companion_not_installed",
        OrbitError::PolicyDenied(_) => "policy_denied",
        OrbitError::TaskApprovalRequired(_) => "approval_required",
        OrbitError::InvalidInput(_) => "invalid_input",
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
    }
}
