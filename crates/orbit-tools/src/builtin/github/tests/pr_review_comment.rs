//! Tests for parse_review_comment_response helper.
//
// Migrated from nested `pr_review_comment/tests/` to sibling under `github/tests/`
// per ORB-00243 / docs/design-patterns/test_layout.md.

use serde_json::json;

use orbit_common::types::OrbitError;

use super::super::pr_review_comment::*;

#[test]
fn parse_review_comment_response_returns_id_from_valid_stdout() {
    let response = parse_review_comment_response(r#"{"id":67890,"body":"Looks good"}"#).unwrap();

    assert_eq!(response["id"], json!(67890));
    assert_eq!(response["commented"], json!(true));
}

#[test]
fn parse_review_comment_response_rejects_malformed_stdout() {
    let error = parse_review_comment_response("not json").unwrap_err();

    assert!(matches!(error, OrbitError::Execution(_)));
}

#[test]
fn parse_review_comment_response_rejects_empty_stdout() {
    let error = parse_review_comment_response("").unwrap_err();

    assert!(matches!(error, OrbitError::Execution(_)));
}

#[test]
fn parse_review_comment_response_rejects_object_without_id() {
    let error = parse_review_comment_response(r#"{"body":"Looks good"}"#).unwrap_err();

    assert!(matches!(error, OrbitError::Execution(_)));
}
