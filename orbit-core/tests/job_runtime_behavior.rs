use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use orbit_core::OrbitRuntime;
use orbit_core::command::job::JobAddParams;
use orbit_core::command::task::TaskAddParams;
use orbit_policy::PolicyEngine;
use orbit_types::{EntityType, JobSessionStatus};
use serde_json::json;
use tempfile::tempdir;

fn add_task_with_tool_calls(
    runtime: &OrbitRuntime,
    title: &str,
    calls: serde_json::Value,
) -> String {
    runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            instructions: json!({ "tool_calls": calls }).to_string(),
            ..Default::default()
        })
        .expect("add task")
        .id
}

fn add_scheduled_job(runtime: &OrbitRuntime, task_id: &str, name: &str) -> String {
    runtime
        .add_job(JobAddParams {
            name: name.to_string(),
            task_id: task_id.to_string(),
            schedule_spec: "every 1s".to_string(),
            timezone: Some("UTC".to_string()),
        })
        .expect("add job")
        .job_id
}

#[test]
fn scheduled_job_run_executes_task_tool_calls_and_records_succeeded_session() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let output_file = dir.path().join("job-success.txt");

    let task_id = add_task_with_tool_calls(
        &runtime,
        "job-success",
        json!([
            {
                "name": "fs.write",
                "input": {
                    "path": output_file.to_string_lossy(),
                    "content": "ok"
                }
            }
        ]),
    );

    let job_id = add_scheduled_job(&runtime, &task_id, "success");
    let due_at = runtime
        .show_job(&job_id)
        .expect("show job")
        .next_run_at
        .expect("next run");

    let ran = runtime.run_due_jobs(due_at).expect("run jobs");
    assert_eq!(ran, 1);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, JobSessionStatus::Succeeded);
    assert_eq!(history[0].trigger.to_string(), "schedule");

    let output = std::fs::read_to_string(&output_file).expect("output");
    assert_eq!(output, "ok");
}

#[test]
fn denied_job_execution_emits_policy_denied_and_failed_session() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path())
        .expect("runtime")
        .with_policy(PolicyEngine::new_local_default_allow().deny_tool("fs.write"));

    let denied_file = dir.path().join("should-not-exist.txt");
    let task_id = add_task_with_tool_calls(
        &runtime,
        "job-denied",
        json!([
            {
                "name": "fs.write",
                "input": {
                    "path": denied_file.to_string_lossy(),
                    "content": "no"
                }
            }
        ]),
    );

    let job_id = add_scheduled_job(&runtime, &task_id, "denied");
    let due_at = runtime
        .show_job(&job_id)
        .expect("show job")
        .next_run_at
        .expect("next run");

    let ran = runtime.run_due_jobs(due_at).expect("run jobs");
    assert_eq!(ran, 1);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, JobSessionStatus::Failed);
    assert!(!denied_file.exists(), "denied job must not write file");

    let audits = runtime.list_audits(20).expect("audits");
    assert!(
        audits.iter().any(|a| a.event_type == "PolicyDenied"),
        "policy denied must be audited"
    );
}

#[test]
fn concurrent_job_run_invocations_do_not_double_run_job() {
    let dir = tempdir().expect("tempdir");
    let runtime = Arc::new(OrbitRuntime::from_data_root(dir.path()).expect("runtime"));

    let task_id = add_task_with_tool_calls(
        &runtime,
        "job-concurrent",
        json!([
            {
                "name": "proc.spawn",
                "input": {
                    "program": "sleep",
                    "args": ["0.2"],
                    "timeout_ms": 2000
                }
            }
        ]),
    );

    let job_id = add_scheduled_job(&runtime, &task_id, "concurrent");
    let due_at = runtime
        .show_job(&job_id)
        .expect("show job")
        .next_run_at
        .expect("next run");

    let barrier = Arc::new(Barrier::new(3));

    let r1 = Arc::clone(&runtime);
    let b1 = Arc::clone(&barrier);
    let due_one = due_at;
    let t1 = thread::spawn(move || {
        b1.wait();
        r1.run_due_jobs(due_one).expect("thread 1 run")
    });

    let r2 = Arc::clone(&runtime);
    let b2 = Arc::clone(&barrier);
    let due_two = due_at;
    let t2 = thread::spawn(move || {
        b2.wait();
        r2.run_due_jobs(due_two).expect("thread 2 run")
    });

    barrier.wait();

    let c1 = t1.join().expect("join t1");
    let c2 = t2.join().expect("join t2");
    assert_eq!(c1 + c2, 1, "job should be claimed exactly once");

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].status, JobSessionStatus::Succeeded);
}

#[test]
fn cancel_job_is_cooperative_and_marks_session_cancelled() {
    let dir = tempdir().expect("tempdir");
    let runtime = Arc::new(OrbitRuntime::from_data_root(dir.path()).expect("runtime"));
    let output_file = dir.path().join("cancelled-write.txt");

    let task_id = add_task_with_tool_calls(
        &runtime,
        "job-cancel",
        json!([
            {
                "name": "proc.spawn",
                "input": {
                    "program": "sleep",
                    "args": ["0.2"],
                    "timeout_ms": 2000
                }
            },
            {
                "name": "fs.write",
                "input": {
                    "path": output_file.to_string_lossy(),
                    "content": "should-not-write"
                }
            }
        ]),
    );

    let job_id = add_scheduled_job(&runtime, &task_id, "cancel-me");
    let due_at = runtime
        .show_job(&job_id)
        .expect("show job")
        .next_run_at
        .expect("next run");

    let runner = {
        let runtime = Arc::clone(&runtime);
        thread::spawn(move || runtime.run_due_jobs(due_at).expect("run jobs"))
    };

    let mut observed_running = false;
    for _ in 0..80 {
        let history = runtime.job_history(&job_id).expect("history");
        if history
            .iter()
            .any(|session| session.status == JobSessionStatus::Running)
        {
            observed_running = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        observed_running,
        "expected to observe running session before cancel"
    );

    let cancelled_session = runtime.cancel_job(&job_id).expect("cancel job");
    let ran = runner.join().expect("runner join");
    assert_eq!(ran, 1);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].session_id, cancelled_session);
    assert_eq!(history[0].status, JobSessionStatus::Cancelled);
    assert!(history[0].cancel_requested_at.is_some());
    assert!(!output_file.exists(), "second tool call should be skipped");

    let entries = runtime
        .list_entries(EntityType::Job, &job_id)
        .expect("job entries");
    assert!(
        entries
            .iter()
            .any(|entry| { entry.body.contains("job cancellation requested: session=") })
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.body.contains("status=cancelled")),
        "completion entry should record cancelled status"
    );
}
