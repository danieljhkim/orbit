//! Tests for parse_review_response helper.

use serde_json::json;

use super::super::*;

#[test]
fn parse_review_response_returns_id_from_valid_stdout() {
    let response = parse_review_response(r#"{"id":12345,"state":"APPROVED"}"#).unwrap();

    assert_eq!(response["id"], json!(12345));
    assert_eq!(response["reviewed"], json!(true));
}

#[test]
fn parse_review_response_rejects_malformed_stdout() {
    let error = parse_review_response("not json").unwrap_err();

    assert!(matches!(error, OrbitError::Execution(_)));
}

#[test]
fn parse_review_response_rejects_empty_stdout() {
    let error = parse_review_response("").unwrap_err();

    assert!(matches!(error, OrbitError::Execution(_)));
}

#[test]
fn parse_review_response_rejects_object_without_id() {
    let error = parse_review_response(r#"{"state":"APPROVED"}"#).unwrap_err();

    assert!(matches!(error, OrbitError::Execution(_)));
}
