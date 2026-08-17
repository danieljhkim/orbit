use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IdentityError {
    #[error("{0}")]
    Invalid(String),
    #[error("{message}")]
    InvalidWithSuggestions {
        message: String,
        did_you_mean: Vec<String>,
    },
}

impl IdentityError {
    pub fn invalid_input_with_suggestions(
        message: impl Into<String>,
        did_you_mean: Vec<String>,
    ) -> Self {
        if did_you_mean.is_empty() {
            Self::Invalid(message.into())
        } else {
            Self::InvalidWithSuggestions {
                message: message.into(),
                did_you_mean,
            }
        }
    }
}
