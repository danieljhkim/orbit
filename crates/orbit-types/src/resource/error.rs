use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ResourceError {
    #[error("{0}")]
    Invalid(String),
}
