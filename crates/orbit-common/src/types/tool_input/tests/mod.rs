use serde_json::json;

use super::super::*;

#[test]
fn optional_string_list_accepts_scalar_string() {
    assert_eq!(
        optional_string_list_alias(&json!({"values":"one"}), &["values"]).unwrap(),
        Some(vec!["one".to_string()])
    );
}

#[test]
fn optional_string_list_preserves_array_behavior() {
    assert_eq!(
        optional_string_list_alias(&json!({"values":["one", "two"]}), &["values"]).unwrap(),
        Some(vec!["one".to_string(), "two".to_string()])
    );
}

#[test]
fn optional_string_list_rejects_non_string_shapes() {
    let error = optional_string_list_alias(&json!({"values":{"one":true}}), &["values"])
        .unwrap_err()
        .to_string();
    assert!(error.contains("`values` must be a string or array of strings"));
}

#[test]
fn optional_string_list_recovers_json_encoded_array() {
    assert_eq!(
        optional_string_list_alias(&json!({"values": "[\"a\",\"b\"]"}), &["values"]).unwrap(),
        Some(vec!["a".to_string(), "b".to_string()])
    );
}

#[test]
fn optional_string_list_recovers_json_encoded_array_with_whitespace() {
    assert_eq!(
        optional_string_list_alias(&json!({"values": "  [\"a\", \"b\"]  "}), &["values"]).unwrap(),
        Some(vec!["a".to_string(), "b".to_string()])
    );
}

#[test]
fn optional_string_list_recovers_single_encoded_array_element() {
    assert_eq!(
        optional_string_list_alias(&json!({"values": ["[\"a\",\"b\"]"]}), &["values"]).unwrap(),
        Some(vec!["a".to_string(), "b".to_string()])
    );
}

#[test]
fn optional_string_list_keeps_plain_string_with_brackets() {
    assert_eq!(
        optional_string_list_alias(&json!({"values": "[draft] note"}), &["values"]).unwrap(),
        Some(vec!["[draft] note".to_string()])
    );
}

#[test]
fn optional_string_list_falls_back_for_heterogeneous_json_arrays() {
    assert_eq!(
        optional_string_list_alias(&json!({"values": "[\"a\", 5]"}), &["values"]).unwrap(),
        Some(vec!["[\"a\", 5]".to_string()])
    );
}

#[test]
fn optional_string_list_falls_back_for_recovered_empty_strings() {
    assert_eq!(
        optional_string_list_alias(&json!({"values": "[\"a\", \"\"]"}), &["values"]).unwrap(),
        Some(vec!["[\"a\", \"\"]".to_string()])
    );
}

#[test]
fn optional_csv_or_string_list_recovers_json_encoded_selectors() {
    let recovered = optional_csv_or_string_list_alias(
        &json!({"context_files": "[\"file:src/lib.rs\", \"file:src/main.rs\"]"}),
        &["context_files"],
    )
    .unwrap();
    assert_eq!(
        recovered,
        Some(vec![
            "file:src/lib.rs".to_string(),
            "file:src/main.rs".to_string()
        ])
    );
}

#[test]
fn optional_csv_or_string_list_recovers_single_encoded_array_element() {
    let recovered = optional_csv_or_string_list_alias(
        &json!({"context_files": ["[\"file:src/lib.rs\", \"file:src/main.rs\"]"]}),
        &["context_files"],
    )
    .unwrap();
    assert_eq!(
        recovered,
        Some(vec![
            "file:src/lib.rs".to_string(),
            "file:src/main.rs".to_string()
        ])
    );
}
