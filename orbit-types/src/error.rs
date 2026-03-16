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
    #[error("agent session not found: {0}")]
    AgentSessionNotFound(String),
    #[error("identity not found: {0}")]
    IdentityNotFound(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("skill validation failed: {0}")]
    SkillValidation(String),
    #[error("identity validation failed: {0}")]
    IdentityValidation(String),
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
    #[error("io error: {0}")]
    Io(String),
}
