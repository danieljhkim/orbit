//! Cancellation behavior and process-group cancellation tests.

use super::*;

#[cfg(unix)]
use super::super::owner::process_is_alive;
use chrono::{Duration, Utc};
use std::path::Path;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::time::{Duration as StdDuration, Instant as StdInstant};
use tempfile::tempdir;

#[test]
fn cancel_job_run_marks_pending_cancelled_without_signal() {
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_cancel_pending");

    let result = runtime
        .cancel_job_run_with_context(&run.run_id, "tester", "unit")
        .expect("cancel pending");

    assert_eq!(result.previous_state, "pending");
    assert_eq!(result.final_state, "cancelled");
    assert!(!result.signal_attempted);
    assert_eq!(result.signal_outcome, None);
    let stored = runtime.show_job_run(&run.run_id).expect("show run");
    assert_eq!(stored.state, JobRunState::Cancelled);
    assert!(stored.finished_at.is_some());
    assert_eq!(stored.duration_ms, None);

    let audits = runtime.list_session_events(10).expect("events");
    let payload = audits
        .iter()
        .find(|event| event.event_type == "JobRunCancelled")
        .map(|event| &event.payload["data"])
        .expect("cancel event");
    assert_eq!(payload["run_id"], run.run_id);
    assert_eq!(payload["previous_state"], "pending");
    assert_eq!(payload["final_state"], "cancelled");
    assert_eq!(payload["actor"], "tester");
    assert_eq!(payload["source"], "unit");
    assert_eq!(payload["signal_attempted"], false);
}

#[test]
fn cancelled_pending_run_is_not_claimed_by_pipeline_worker() {
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_cancel_worker_skip");
    runtime
        .cancel_job_run(&run.run_id)
        .expect("cancel pending run");

    runtime
        .execute_pipeline_run_worker(&run.run_id)
        .expect("worker exits without executing cancelled run");

    let stored = runtime.show_job_run(&run.run_id).expect("show run");
    assert_eq!(stored.state, JobRunState::Cancelled);
    assert!(stored.started_at.is_none());
    assert!(stored.steps.is_empty());
}

#[test]
fn cancelled_run_wait_status_reports_cancelled() {
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_cancel_wait");
    runtime.cancel_job_run(&run.run_id).expect("cancel run");

    let result = runtime
        .wait_pipeline_runs(std::slice::from_ref(&run.run_id), 1, 1, Some("test"))
        .expect("wait cancelled");

    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].run_id, run.run_id);
    assert_eq!(result.results[0].status, "cancelled");
}

#[test]
fn cancel_job_run_rejects_terminal_run_without_mutating_bundle() {
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_cancel_terminal");
    let started_at = Utc::now() - Duration::seconds(2);
    let finished_at = Utc::now();
    runtime
        .stores()
        .jobs()
        .mark_run_running(&run.run_id, started_at, std::process::id())
        .expect("mark running");
    runtime
        .stores()
        .jobs()
        .finalize_run(&run.run_id, JobRunState::Success, finished_at, Some(2_000))
        .expect("finalize success");
    let before = runtime.show_job_run(&run.run_id).expect("show before");

    let error = runtime
        .cancel_job_run(&run.run_id)
        .expect_err("terminal cancellation must fail");

    assert!(
        error.to_string().contains("cannot cancel job run"),
        "{error}"
    );
    let after = runtime.show_job_run(&run.run_id).expect("show after");
    assert_eq!(after, before);
    let events = runtime.list_session_events(20).expect("events");
    assert!(
        events
            .iter()
            .all(|event| event.event_type != "JobRunCancelled")
    );
}

#[cfg(unix)]
fn wait_until<F>(timeout: StdDuration, mut condition: F) -> bool
where
    F: FnMut() -> bool,
{
    let started = StdInstant::now();
    while started.elapsed() < timeout {
        if condition() {
            return true;
        }
        std::thread::sleep(StdDuration::from_millis(50));
    }
    condition()
}

#[cfg(unix)]
fn read_pid_pair(path: &Path) -> (u32, u32) {
    let raw = std::fs::read_to_string(path).expect("read pid file");
    let mut parts = raw.split_whitespace();
    let owner = parts
        .next()
        .expect("owner pid")
        .parse()
        .expect("parse owner pid");
    let child = parts
        .next()
        .expect("child pid")
        .parse()
        .expect("parse child pid");
    (owner, child)
}

#[cfg(unix)]
#[test]
fn cancel_job_run_does_not_signal_reused_pid_identity_mismatch() {
    use orbit_common::utility::process_identity::STABLE_TOKEN_PREFIX;

    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_cancel_reused_pid");
    let mut sentinel = Command::new("sleep")
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sentinel");
    let sentinel_pid = sentinel.id();
    let started_at = Utc::now() - Duration::seconds(1);
    runtime
        .stores()
        .jobs()
        .mark_run_running(&run.run_id, started_at, sentinel_pid)
        .expect("mark running");
    // Versioned token guarantees we exercise the strict `Mismatch`
    // classification path; legacy unversioned tokens may flow through the
    // softer LegacyLiveUnverified branch but must still produce
    // owner_identity_mismatch from `signal_run_owner_process`.
    let mismatched_versioned =
        format!("{STABLE_TOKEN_PREFIX}definitely-not-the-sentinel-start-token");
    set_run_pid_start_time(&runtime, &run, &mismatched_versioned);

    let result = runtime.cancel_job_run(&run.run_id).expect("cancel run");

    assert!(result.signal_attempted);
    assert_eq!(
        result.signal_outcome.as_deref(),
        Some("owner_identity_mismatch")
    );
    assert!(
        process_is_alive(sentinel_pid),
        "sentinel process must not be killed by mismatched owner identity"
    );
    let _ = sentinel.kill();
    let _ = sentinel.wait();
}

#[cfg(unix)]
#[test]
fn cancel_job_run_terminates_owner_process_group_and_child() {
    use std::os::unix::process::CommandExt;

    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_cancel_process_group");
    let pid_dir = tempdir().expect("pid tempdir");
    let pid_file = pid_dir.path().join("pids");
    let script = format!(
        "trap 'exit 0' TERM; (trap '' TERM; sleep 30) & child=$!; printf '%s %s\\n' $$ \"$child\" > {}; wait",
        shell_quote(pid_file.to_string_lossy().as_ref())
    );
    let mut owner = Command::new("/bin/sh");
    owner
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        owner.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut owner = owner.spawn().expect("spawn owner");
    assert!(
        wait_until(StdDuration::from_secs(2), || pid_file.exists()),
        "owner did not write pid file"
    );
    let (owner_pid, child_pid) = read_pid_pair(&pid_file);
    assert_eq!(owner.id(), owner_pid);
    runtime
        .stores()
        .jobs()
        .mark_run_running(&run.run_id, Utc::now(), owner_pid)
        .expect("mark running");

    let result = runtime.cancel_job_run(&run.run_id).expect("cancel run");
    let _ = owner.wait();

    assert!(result.signal_attempted);
    assert_eq!(
        result.signal_outcome.as_deref(),
        Some("killed_process_group")
    );
    assert!(
        wait_until(StdDuration::from_secs(3), || !process_is_alive(child_pid)),
        "child process {child_pid} should be gone after process-group cancellation"
    );
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
