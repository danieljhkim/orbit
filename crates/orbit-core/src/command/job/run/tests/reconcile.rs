//! Stale read/list behavior and terminal timing repair tests.

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
        .mark_job_run_running(&run.run_id, started_at, 999_999)
        .expect("mark running with impossible pid");

    let shown = runtime.show_job_run(&run.run_id).expect("show run");

    assert_eq!(shown.state, JobRunState::Interrupted);
    assert!(shown.finished_at.is_some());
    assert!(shown.duration_ms.is_some_and(|value| value > 0));
    assert!(shown.steps.iter().any(|step| {
        step.state == JobRunState::Interrupted
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
        .mark_job_run_running(&run.run_id, Utc::now(), pid)
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
        .mark_job_run_running(&run.run_id, started_at, std::process::id())
        .expect("mark running");
    runtime
        .stores()
        .jobs()
        .finalize_job_run(&run.run_id, JobRunState::Success, finished_at, Some(5_000))
        .expect("finalize success");
    let finalized = runtime.show_job_run(&run.run_id).expect("show finalized");
    strip_run_timing(&runtime, &finalized);
    write_run_finished_audit(&runtime, &run.run_id, finished_at);

    let repaired = runtime.show_job_run(&run.run_id).expect("show repaired");

    assert_eq!(repaired.state, JobRunState::Success);
    assert_eq!(repaired.finished_at, Some(finished_at));
    assert_eq!(repaired.duration_ms, Some(5_000));
}

/// [ORB-10070] The workspace-open orphan scan terminalizes `pending` children
/// left behind by an interrupted parent run: never claimed by any worker and
/// far older than the claim grace window, they finalize as `interrupted`
/// instead of deferring consumers forever.
#[test]
fn workspace_open_reconciles_orphaned_pending_children_of_interrupted_parent() {
    let _env = orbit_common::test_env::unset(["ORBIT_MANAGED_RUN_CONTEXT", "ORBIT_RUN_ID"]);
    let (root, runtime) = test_runtime();
    let parent = insert_pending_run(&runtime, "task_auto_pipeline");
    runtime
        .stores()
        .jobs()
        .mark_job_run_running(&parent.run_id, Utc::now() - Duration::days(4), 999_999)
        .expect("mark parent running");
    runtime
        .stores()
        .jobs()
        .finalize_job_run(
            &parent.run_id,
            JobRunState::Interrupted,
            Utc::now() - Duration::days(4),
            Some(1_000),
        )
        .expect("finalize parent interrupted");
    let children = [
        insert_pending_run(&runtime, "task_gate_pipeline"),
        insert_pending_run(&runtime, "task_gate_pipeline"),
    ];
    for child in &children {
        backdate_run_created_at(&runtime, child, Utc::now() - Duration::days(4));
    }
    drop(runtime);

    let reopened = OrbitRuntime::from_roots(
        &root.path().join("global"),
        &root.path().join("repo").join(".orbit"),
    )
    .expect("reopen workspace");

    for child in &children {
        let reconciled = reopened
            .get_job_run_backend(&child.run_id)
            .expect("read child run")
            .expect("child run exists");
        assert_eq!(reconciled.state, JobRunState::Interrupted);
        assert!(reconciled.finished_at.is_some());
        let shown = reopened.show_job_run(&child.run_id).expect("show child");
        assert!(shown.steps.iter().any(|step| {
            step.state == JobRunState::Interrupted
                && step
                    .error_message
                    .as_deref()
                    .is_some_and(|message| message.contains("no live worker process owns it"))
        }));
    }
    let parent = reopened
        .get_job_run_backend(&parent.run_id)
        .expect("read parent run")
        .expect("parent run exists");
    assert_eq!(parent.state, JobRunState::Interrupted);
}

/// [ORB-10070] A freshly queued run that no worker has claimed yet is inside
/// the grace window and must never be terminalized by a racing reconcile.
#[test]
fn reconcile_keeps_fresh_unclaimed_pending_run_pending() {
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_fresh_pending");

    runtime
        .reconcile_stale_job_runs(None)
        .expect("reconcile scan");

    let stored = runtime
        .get_job_run_backend(&run.run_id)
        .expect("read run")
        .expect("run exists");
    assert_eq!(stored.state, JobRunState::Pending);
}

/// [ORB-10070] A pending run claimed by a live worker stays pending even far
/// past the unclaimed grace window: the claim, not the run's age, decides.
#[cfg(unix)]
#[test]
fn pending_run_with_live_claimed_owner_stays_pending() {
    use orbit_common::utility::process_identity::process_start_identity_token;

    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_claimed_live");
    let pid = std::process::id();
    if process_start_identity_token(pid).is_none() {
        return;
    }
    assert!(
        runtime
            .stores()
            .jobs()
            .claim_pending_job_run_owner(&run.run_id, pid)
            .expect("claim pending run")
    );
    backdate_run_created_at(&runtime, &run, Utc::now() - Duration::days(4));

    runtime
        .reconcile_stale_job_runs(None)
        .expect("reconcile scan");

    let stored = runtime
        .get_job_run_backend(&run.run_id)
        .expect("read run")
        .expect("run exists");
    assert_eq!(stored.state, JobRunState::Pending);
    assert_eq!(stored.pid, Some(pid));
}

/// [ORB-10070] A pending run whose claimed worker died is conclusively
/// orphaned and reconciles to `interrupted` immediately — no grace window.
#[cfg(unix)]
#[test]
fn pending_run_with_dead_claimed_owner_is_reconciled() {
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_claimed_dead");
    assert!(
        runtime
            .stores()
            .jobs()
            .claim_pending_job_run_owner(&run.run_id, 999_999)
            .expect("claim with impossible pid")
    );

    runtime
        .reconcile_stale_job_runs(None)
        .expect("reconcile scan");

    let shown = runtime.show_job_run(&run.run_id).expect("show run");
    assert_eq!(shown.state, JobRunState::Interrupted);
    assert!(shown.finished_at.is_some());
    assert!(shown.steps.iter().any(|step| {
        step.state == JobRunState::Interrupted
            && step
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("no live worker process owns it"))
    }));
}

#[cfg(unix)]
#[test]
fn list_job_runs_reconciles_before_state_filtering() {
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_filter");
    runtime
        .stores()
        .jobs()
        .mark_job_run_running(&run.run_id, Utc::now() - Duration::seconds(3), 999_999)
        .expect("mark stale running");

    let running = runtime
        .list_job_runs(JobRunListParams {
            state: Some(JobRunState::Running),
            ..JobRunListParams::default()
        })
        .expect("list running");
    let interrupted = runtime
        .list_job_runs(JobRunListParams {
            state: Some(JobRunState::Interrupted),
            ..JobRunListParams::default()
        })
        .expect("list interrupted");

    assert!(
        !running
            .iter()
            .any(|candidate| candidate.run_id == run.run_id)
    );
    assert!(
        interrupted
            .iter()
            .any(|candidate| candidate.run_id == run.run_id)
    );
}
