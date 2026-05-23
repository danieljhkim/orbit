use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum KnowledgeErrorKind {
    #[serde(rename = "knowledge_invalid")]
    Invalid,
    #[serde(rename = "knowledge_unavailable")]
    Unavailable,
    #[serde(rename = "io_error")]
    Io,
}

impl std::fmt::Display for KnowledgeErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            Self::Invalid => "knowledge_invalid",
            Self::Unavailable => "knowledge_unavailable",
            Self::Io => "io_error",
        };
        f.write_str(kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Error)]
#[error("{kind}: {reason}")]
pub struct KnowledgeError {
    pub kind: KnowledgeErrorKind,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub did_you_mean: Vec<String>,
}

impl KnowledgeError {
    pub(crate) fn knowledge_unavailable(reason: impl Into<String>) -> Self {
        Self {
            kind: KnowledgeErrorKind::Unavailable,
            reason: reason.into(),
            did_you_mean: Vec::new(),
        }
    }

    pub(crate) fn invalid_data(reason: impl Into<String>) -> Self {
        Self {
            kind: KnowledgeErrorKind::Invalid,
            reason: reason.into(),
            did_you_mean: Vec::new(),
        }
    }

    pub(crate) fn invalid_data_with_suggestions(
        reason: impl Into<String>,
        did_you_mean: Vec<String>,
    ) -> Self {
        Self {
            kind: KnowledgeErrorKind::Invalid,
            reason: reason.into(),
            did_you_mean,
        }
    }

    pub(crate) fn io(reason: impl Into<String>) -> Self {
        Self {
            kind: KnowledgeErrorKind::Io,
            reason: reason.into(),
            did_you_mean: Vec::new(),
        }
    }
}
