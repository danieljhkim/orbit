//! [ORB-10971] Child-dispatch bookkeeping on a run's pipeline state.

use chrono::Utc;

use crate::workflow::{
    ChildCancellation, ChildCancellationPolicy, ChildDispatch, ChildDispatchPhase, PipelineState,
};

fn state() -> PipelineState {
    PipelineState::new(
        "jrun-parent".to_string(),
        "workspace_auto_pipeline".to_string(),
        serde_json::json!({}),
    )
}

fn dispatch(child_run_id: &str, blocking: bool) -> ChildDispatch {
    ChildDispatch::submitted(
        child_run_id.to_string(),
        "task_auto_pipeline".to_string(),
        "invoke_and_wait".to_string(),
        blocking,
        false,
        Utc::now(),
    )
}

#[test]
fn re_recording_the_same_child_updates_it_rather_than_duplicating_it() {
    // A resumed or retried parent re-executes its dispatch step. The child is
    // one child, not two rows.
    let mut state = state();
    let first = dispatch("jrun-child", true);
    let submitted_at = first.submitted_at;
    state.record_child_dispatch(first);

    let mut second = dispatch("jrun-child", true);
    second.queued = true;
    second.submitted_at = Utc::now();
    state.record_child_dispatch(second);

    assert_eq!(state.child_dispatches.len(), 1);
    assert!(state.child_dispatches[0].queued);
    assert_eq!(
        state.child_dispatches[0].submitted_at, submitted_at,
        "the observable submission instant must not drift on re-record"
    );
}

#[test]
fn advancing_an_unrecorded_child_reports_the_lost_checkpoint() {
    let mut state = state();
    assert!(
        !state.advance_child_dispatch("jrun-missing", ChildDispatchPhase::Waiting, None, None),
        "a caller must be able to tell a lost checkpoint from a successful update"
    );
}

#[test]
fn clearing_waiting_reasons_leaves_child_lineage_intact() {
    // Cancellation clears the waiting reasons. The child link must survive it:
    // it is the only handle on the work the parent left behind.
    let mut state = state();
    state.set_waiting_reasons(Some(vec!["ORB-1".to_string()]), None);
    state.record_child_dispatch(dispatch("jrun-child", true));

    state.clear_waiting_reasons();

    assert_eq!(state.waiting_on_deps, None);
    assert_eq!(state.child_dispatches.len(), 1);
}

#[test]
fn only_blocking_children_are_cascade_targets() {
    let mut state = state();
    state.record_child_dispatch(dispatch("jrun-blocking", true));
    state.record_child_dispatch(dispatch("jrun-detached", false));

    assert_eq!(state.cascade_cancellation_targets(), vec!["jrun-blocking"]);
}

#[test]
fn a_terminalized_child_is_no_longer_open_but_stays_recorded() {
    let mut state = state();
    state.record_child_dispatch(dispatch("jrun-child", true));

    assert!(state.terminalize_child_dispatch(
        "jrun-child",
        ChildCancellation {
            policy: ChildCancellationPolicy::Cascade,
            outcome: "cancelled".to_string(),
            error: None,
            at: Utc::now(),
        },
    ));

    assert_eq!(state.open_child_dispatches().count(), 0);
    assert!(state.cascade_cancellation_targets().is_empty());
    assert_eq!(state.child_dispatches.len(), 1);
    assert_eq!(
        state.child_dispatches[0].phase,
        ChildDispatchPhase::Terminal
    );
}

#[test]
fn a_state_without_children_round_trips_without_the_field() {
    // Existing on-disk `state.json` files predate this field and must keep
    // decoding, and a childless run must not start emitting an empty array.
    let encoded = serde_json::to_value(state()).expect("encode");
    assert!(
        !encoded
            .as_object()
            .expect("object")
            .contains_key("child_dispatches")
    );

    let decoded: PipelineState = serde_json::from_value(encoded).expect("decode");
    assert!(decoded.child_dispatches.is_empty());
}
