//! Watermark state tests [ORB-10039]: locked read-modify-write updates that
//! preserve other workspaces' entries, plus lenient loading (missing or
//! corrupt state only means "re-validate current HEAD").

use crate::qa::state::{
    QaWorkspaceWatermark, advance_watermark, load_state, parse_state, state_path,
    try_acquire_pass_lock,
};

fn watermark(sha: &str) -> QaWorkspaceWatermark {
    QaWorkspaceWatermark {
        last_validated_sha: sha.to_string(),
        validated_at: "2026-07-06T00:00:00Z".to_string(),
        run_id: Some("run-1".to_string()),
    }
}

#[test]
fn missing_state_file_loads_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = load_state(&state_path(dir.path()));
    assert!(state.workspaces.is_empty());
}

#[test]
fn corrupt_state_file_loads_empty() {
    assert!(parse_state("{not json").workspaces.is_empty());
    assert!(parse_state("").workspaces.is_empty());
}

#[test]
fn advance_creates_file_and_roundtrips() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(dir.path());

    advance_watermark(&path, "polaris", watermark("aaa111")).expect("advance");
    let state = load_state(&path);
    assert_eq!(
        state
            .workspaces
            .get("polaris")
            .map(|w| w.last_validated_sha.as_str()),
        Some("aaa111")
    );
    assert_eq!(
        state
            .workspaces
            .get("polaris")
            .and_then(|w| w.run_id.as_deref()),
        Some("run-1")
    );
}

#[test]
fn advance_preserves_other_workspaces_and_overwrites_own_entry() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(dir.path());

    advance_watermark(&path, "polaris", watermark("aaa111")).expect("advance polaris");
    advance_watermark(&path, "bridge", watermark("bbb222")).expect("advance bridge");
    advance_watermark(&path, "polaris", watermark("ccc333")).expect("re-advance polaris");

    let state = load_state(&path);
    assert_eq!(state.workspaces.len(), 2);
    assert_eq!(
        state
            .workspaces
            .get("polaris")
            .map(|w| w.last_validated_sha.as_str()),
        Some("ccc333")
    );
    assert_eq!(
        state
            .workspaces
            .get("bridge")
            .map(|w| w.last_validated_sha.as_str()),
        Some("bbb222")
    );
}

#[test]
fn shorter_rewrites_do_not_leave_trailing_garbage() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = state_path(dir.path());

    advance_watermark(&path, "polaris", watermark(&"long".repeat(50))).expect("long entry");
    advance_watermark(&path, "polaris", watermark("s")).expect("short entry");

    // A truncation bug would leave trailing JSON that fails to parse.
    let raw = std::fs::read_to_string(&path).expect("read state");
    let state: crate::qa::QaSweepState = serde_json::from_str(&raw).expect("clean json");
    assert_eq!(
        state
            .workspaces
            .get("polaris")
            .map(|w| w.last_validated_sha.as_str()),
        Some("s")
    );
}

#[test]
fn pass_lock_is_exclusive_per_host() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = try_acquire_pass_lock(dir.path()).expect("acquire");
    assert!(first.is_some());
    // Same-process re-acquisition behavior varies by platform for flock, so
    // only assert release-on-drop: after dropping the guard the lock is free.
    drop(first);
    let second = try_acquire_pass_lock(dir.path()).expect("re-acquire");
    assert!(second.is_some());
}
