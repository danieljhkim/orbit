use thiserror::Error;

#[derive(Debug, Error)]
pub enum OrbitError {
    #[error("policy denied: {0}")]
    PolicyDenied(String),
    #[error("tool not found: {0}")]
    ToolNotFound(String),
    #[error("task not found: {0}")]
    TaskNotFound(String),
    #[error("task requires approval: {0}")]
    TaskApprovalRequired(String),
    #[error("skill not found: {0}")]
    SkillNotFound(String),
    #[error("job not found: {0}")]
    JobNotFound(String),
    #[error("job run not found: {0}")]
    JobRunNotFound(String),
    #[error("activity not found: {0}")]
    ActivityNotFound(String),
    #[error("Invalid ADR status transition: {0}")]
    AdrInvalidTransition(String),
    #[error("ADR not found: {0}")]
    AdrNotFound(String),
    #[error("learning not found: {0}")]
    LearningNotFound(String),
    #[error("agent session not found: {0}")]
    AgentSessionNotFound(String),
    #[error("companion not installed: {0}")]
    CompanionNotInstalled(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("skill validation failed: {0}")]
    SkillValidation(String),
    #[error("job validation failed: {0}")]
    JobValidation(String),
    #[error("agent protocol violation: {0}")]
    AgentProtocolViolation(String),
    #[error("unsupported agent provider: {0}")]
    UnsupportedAgentProvider(String),
    #[error("execution failed: {0}")]
    Execution(String),
    #[error("store error: {0}")]
    Store(String),
    #[error("invalid task status transition: {0}")]
    TaskStatusTransition(String),
    #[error("invalid job run state transition: {0}")]
    JobRunStateTransition(String),
    #[error("workspace not found: {0}")]
    WorkspaceNotFound(String),
    #[error("workspace error: {0}")]
    WorkspaceError(String),
    #[error("io error: {0}")]
    Io(String),
}

impl From<std::io::Error> for OrbitError {
    fn from(err: std::io::Error) -> Self {
        OrbitError::Io(err.to_string())
    }
}
