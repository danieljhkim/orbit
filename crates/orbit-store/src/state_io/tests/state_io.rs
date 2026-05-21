// Migrated from state_io.rs per ORB-00231
use super::super::*;

use serde_json::json;

fn test_state() -> PipelineState {
    PipelineState::new("jrun-test".to_string(), "job-test".to_string(), json!({}))
}

#[test]
fn pipeline_state_waiting_reasons_round_trip_populated() {
    let mut state = test_state();
    state.set_waiting_reasons(
        Some(vec!["ORB-1".to_string(), "ORB-2".to_string()]),
        Some(vec!["file:src/lib.rs".to_string()]),
    );

    let encoded = serde_json::to_value(&state).expect("serialize state");
    assert_eq!(encoded["waiting_on_deps"], json!(["ORB-1", "ORB-2"]));
    assert_eq!(encoded["waiting_on_locks"], json!(["file:src/lib.rs"]));

    let decoded: PipelineState = serde_json::from_value(encoded).expect("deserialize state");
    assert_eq!(
        decoded.waiting_on_deps,
        Some(vec!["ORB-1".to_string(), "ORB-2".to_string()])
    );
    assert_eq!(
        decoded.waiting_on_locks,
        Some(vec!["file:src/lib.rs".to_string()])
    );
}

#[test]
fn pipeline_state_waiting_reasons_round_trip_empty_arrays() {
    let mut state = test_state();
    state.set_waiting_reasons(Some(Vec::new()), Some(Vec::new()));

    let encoded = serde_json::to_value(&state).expect("serialize state");
    assert_eq!(encoded["waiting_on_deps"], json!([]));
    assert_eq!(encoded["waiting_on_locks"], json!([]));

    let decoded: PipelineState = serde_json::from_value(encoded).expect("deserialize state");
    assert_eq!(decoded.waiting_on_deps, Some(Vec::new()));
    assert_eq!(decoded.waiting_on_locks, Some(Vec::new()));
}

#[test]
fn pipeline_state_waiting_reasons_round_trip_none_absent() {
    let state = test_state();

    let encoded = serde_json::to_value(&state).expect("serialize state");
    let object = encoded.as_object().expect("state object");
    assert!(!object.contains_key("waiting_on_deps"));
    assert!(!object.contains_key("waiting_on_locks"));

    let decoded: PipelineState = serde_json::from_value(encoded).expect("deserialize state");
    assert_eq!(decoded.waiting_on_deps, None);
    assert_eq!(decoded.waiting_on_locks, None);
}
