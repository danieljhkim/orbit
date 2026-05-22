#![allow(missing_docs)]

use crate::error::{KnowledgeError, KnowledgeErrorKind};

#[test]
fn knowledge_error_serializes_kind_as_stable_code() {
    let error = KnowledgeError {
        kind: KnowledgeErrorKind::Invalid,
        reason: "bad selector".to_string(),
        did_you_mean: Vec::new(),
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

#[test]
fn knowledge_error_serializes_suggestions_when_present() {
    let error = KnowledgeError {
        kind: KnowledgeErrorKind::Invalid,
        reason: "bad selector".to_string(),
        did_you_mean: vec!["symbol:src/lib.rs#good:method".to_string()],
    };

    let value = serde_json::to_value(error).expect("serialize knowledge error");

    assert_eq!(
        value,
        serde_json::json!({
            "kind": "knowledge_invalid",
            "reason": "bad selector",
            "did_you_mean": ["symbol:src/lib.rs#good:method"]
        })
    );
}
