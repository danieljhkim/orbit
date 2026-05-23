use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotFoundKind {
    Tool,
    Task,
    Skill,
    Job,
    JobRun,
    Activity,
    Adr,
    DesignFeature,
    Learning,
    LearningComment,
    AgentSession,
    Workspace,
}

impl std::fmt::Display for NotFoundKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            Self::Tool => "tool",
            Self::Task => "task",
            Self::Skill => "skill",
            Self::Job => "job",
            Self::JobRun => "job run",
            Self::Activity => "activity",
            Self::Adr => "ADR",
            Self::DesignFeature => "design feature",
            Self::Learning => "learning",
            Self::LearningComment => "learning comment",
            Self::AgentSession => "agent session",
            Self::Workspace => "workspace",
        };
        f.write_str(kind)
    }
}

#[derive(Debug, Error, Serialize)]
pub enum OrbitError {
    #[error("policy denied: {0}")]
    PolicyDenied(String),
    #[error("{kind} not found: {id}")]
    NotFound { kind: NotFoundKind, id: String },
    #[error("task requires approval: {0}")]
    TaskApprovalRequired(String),
    #[error("Invalid ADR status transition: {0}")]
    AdrInvalidTransition(String),
    #[error("companion not installed: {0}")]
    CompanionNotInstalled(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("sensitive input rejected for `{field}`: {reason}")]
    SensitiveInput { field: String, reason: String },
    #[error("invalid input: {message}")]
    InvalidInputDiagnostic {
        message: String,
        did_you_mean: Vec<String>,
    },
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
    #[error("workspace error: {0}")]
    WorkspaceError(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("schema migration failed: {0}")]
    Migration(String),
}

impl OrbitError {
    pub fn not_found(kind: NotFoundKind, id: impl Into<String>) -> Self {
        Self::NotFound {
            kind,
            id: id.into(),
        }
    }

    pub fn invalid_input_with_suggestions(
        message: impl Into<String>,
        did_you_mean: Vec<String>,
    ) -> Self {
        if did_you_mean.is_empty() {
            Self::InvalidInput(message.into())
        } else {
            Self::InvalidInputDiagnostic {
                message: message.into(),
                did_you_mean,
            }
        }
    }

    pub fn did_you_mean(&self) -> Option<&[String]> {
        match self {
            Self::InvalidInputDiagnostic { did_you_mean, .. } if !did_you_mean.is_empty() => {
                Some(did_you_mean)
            }
            _ => None,
        }
    }
}

impl From<std::io::Error> for OrbitError {
    fn from(err: std::io::Error) -> Self {
        OrbitError::Io(err.to_string())
    }
}
