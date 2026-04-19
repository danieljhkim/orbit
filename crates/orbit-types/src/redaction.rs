//! Re-exports of the generic redaction helpers from `orbit_common::redaction`,
//! plus the `OrbitError`-variant helper that stays here because it is coupled
//! to the error enum defined in this crate.
//!
//! Existing call sites (`orbit_types::redact_sensitive_env_*`) continue to
//! work unchanged.

pub use orbit_common::redaction::{
    is_sensitive_env_name, redact_home_dir, redact_sensitive_env_json, redact_sensitive_env_option,
    redact_sensitive_env_text,
};

use crate::OrbitError;

/// Apply env-value redaction to the message carried by any `OrbitError` variant.
pub fn redact_sensitive_env_error(error: OrbitError) -> OrbitError {
    match error {
        OrbitError::PolicyDenied(m) => OrbitError::PolicyDenied(redact_sensitive_env_text(&m)),
        OrbitError::ToolNotFound(m) => OrbitError::ToolNotFound(redact_sensitive_env_text(&m)),
        OrbitError::TaskNotFound(m) => OrbitError::TaskNotFound(redact_sensitive_env_text(&m)),
        OrbitError::TaskApprovalRequired(m) => {
            OrbitError::TaskApprovalRequired(redact_sensitive_env_text(&m))
        }
        OrbitError::SkillNotFound(m) => OrbitError::SkillNotFound(redact_sensitive_env_text(&m)),
        OrbitError::JobNotFound(m) => OrbitError::JobNotFound(redact_sensitive_env_text(&m)),
        OrbitError::JobRunNotFound(m) => OrbitError::JobRunNotFound(redact_sensitive_env_text(&m)),
        OrbitError::ActivityNotFound(m) => {
            OrbitError::ActivityNotFound(redact_sensitive_env_text(&m))
        }
        OrbitError::AgentSessionNotFound(m) => {
            OrbitError::AgentSessionNotFound(redact_sensitive_env_text(&m))
        }
        OrbitError::InvalidInput(m) => OrbitError::InvalidInput(redact_sensitive_env_text(&m)),
        OrbitError::SkillValidation(m) => {
            OrbitError::SkillValidation(redact_sensitive_env_text(&m))
        }
        OrbitError::JobValidation(m) => OrbitError::JobValidation(redact_sensitive_env_text(&m)),
        OrbitError::AgentProtocolViolation(m) => {
            OrbitError::AgentProtocolViolation(redact_sensitive_env_text(&m))
        }
        OrbitError::UnsupportedAgentProvider(m) => {
            OrbitError::UnsupportedAgentProvider(redact_sensitive_env_text(&m))
        }
        OrbitError::Execution(m) => OrbitError::Execution(redact_sensitive_env_text(&m)),
        OrbitError::Store(m) => OrbitError::Store(redact_sensitive_env_text(&m)),
        OrbitError::TaskStatusTransition(m) => {
            OrbitError::TaskStatusTransition(redact_sensitive_env_text(&m))
        }
        OrbitError::JobRunStateTransition(m) => {
            OrbitError::JobRunStateTransition(redact_sensitive_env_text(&m))
        }
        OrbitError::Io(m) => OrbitError::Io(redact_sensitive_env_text(&m)),
        OrbitError::WorkspaceNotFound(m) => {
            OrbitError::WorkspaceNotFound(redact_sensitive_env_text(&m))
        }
        OrbitError::WorkspaceError(m) => OrbitError::WorkspaceError(redact_sensitive_env_text(&m)),
    }
}
