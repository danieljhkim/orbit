use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WorkspaceError {
    #[error("{0}")]
    Invalid(String),
}
