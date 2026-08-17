use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RecordError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    InvalidTransition(String),
}
