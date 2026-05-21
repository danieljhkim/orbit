use super::super::job::*;
use chrono::Utc;
use orbit_common::types::JobRunState;
use orbit_core::NotFoundKind;
use orbit_core::OrbitError;
use serde_json::json;

fn test_run(state: JobRunState) -> JobRun {
    let now = Utc::now();
    JobRun {
        run_id: "jrun-test".to_string(),
        job_id: "task_gate_pipeline".to_string(),
        attempt: 1,
        state,
        scheduled_at: now,
        started_at: Some(now),
        finished_at: None,
        duration_ms: None,
        created_at: now,
        pid: None,
        pid_start_time: None,
        input: None,
        retry_source_run_id: None,
        knowledge_metrics: None,
        resolved_crew: None,
        planner_model: None,
        implementer_model: None,
        reviewer_model: None,
        steps: Vec::new(),
    }
}

fn write_replay_job(runtime: &OrbitRuntime, name: &str) -> PathBuf {
    let jobs_dir = runtime.data_root().join("resources/jobs");
    std::fs::create_dir_all(&jobs_dir).expect("create jobs dir");
    let path = jobs_dir.join(format!("{name}.yaml"));
    std::fs::write(
        &path,
        format!(
            r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  steps:
- id: nap
  spec:
    type: deterministic
    action: sleep
    config: {{}}
"#
        ),
    )
    .expect("write replay job");
    path
}

#[test]
fn job_run_json_includes_waiting_reasons_from_state() {
    let run = test_run(JobRunState::Running);
    let mut state = PipelineState::new(run.run_id.clone(), run.job_id.clone(), json!({}));
    state.set_waiting_reasons(
        Some(vec!["ORB-1".to_string()]),
        Some(vec!["file:src/lib.rs".to_string()]),
    );

    let value = job_run_to_json_with_state(&run, Some(&state));

    assert_eq!(value["waiting_on_deps"], json!(["ORB-1"]));
    assert_eq!(value["waiting_on_locks"], json!(["file:src/lib.rs"]));
}

#[test]
fn job_run_json_omits_stale_waiting_reasons_for_terminal_run() {
    let run = test_run(JobRunState::Success);
    let mut state = PipelineState::new(run.run_id.clone(), run.job_id.clone(), json!({}));
    state.set_waiting_reasons(
        Some(vec!["ORB-1".to_string()]),
        Some(vec!["file:src/lib.rs".to_string()]),
    );

    let value = job_run_to_json_with_state(&run, Some(&state));

    assert_eq!(value["waiting_on_deps"], Value::Null);
    assert_eq!(value["waiting_on_locks"], Value::Null);
}

#[test]
fn job_replay_args_execute_creates_linked_run() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let job_path = write_replay_job(&runtime, "cli_replay_success");
    let source = runtime
        .run_job_v2_from_yaml(&job_path, json!({ "seconds": 0 }), None)
        .expect("source run");

    JobReplayArgs {
        run_id: source.run_id.clone(),
        json: true,
    }
    .execute(&runtime)
    .expect("replay run");

    let history = runtime
        .job_history("cli_replay_success")
        .expect("job history");
    assert!(history.iter().any(|run| {
        run.retry_source_run_id.as_deref() == Some(source.run_id.as_str())
            && run.state == orbit_common::types::JobRunState::Success
    }));
}

#[test]
fn job_replay_args_execute_unknown_run_returns_error() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let error = JobReplayArgs {
        run_id: "jrun-missing".to_string(),
        json: true,
    }
    .execute(&runtime)
    .expect_err("unknown source run should fail");

    assert!(matches!(
        error,
        OrbitError::NotFound {
            kind: NotFoundKind::JobRun,
            ..
        }
    ));
}
