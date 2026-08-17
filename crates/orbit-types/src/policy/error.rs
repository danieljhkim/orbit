use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PolicyError {
    #[error("{0}")]
    Invalid(String),
}
