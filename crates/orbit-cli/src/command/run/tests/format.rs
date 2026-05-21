use super::super::format::*;
use orbit_common::types::{JobRunState, PipelineState};
use serde_json::json;

fn state_with_waiting(deps: Option<Vec<&str>>, locks: Option<Vec<&str>>) -> PipelineState {
    let mut state =
        PipelineState::new("jrun-test".to_string(), "job-test".to_string(), json!({}));
    state.set_waiting_reasons(
        deps.map(|values| values.into_iter().map(str::to_string).collect()),
        locks.map(|values| values.into_iter().map(str::to_string).collect()),
    );
    state
}

#[test]
fn waiting_line_lists_deps_and_locks_for_waiting_run() {
    let state = state_with_waiting(Some(vec!["ORB-1", "ORB-2"]), Some(vec!["file:src/lib.rs"]));

    assert_eq!(
        format_waiting_line(JobRunState::Running, Some(&state)),
        Some("Waiting on deps: ORB-1, ORB-2; locks: file:src/lib.rs".to_string())
    );
}

#[test]
fn waiting_line_omits_non_waiting_run() {
    let state = PipelineState::new("jrun-test".to_string(), "job-test".to_string(), json!({}));

    assert_eq!(
        format_waiting_line(JobRunState::Running, Some(&state)),
        None
    );
}

#[test]
fn waiting_line_omits_terminal_run_even_with_stale_reasons() {
    let state = state_with_waiting(Some(vec!["ORB-1"]), Some(vec!["file:src/lib.rs"]));

    assert_eq!(
        format_waiting_line(JobRunState::Success, Some(&state)),
        None
    );
}
