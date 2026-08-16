use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WorkflowError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    JobValidation(String),
    #[error("{0}")]
    SkillValidation(String),
}
