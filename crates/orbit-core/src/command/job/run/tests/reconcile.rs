//! Stale read/list behavior and terminal timing repair tests.

use super::super::*;
use super::*;

use super::super::JobRunListParams;
use chrono::{Duration, Utc};
use orbit_common::types::JobRunState;

#[test]
fn show_job_run_reconciles_stale_running_owner() {
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_stale");
    let started_at = Utc::now() - Duration::seconds(3);
    runtime
        .stores()
        .jobs()
        .mark_run_running(&run.run_id, started_at, 999_999)
        .expect("mark running with impossible pid");

    let shown = runtime.show_job_run(&run.run_id).expect("show run");

    assert_eq!(shown.state, JobRunState::Failed);
    assert!(shown.finished_at.is_some());
    assert!(shown.duration_ms.is_some_and(|value| value > 0));
    assert!(shown.steps.iter().any(|step| {
        step.state == JobRunState::Failed
            && step.error_message.as_deref().is_some_and(|message| {
                message.contains("recorded worker process is no longer alive")
            })
    }));
}

#[cfg(unix)]
#[test]
fn show_job_run_keeps_live_owner_running() {
    use orbit_common::utility::process_identity::process_start_identity_token;

    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_live");
    let pid = std::process::id();
    if process_start_identity_token(pid).is_none() {
        return;
    }
    runtime
        .stores()
        .jobs()
        .mark_run_running(&run.run_id, Utc::now(), pid)
        .expect("mark current process running");

    let shown = runtime.show_job_run(&run.run_id).expect("show run");

    assert_eq!(shown.state, JobRunState::Running);
    assert!(shown.finished_at.is_none());
    assert!(shown.duration_ms.is_none());
}

#[test]
fn show_job_run_keeps_pending_runs_pending() {
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_pending");

    let shown = runtime.show_job_run(&run.run_id).expect("show pending run");

    assert_eq!(shown.state, JobRunState::Pending);
    assert!(shown.finished_at.is_none());
    assert!(shown.duration_ms.is_none());
}

#[test]
fn show_job_run_repairs_terminal_run_missing_timing() {
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_terminal");
    let started_at = Utc::now() - Duration::seconds(8);
    let finished_at = started_at + Duration::seconds(5);
    runtime
        .stores()
        .jobs()
        .mark_run_running(&run.run_id, started_at, std::process::id())
        .expect("mark running");
    runtime
        .stores()
        .jobs()
        .finalize_run(&run.run_id, JobRunState::Success, finished_at, Some(5_000))
        .expect("finalize success");
    let finalized = runtime.show_job_run(&run.run_id).expect("show finalized");
    strip_run_timing(&runtime, &finalized);
    write_run_finished_audit(&runtime, &run.run_id, finished_at);

    let repaired = runtime.show_job_run(&run.run_id).expect("show repaired");

    assert_eq!(repaired.state, JobRunState::Success);
    assert_eq!(repaired.finished_at, Some(finished_at));
    assert_eq!(repaired.duration_ms, Some(5_000));
}

#[cfg(unix)]
#[test]
fn list_job_runs_reconciles_before_state_filtering() {
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_filter");
    runtime
        .stores()
        .jobs()
        .mark_run_running(&run.run_id, Utc::now() - Duration::seconds(3), 999_999)
        .expect("mark stale running");

    let running = runtime
        .list_job_runs(JobRunListParams {
            state: Some(JobRunState::Running),
            ..JobRunListParams::default()
        })
        .expect("list running");
    let failed = runtime
        .list_job_runs(JobRunListParams {
            state: Some(JobRunState::Failed),
            ..JobRunListParams::default()
        })
        .expect("list failed");

    assert!(
        !running
            .iter()
            .any(|candidate| candidate.run_id == run.run_id)
    );
    assert!(
        failed
            .iter()
            .any(|candidate| candidate.run_id == run.run_id)
    );
}
