#![allow(missing_docs)]

use serde_json::{Map, Value, json};

use super::super::dispatcher::agent_loop_output_from_final_message;

#[test]
fn agent_loop_output_exposes_structured_final_message_fields() {
    let mut metadata = Map::new();
    metadata.insert(
        "final_message".to_string(),
        Value::String("raw".to_string()),
    );

    let output = agent_loop_output_from_final_message(
        r#"{"cycle_notes":"dispatched one","dispatched_run_ids":["jrun-1"]}"#,
        metadata,
    );

    assert_eq!(output["cycle_notes"], json!("dispatched one"));
    assert_eq!(output["dispatched_run_ids"], json!(["jrun-1"]));
    assert_eq!(output["final_message"], json!("raw"));
}

#[test]
fn agent_loop_output_unwraps_response_envelope_result() {
    let output = agent_loop_output_from_final_message(
        r#"{"schemaVersion":1,"status":"success","result":{"dispatched_run_ids":[]}}"#,
        Map::new(),
    );

    assert_eq!(output["dispatched_run_ids"], json!([]));
    assert!(output.get("schemaVersion").is_none());
}
