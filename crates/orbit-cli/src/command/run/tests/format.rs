use super::super::format::*;
use orbit_types::workflow::{JobRunState, PipelineState};
use serde_json::json;

fn state_with_waiting(deps: Option<Vec<&str>>, locks: Option<Vec<&str>>) -> PipelineState {
    let mut state = PipelineState::new("jrun-test".to_string(), "job-test".to_string(), json!({}));
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

/// [ORB-10971] The child-lineage lines an operator sees on `orbit run show`.
#[test]
fn child_dispatch_lines_name_the_child_run_job_and_phase() {
    let mut state = PipelineState::new("jrun-test".to_string(), "job-test".to_string(), json!({}));
    state.record_child_dispatch(
        orbit_types::workflow::ChildDispatch::submitted(
            "jrun-child-leaves".to_string(),
            "task_auto_pipeline".to_string(),
            "invoke_and_wait".to_string(),
            true,
            false,
            chrono::Utc::now(),
        )
        .with_parent_step_id(Some("ship_leaves".to_string())),
    );
    state.advance_child_dispatch(
        "jrun-child-leaves",
        orbit_types::workflow::ChildDispatchPhase::Waiting,
        None,
        None,
    );

    assert_eq!(
        format_child_dispatch_lines(Some(&state)),
        vec![
            "Child jrun-child-leaves job=task_auto_pipeline step=ship_leaves phase=waiting queued=false"
                .to_string()
        ]
    );
}

#[test]
fn child_dispatch_lines_survive_a_cancelled_parent_and_name_the_policy() {
    let mut state = PipelineState::new("jrun-test".to_string(), "job-test".to_string(), json!({}));
    state.record_child_dispatch(orbit_types::workflow::ChildDispatch::submitted(
        "jrun-child-leaves".to_string(),
        "task_auto_pipeline".to_string(),
        "invoke_and_wait".to_string(),
        true,
        false,
        chrono::Utc::now(),
    ));
    state.terminalize_child_dispatch(
        "jrun-child-leaves",
        orbit_types::workflow::ChildCancellation {
            policy: orbit_types::workflow::ChildCancellationPolicy::Cascade,
            outcome: "cancelled".to_string(),
            error: None,
            at: chrono::Utc::now(),
        },
    );

    let lines = format_child_dispatch_lines(Some(&state));
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("phase=terminal"), "{}", lines[0]);
    assert!(
        lines[0].contains("cancellation=cascade/cancelled"),
        "{}",
        lines[0]
    );
}

#[test]
fn no_child_dispatches_prints_nothing() {
    let state = PipelineState::new("jrun-test".to_string(), "job-test".to_string(), json!({}));
    assert!(format_child_dispatch_lines(Some(&state)).is_empty());
    assert!(format_child_dispatch_lines(None).is_empty());
}
