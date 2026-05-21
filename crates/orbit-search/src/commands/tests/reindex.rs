//! Unit tests for `reindex` — sibling layout under commands/tests/.

use std::str::FromStr;

use super::super::reindex::{IndexKind, SemanticIndexParams, SemanticIndexResult};
use crate::vector::UpsertReport;
use serde_json::json;

#[test]
fn semantic_index_params_serde_defaults_to_tasks_at_runtime() {
    let empty: SemanticIndexParams = serde_json::from_str("{}").unwrap();
    assert_eq!(empty.kind, None);
    assert_eq!(empty.resolved_kind(), IndexKind::Tasks);

    let model_only: SemanticIndexParams = serde_json::from_str(r#"{"model":"bge-small"}"#).unwrap();
    assert_eq!(model_only.model.as_deref(), Some("bge-small"));
    assert_eq!(model_only.kind, None);
    assert_eq!(model_only.resolved_kind(), IndexKind::Tasks);

    let docs: SemanticIndexParams = serde_json::from_str(r#"{"kind":"docs"}"#).unwrap();
    assert_eq!(docs.kind, Some(IndexKind::Docs));
    assert_eq!(docs.resolved_kind(), IndexKind::Docs);

    let adrs: SemanticIndexParams = serde_json::from_str(r#"{"kind":"adrs"}"#).unwrap();
    assert_eq!(adrs.kind, Some(IndexKind::Adrs));
    assert_eq!(adrs.resolved_kind(), IndexKind::Adrs);

    let learnings: SemanticIndexParams = serde_json::from_str(r#"{"kind":"learnings"}"#).unwrap();
    assert_eq!(learnings.kind, Some(IndexKind::Learnings));
    assert_eq!(learnings.resolved_kind(), IndexKind::Learnings);

    let all: SemanticIndexParams = serde_json::from_str(r#"{"kind":"all"}"#).unwrap();
    assert_eq!(all.kind, Some(IndexKind::All));
    assert_eq!(all.resolved_kind(), IndexKind::All);
}

#[test]
fn semantic_index_kind_rejects_singular_learning() {
    let error = IndexKind::from_str("learning").expect_err("singular kind should fail");

    assert!(error.to_string().contains("`learning`"));
    assert!(error.to_string().contains("learnings"));
}

#[test]
fn semantic_index_kind_rejects_singular_adr() {
    let error = IndexKind::from_str("adr").expect_err("singular kind should fail");

    assert!(error.to_string().contains("`adr`"));
    assert!(error.to_string().contains("adrs"));
}

#[test]
fn tasks_variant_serializes_like_legacy_reindex_result() {
    let result = SemanticIndexResult::Tasks {
        model_id: "bge-small-en-v1.5".to_string(),
        report: UpsertReport {
            embedded_chunks: 7,
            skipped_fields: 2,
        },
    };

    let expected = json!({
            "model_id": "bge-small-en-v1.5",
            "report": {
                "embedded_chunks": 7,
                "skipped_fields": 2
            }
    });
    assert_eq!(serde_json::to_value(&result).unwrap(), expected);
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        r#"{"model_id":"bge-small-en-v1.5","report":{"embedded_chunks":7,"skipped_fields":2}}"#
    );
}
