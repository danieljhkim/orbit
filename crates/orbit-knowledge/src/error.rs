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
}

impl KnowledgeError {
    pub(crate) fn knowledge_unavailable(reason: impl Into<String>) -> Self {
        Self {
            kind: KnowledgeErrorKind::Unavailable,
            reason: reason.into(),
        }
    }

    pub(crate) fn invalid_data(reason: impl Into<String>) -> Self {
        Self {
            kind: KnowledgeErrorKind::Invalid,
            reason: reason.into(),
        }
    }

    pub(crate) fn io(reason: impl Into<String>) -> Self {
        Self {
            kind: KnowledgeErrorKind::Io,
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{KnowledgeError, KnowledgeErrorKind};

    #[test]
    fn knowledge_error_serializes_kind_as_stable_code() {
        let error = KnowledgeError {
            kind: KnowledgeErrorKind::Invalid,
            reason: "bad selector".to_string(),
        };

        let value = serde_json::to_value(error).expect("serialize knowledge error");

        assert_eq!(
            value,
            serde_json::json!({
                "kind": "knowledge_invalid",
                "reason": "bad selector"
            })
        );
    }
}
