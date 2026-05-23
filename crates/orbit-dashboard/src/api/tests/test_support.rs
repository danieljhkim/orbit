//! Shared test helpers reused by the api submodules.

use axum::body::to_bytes;
use axum::response::Response;
use chrono::Utc;
use orbit_core::{JobRun, JobRunState, OrbitRuntime};
use rusqlite::{Connection, params};
use serde_json::Value;

pub(super) fn write_lines(path: &std::path::Path, lines: &[String]) {
    let mut content = String::new();
    for line in lines {
        content.push_str(line);
        content.push('\n');
    }
    std::fs::write(path, content).expect("write fixture");
}

pub(super) fn write_replay_job(runtime: &OrbitRuntime, name: &str) -> std::path::PathBuf {
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

pub(super) fn seed_run(
    runtime: &OrbitRuntime,
    run_id: &str,
    job_id: &str,
    state: JobRunState,
) -> JobRun {
    let now = Utc::now();
    let run = JobRun {
        run_id: run_id.to_string(),
        job_id: job_id.to_string(),
        attempt: 1,
        state,
        scheduled_at: now,
        started_at: matches!(
            state,
            JobRunState::Running
                | JobRunState::Success
                | JobRunState::Failed
                | JobRunState::Timeout
                | JobRunState::Cancelled
        )
        .then_some(now),
        finished_at: state.is_terminal().then_some(now),
        duration_ms: state.is_terminal().then_some(0),
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
    };
    write_seeded_run(runtime, &run);
    run
}

pub(super) fn write_seeded_run(runtime: &OrbitRuntime, run: &JobRun) {
    let conn = Connection::open(runtime.global_root().join("orbit.db")).expect("open orbit db");
    let workspace_id = runtime.workspace_id().expect("workspace id");
    let input_json = run
        .input
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .expect("serialize input");
    let knowledge_metrics_json = run
        .knowledge_metrics
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .expect("serialize knowledge metrics");
    conn.execute(
        r#"INSERT OR REPLACE INTO job_runs(
            run_id, workspace_id, job_id, attempt, state, scheduled_at, started_at,
            finished_at, duration_ms, created_at, pid, pid_start_time, input_json,
            retry_source_run_id, knowledge_metrics_json, resolved_crew, planner_model,
            implementer_model, reviewer_model, pipeline_state_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, NULL)"#,
        params![
            run.run_id,
            workspace_id,
            run.job_id,
            i64::from(run.attempt),
            run.state.to_string(),
            run.scheduled_at.to_rfc3339(),
            run.started_at.map(|value| value.to_rfc3339()),
            run.finished_at.map(|value| value.to_rfc3339()),
            run.duration_ms.map(|value| value as i64),
            run.created_at.to_rfc3339(),
            run.pid.map(i64::from),
            run.pid_start_time,
            input_json,
            run.retry_source_run_id,
            knowledge_metrics_json,
            run.resolved_crew,
            run.planner_model,
            run.implementer_model,
            run.reviewer_model,
        ],
    )
    .expect("insert job run");
}

pub(super) async fn body_json(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("json response")
}
