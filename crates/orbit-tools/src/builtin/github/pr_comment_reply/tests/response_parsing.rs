//! Tests for parse_reply_response helper.

use serde_json::json;

use super::super::*;

#[test]
fn parse_reply_response_returns_id_from_valid_stdout() {
    let response = parse_reply_response(r#"{"id":24680,"body":"Done"}"#).unwrap();

    assert_eq!(response["id"], json!(24680));
    assert_eq!(response["replied"], json!(true));
}

#[test]
fn parse_reply_response_rejects_malformed_stdout() {
    let error = parse_reply_response("not json").unwrap_err();

    assert!(matches!(error, OrbitError::Execution(_)));
}

#[test]
fn parse_reply_response_rejects_empty_stdout() {
    let error = parse_reply_response("").unwrap_err();

    assert!(matches!(error, OrbitError::Execution(_)));
}

#[test]
fn parse_reply_response_rejects_object_without_id() {
    let error = parse_reply_response(r#"{"body":"Done"}"#).unwrap_err();

    assert!(matches!(error, OrbitError::Execution(_)));
}
