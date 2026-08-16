use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TelemetryError {
    #[error("{0}")]
    Invalid(String),
}
