use std::sync::{Arc, Barrier};
use std::thread;

use chrono::Utc;
use orbit_core::OrbitRuntime;
use orbit_policy::PolicyEngine;
use orbit_types::JobStatus;
use tempfile::tempdir;

#[test]
fn job_run_uses_real_command_outcome_for_status() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let now = Utc::now();

    let success_file = dir.path().join("job-success.txt");
    let success_command = format!("printf ok > {}", success_file.to_string_lossy());

    let success_job = runtime
        .schedule_job("success", &success_command, now)
        .expect("schedule success job");
    let fail_job = runtime
        .schedule_job("fail", "exit 7", now)
        .expect("schedule fail job");

    let ran = runtime.run_due_jobs(now).expect("run jobs");
    assert_eq!(ran, 2);

    let success_status = runtime
        .job_status(&success_job.id)
        .expect("status query")
        .expect("status exists");
    let fail_status = runtime
        .job_status(&fail_job.id)
        .expect("status query")
        .expect("status exists");

    assert_eq!(success_status, JobStatus::Complete);
    assert_eq!(fail_status, JobStatus::Failed);
    assert!(
        success_file.exists(),
        "success command side effect should exist"
    );
}

#[test]
fn denied_job_execution_emits_policy_denied_and_no_side_effects() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path())
        .expect("runtime")
        .with_policy(PolicyEngine::new_local_default_allow().deny_tool("proc.spawn"));
    let now = Utc::now();

    let denied_file = dir.path().join("should-not-exist.txt");
    let denied_command = format!("printf no > {}", denied_file.to_string_lossy());

    let job = runtime
        .schedule_job("denied", &denied_command, now)
        .expect("schedule denied job");

    let ran = runtime.run_due_jobs(now).expect("run jobs");
    assert_eq!(ran, 1);

    let status = runtime
        .job_status(&job.id)
        .expect("status query")
        .expect("status exists");
    assert_eq!(status, JobStatus::Failed);
    assert!(
        !denied_file.exists(),
        "denied job should not execute side effect"
    );

    let audits = runtime.list_audits(20).expect("audits");
    assert!(
        audits.iter().any(|a| a.event_type == "PolicyDenied"),
        "policy denied path must be audited"
    );
}

#[test]
fn concurrent_job_run_invocations_do_not_double_run_job() {
    let dir = tempdir().expect("tempdir");
    let runtime = Arc::new(OrbitRuntime::from_data_root(dir.path()).expect("runtime"));
    let now = Utc::now();

    let output_file = dir.path().join("job-output.txt");
    let command = format!(
        "sleep 0.2; printf run\\n >> {}",
        output_file.to_string_lossy()
    );

    let job = runtime
        .schedule_job("concurrent", &command, now)
        .expect("schedule job");

    let barrier = Arc::new(Barrier::new(3));

    let r1 = Arc::clone(&runtime);
    let b1 = Arc::clone(&barrier);
    let t1 = thread::spawn(move || {
        b1.wait();
        r1.run_due_jobs(now).expect("thread 1 run")
    });

    let r2 = Arc::clone(&runtime);
    let b2 = Arc::clone(&barrier);
    let t2 = thread::spawn(move || {
        b2.wait();
        r2.run_due_jobs(now).expect("thread 2 run")
    });

    barrier.wait();

    let c1 = t1.join().expect("join t1");
    let c2 = t2.join().expect("join t2");
    assert_eq!(c1 + c2, 1, "job should be claimed exactly once");

    let status = runtime
        .job_status(&job.id)
        .expect("status query")
        .expect("status exists");
    assert_eq!(status, JobStatus::Complete);

    let output = std::fs::read_to_string(&output_file).expect("output file");
    assert_eq!(output.lines().count(), 1);
}
