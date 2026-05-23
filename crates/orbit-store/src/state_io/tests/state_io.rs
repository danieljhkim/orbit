// Migrated from state_io.rs per ORB-00231
use super::super::*;

use serde_json::json;
use tempfile::tempdir;

fn test_state() -> PipelineState {
    PipelineState::new("jrun-test".to_string(), "job-test".to_string(), json!({}))
}

fn create_run_dir(orbit_root: &std::path::Path, job_id: &str, run_id: &str) -> std::path::PathBuf {
    let run_dir = orbit_root
        .join("state")
        .join("job-runs")
        .join(job_id)
        .join(run_id);
    std::fs::create_dir_all(&run_dir).expect("create run dir");
    run_dir
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

#[test]
fn resolve_active_run_state_dir_rejects_traversal_run_id() {
    let temp = tempdir().expect("tempdir");
    let orbit_root = temp.path().join(".orbit");
    create_run_dir(&orbit_root, "job-test", "jrun-current");

    let error = resolve_active_run_state_dir(&orbit_root, "../jrun-current")
        .unwrap_err()
        .to_string();

    assert!(error.contains("single path component"), "{error}");
}

#[test]
fn validate_active_run_state_dir_rejects_absolute_path_outside_workspace() {
    let current = tempdir().expect("current tempdir");
    let other = tempdir().expect("other tempdir");
    let current_orbit_root = current.path().join(".orbit");
    let other_orbit_root = other.path().join(".orbit");
    create_run_dir(&current_orbit_root, "job-test", "jrun-current");
    let other_run_dir = create_run_dir(&other_orbit_root, "job-test", "jrun-current");

    let error = validate_active_run_state_dir(&current_orbit_root, &other_run_dir, "jrun-current")
        .unwrap_err()
        .to_string();

    assert!(error.contains("outside"), "{error}");
}

#[test]
fn validate_active_run_state_dir_rejects_traversal_state_dir() {
    let temp = tempdir().expect("tempdir");
    let orbit_root = temp.path().join(".orbit");
    create_run_dir(&orbit_root, "job-test", "jrun-current");
    let traversal = orbit_root
        .join("state")
        .join("job-runs")
        .join("job-test")
        .join("..")
        .join("job-test")
        .join("jrun-current");

    let error = validate_active_run_state_dir(&orbit_root, &traversal, "jrun-current")
        .unwrap_err()
        .to_string();

    assert!(error.contains("must not contain `..`"), "{error}");
}

#[test]
fn validate_active_run_state_dir_accepts_current_run() {
    let temp = tempdir().expect("tempdir");
    let orbit_root = temp.path().join(".orbit");
    let run_dir = create_run_dir(&orbit_root, "job-test", "jrun-current");

    let validated =
        validate_active_run_state_dir(&orbit_root, &run_dir, "jrun-current").expect("valid dir");

    assert_eq!(
        validated,
        run_dir.canonicalize().expect("canonical run dir")
    );
}
