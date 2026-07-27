//! [ORB-10002] Job run state machine tests for the `interrupted` state.

use std::str::FromStr;

use crate::types::{JobRunState, RunEvent};

#[test]
fn running_interrupt_transitions_to_interrupted() {
    assert_eq!(
        JobRunState::Running.try_transition(RunEvent::Interrupt),
        Ok(JobRunState::Interrupted)
    );
}

#[test]
fn interrupted_is_terminal_and_rejects_further_events() {
    assert!(JobRunState::Interrupted.is_terminal());
    for event in [
        RunEvent::Start,
        RunEvent::Complete,
        RunEvent::Fail,
        RunEvent::Timeout,
        RunEvent::Cancel,
        RunEvent::Abandon,
        RunEvent::Interrupt,
    ] {
        assert!(
            JobRunState::Interrupted.try_transition(event).is_err(),
            "interrupted must reject {event}"
        );
    }
}

#[test]
fn pending_interrupt_transitions_to_interrupted() {
    // [ORB-10070] Orphaned queued runs (dead or never-claimed worker) finalize
    // as interrupted, the same terminal state as orphaned running runs.
    assert_eq!(
        JobRunState::Pending.try_transition(RunEvent::Interrupt),
        Ok(JobRunState::Interrupted)
    );
}

#[test]
fn interrupted_display_and_parse_round_trip() {
    assert_eq!(JobRunState::Interrupted.to_string(), "interrupted");
    assert_eq!(
        JobRunState::from_str("interrupted"),
        Ok(JobRunState::Interrupted)
    );
}

#[test]
fn interrupted_is_a_valid_step_state() {
    assert!(JobRunState::Interrupted.validate_step_state().is_ok());
}
