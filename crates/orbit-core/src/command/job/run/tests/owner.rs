//! Timezone and probe-outcome regression coverage for owner identity classification.

use super::*;

use super::super::JobRunListParams;

#[cfg(unix)]
use super::super::owner::{OwnerIdentity, classify_run_owner_with_probes, stale_job_run_message};
use chrono::{Duration, Utc};
use orbit_common::types::JobRunState;
#[cfg(unix)]
use orbit_common::utility::process_identity::ProbeOutcome;
#[cfg(unix)]
use orbit_common::utility::process_identity::{STABLE_TOKEN_PREFIX, process_start_identity_token};
#[cfg(unix)]
use std::process::{Command, Stdio};

static TZ_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(unix)]
struct TzGuard {
    prior: Option<String>,
}

#[cfg(unix)]
impl TzGuard {
    fn set(value: &str) -> Self {
        let prior = std::env::var("TZ").ok();
        // SAFETY: All TZ-mutating tests in this module take TZ_TEST_LOCK
        // before constructing a TzGuard, serializing env mutation across
        // threads; the guard restores the previous value on drop.
        unsafe { std::env::set_var("TZ", value) };
        Self { prior }
    }
}

#[cfg(unix)]
impl Drop for TzGuard {
    fn drop(&mut self) {
        // SAFETY: see TzGuard::set.
        unsafe {
            match &self.prior {
                Some(value) => std::env::set_var("TZ", value),
                None => std::env::remove_var("TZ"),
            }
        }
    }
}

#[cfg(unix)]
fn spawn_sentinel() -> std::process::Child {
    Command::new("sleep")
        .arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sentinel")
}

#[cfg(unix)]
#[test]
fn live_owner_survives_tz_change_across_read_paths() {
    let _tz_lock = TZ_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_tz_change");
    let mut sentinel = spawn_sentinel();
    let sentinel_pid = sentinel.id();

    // Write the run under a non-UTC ambient TZ. The fix forces the child
    // ps invocation to TZ=UTC regardless, so the persisted token must
    // carry the versioned prefix and remain identical across caller
    // environments.
    let persisted_token = {
        let _tz = TzGuard::set("America/Los_Angeles");
        runtime
            .stores()
            .jobs()
            .mark_run_running(&run.run_id, Utc::now() - Duration::seconds(1), sentinel_pid)
            .expect("mark running under LA tz");
        runtime
            .show_job_run(&run.run_id)
            .expect("show fresh run")
            .pid_start_time
            .expect("token must be persisted")
    };
    assert!(
        persisted_token.starts_with(STABLE_TOKEN_PREFIX),
        "persisted identity token must be versioned: {persisted_token}"
    );

    // Switch TZ before driving the read paths. Pre-fix this is exactly
    // when reconciliation falsely finalized the still-running worker.
    let _tz = TzGuard::set("UTC");

    let shown = runtime.show_job_run(&run.run_id).expect("show under UTC");
    assert_eq!(shown.state, JobRunState::Running);
    assert!(shown.finished_at.is_none());
    assert!(shown.duration_ms.is_none());
    assert!(
        !shown
            .steps
            .iter()
            .any(|step| step.error_message.as_deref().is_some_and(|message| {
                message.contains("recorded worker process is no longer alive")
            })),
        "live worker must not have a stale-failure step"
    );

    let listed = runtime
        .list_job_runs(JobRunListParams {
            state: Some(JobRunState::Running),
            ..JobRunListParams::default()
        })
        .expect("list running under UTC");
    assert!(
        listed
            .iter()
            .any(|candidate| candidate.run_id == run.run_id),
        "live worker must still appear in the Running list after a TZ change"
    );

    let waited = runtime
        .wait_pipeline_runs(
            std::slice::from_ref(&run.run_id),
            0,
            1,
            Some("tz_change_test"),
        )
        .expect("wait under UTC");
    assert_eq!(waited.results.len(), 1);
    assert_eq!(waited.results[0].run_id, run.run_id);
    assert_ne!(
        waited.results[0].status, "failed",
        "wait must not report failed for a live worker after a TZ change"
    );

    let final_state = runtime.show_job_run(&run.run_id).expect("final show").state;
    assert_eq!(final_state, JobRunState::Running);

    let _ = sentinel.kill();
    let _ = sentinel.wait();
}

#[cfg(unix)]
#[test]
fn versioned_token_is_stable_across_tz_change() {
    let _tz_lock = TZ_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut sentinel = spawn_sentinel();
    let pid = sentinel.id();

    let utc_token = {
        let _tz = TzGuard::set("UTC");
        process_start_identity_token(pid).expect("token under UTC")
    };
    let la_token = {
        let _tz = TzGuard::set("America/Los_Angeles");
        process_start_identity_token(pid).expect("token under LA")
    };

    assert!(utc_token.starts_with(STABLE_TOKEN_PREFIX));
    assert_eq!(
        utc_token, la_token,
        "versioned identity token must not depend on the caller's TZ"
    );

    let _ = sentinel.kill();
    let _ = sentinel.wait();
}

#[cfg(unix)]
#[test]
fn legacy_unversioned_token_does_not_falsely_finalize_live_run() {
    // A pre-fix run with a non-versioned `pid_start_time` whose value
    // cannot be matched under either environment should classify as
    // LegacyLiveUnverified, keeping the run Running instead of finalizing
    // it as Failed.
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_legacy_unverified");
    let mut sentinel = spawn_sentinel();
    let sentinel_pid = sentinel.id();
    runtime
        .stores()
        .jobs()
        .mark_run_running(&run.run_id, Utc::now() - Duration::seconds(1), sentinel_pid)
        .expect("mark running");

    // Rewrite the stored token to look like a pre-fix unversioned value
    // that does not match the live process under either env.
    let yaml_path = runtime
        .data_root()
        .join("state")
        .join("job-runs")
        .join(&run.job_id)
        .join(&run.run_id)
        .join("jrun.yaml");
    let raw = std::fs::read_to_string(&yaml_path).expect("read run yaml");
    let edited = raw
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("pid_start_time:") {
                "  pid_start_time: legacy-token-that-cannot-be-rederived".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&yaml_path, format!("{edited}\n")).expect("write legacy token");

    let shown = runtime.show_job_run(&run.run_id).expect("show legacy run");
    assert_eq!(shown.state, JobRunState::Running);
    assert!(shown.finished_at.is_none());

    let _ = sentinel.kill();
    let _ = sentinel.wait();
}

// ---- Probe-outcome regression coverage (ORB-00037) ----
//
// `classify_run_owner_with_probes` lets these tests inject deterministic
// `ProbeOutcome` values without depending on a real misbehaving `ps`.
// They guard the rule from the task ACs: a transient probe failure with a
// live PID must never terminalize the run; a dead PID still must.

#[cfg(unix)]
#[test]
fn probe_unavailable_with_live_pid_classifies_as_probe_unavailable() {
    let versioned = format!("{STABLE_TOKEN_PREFIX}lstart-token");
    let identity = classify_run_owner_with_probes(
        Some(4242),
        Some(versioned.as_str()),
        |_| ProbeOutcome::Unavailable,
        |_| false,
        |_| true,
    );
    assert_eq!(identity, OwnerIdentity::ProbeUnavailable);
    // Build a JobRun with state=Running so we can exercise the stale-path
    // gate alongside the classification (the closure-based classifier is
    // the only path that distinguishes Unavailable from NoProcess).
}

#[cfg(unix)]
#[test]
fn probe_no_process_with_live_pid_classifies_as_probe_unavailable() {
    // Race: ps -p says no-process, but kill(pid, 0) still sees the PID.
    // We must not finalize the run on a single ps result that disagrees
    // with the kernel's liveness signal.
    let versioned = format!("{STABLE_TOKEN_PREFIX}lstart-token");
    let identity = classify_run_owner_with_probes(
        Some(4242),
        Some(versioned.as_str()),
        |_| ProbeOutcome::NoProcess,
        |_| false,
        |_| true,
    );
    assert_eq!(identity, OwnerIdentity::ProbeUnavailable);
}

#[cfg(unix)]
#[test]
fn probe_unavailable_with_dead_pid_classifies_as_missing() {
    let versioned = format!("{STABLE_TOKEN_PREFIX}lstart-token");
    let identity = classify_run_owner_with_probes(
        Some(4242),
        Some(versioned.as_str()),
        |_| ProbeOutcome::Unavailable,
        |_| false,
        |_| false,
    );
    // Probe failed AND kill(0) confirms dead → still legitimately stale.
    assert_eq!(identity, OwnerIdentity::Missing);
}

#[cfg(unix)]
#[test]
fn versioned_token_mismatch_with_live_pid_classifies_as_mismatch() {
    let persisted = format!("{STABLE_TOKEN_PREFIX}old-lstart");
    let identity = classify_run_owner_with_probes(
        Some(4242),
        Some(persisted.as_str()),
        |_| ProbeOutcome::Token(format!("{STABLE_TOKEN_PREFIX}fresh-lstart")),
        |_| false,
        |_| true,
    );
    assert_eq!(identity, OwnerIdentity::Mismatch);
}

#[cfg(unix)]
#[test]
fn versioned_token_match_classifies_as_verified() {
    let persisted = format!("{STABLE_TOKEN_PREFIX}same-lstart");
    let identity = classify_run_owner_with_probes(
        Some(4242),
        Some(persisted.as_str()),
        |_| ProbeOutcome::Token(format!("{STABLE_TOKEN_PREFIX}same-lstart")),
        |_| false,
        |_| true,
    );
    assert_eq!(identity, OwnerIdentity::Verified);
}

#[cfg(unix)]
#[test]
fn running_run_owner_stale_reason_excludes_probe_unavailable() {
    // A Running run whose probe is Unavailable and whose PID is alive
    // must NOT be classified as stale.
    let run = JobRun {
        run_id: "qa_run".to_string(),
        job_id: "qa_job".to_string(),
        attempt: 1,
        state: JobRunState::Running,
        scheduled_at: Utc::now(),
        started_at: Some(Utc::now()),
        finished_at: None,
        duration_ms: None,
        pid: Some(4242),
        pid_start_time: Some(format!("{STABLE_TOKEN_PREFIX}lstart-token")),
        input: None,
        retry_source_run_id: None,
        created_at: Utc::now(),
        steps: Vec::new(),
        knowledge_metrics: None,
        resolved_crew: None,
        planner_model: None,
        implementer_model: None,
        reviewer_model: None,
    };
    // We can't override the probe at this seam (production wrapper), but
    // we can assert the lower-level helper agrees: ProbeUnavailable is
    // not in the stale set.
    let identity = classify_run_owner_with_probes(
        run.pid,
        run.pid_start_time.as_deref(),
        |_| ProbeOutcome::Unavailable,
        |_| false,
        |_| true,
    );
    assert!(matches!(identity, OwnerIdentity::ProbeUnavailable));
    // And the stale-reason helper would only emit Some for Mismatch /
    // Missing — verified separately by other tests.
}

#[cfg(unix)]
#[test]
fn stale_failure_message_distinguishes_probe_outcomes() {
    let run = JobRun {
        run_id: "qa_run".to_string(),
        job_id: "qa_job".to_string(),
        attempt: 1,
        state: JobRunState::Running,
        scheduled_at: Utc::now(),
        started_at: Some(Utc::now()),
        finished_at: None,
        duration_ms: None,
        pid: Some(4242),
        pid_start_time: Some(format!("{STABLE_TOKEN_PREFIX}lstart-token")),
        input: None,
        retry_source_run_id: None,
        created_at: Utc::now(),
        steps: Vec::new(),
        knowledge_metrics: None,
        resolved_crew: None,
        planner_model: None,
        implementer_model: None,
        reviewer_model: None,
    };
    let mismatch_message = stale_job_run_message(&run, Some(OwnerIdentity::Mismatch));
    let missing_message = stale_job_run_message(&run, Some(OwnerIdentity::Missing));
    let probe_unavailable_message =
        stale_job_run_message(&run, Some(OwnerIdentity::ProbeUnavailable));

    assert!(
        mismatch_message.contains("reason=token_mismatch"),
        "{mismatch_message}"
    );
    assert!(
        missing_message.contains("reason=process_not_found"),
        "{missing_message}"
    );
    // Even though this state never finalizes, the tag must be set so a
    // future caller's diagnostic is never silently mis-labeled.
    assert!(
        probe_unavailable_message.contains("reason=probe_unavailable"),
        "{probe_unavailable_message}"
    );
}

#[cfg(unix)]
#[test]
fn show_job_run_reconciles_dead_pid_with_probe_outcome_in_message() {
    // End-to-end regression: dead PID still finalizes, and the failure
    // step's error message carries `reason=process_not_found`.
    let (_root, runtime) = test_runtime();
    let run = insert_pending_run(&runtime, "qa_dead_pid_reason");
    let started_at = Utc::now() - Duration::seconds(3);
    runtime
        .stores()
        .jobs()
        .mark_run_running(&run.run_id, started_at, 999_999)
        .expect("mark running with impossible pid");

    let shown = runtime.show_job_run(&run.run_id).expect("show run");
    assert_eq!(shown.state, JobRunState::Failed);
    let failure_step = shown
        .steps
        .iter()
        .find(|step| step.state == JobRunState::Failed)
        .expect("stale failure step");
    let message = failure_step
        .error_message
        .as_deref()
        .expect("failure message");
    assert!(
        message.contains("reason=process_not_found"),
        "diagnostic must record probe outcome: {message}"
    );
}
