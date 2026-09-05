//! Regression coverage for the shared job-run JSON projection.

use chrono::Utc;
use orbit_types::workflow::{
    ChildDispatch, ChildDispatchPhase, JobRun, JobRunState, PipelineState,
};
use serde_json::{Value, json};

use super::super::projection::job_run_to_json;

fn test_run(state: JobRunState) -> JobRun {
    let now = Utc::now();
    JobRun {
        run_id: "jrun-projection".to_string(),
        job_id: "task_auto_pipeline".to_string(),
        attempt: 1,
        state,
        scheduled_at: now,
        started_at: Some(now),
        finished_at: None,
        duration_ms: None,
        created_at: now,
        pid: Some(42),
        pid_start_time: None,
        input: None,
        retry_source_run_id: None,
        knowledge_metrics: None,
        resolved_crew: None,
        crew_model: None,
        steps: Vec::new(),
    }
}

fn state_with_child_dispatch(run: &JobRun) -> PipelineState {
    let mut state = PipelineState::new(run.run_id.clone(), run.job_id.clone(), json!({}));
    state.set_waiting_reasons(
        Some(vec!["ORB-1".to_string()]),
        Some(vec!["file:src/lib.rs".to_string()]),
    );
    state.record_child_dispatch(
        ChildDispatch::submitted(
            "jrun-child".to_string(),
            "task_auto_pipeline".to_string(),
            "invoke_and_wait".to_string(),
            true,
            false,
            Utc::now(),
        )
        .with_parent_step_id(Some("ship".to_string())),
    );
    state.advance_child_dispatch("jrun-child", ChildDispatchPhase::Waiting, None, None);
    state
}

#[test]
fn active_run_projects_waiting_reasons_and_child_dispatches() {
    let run = test_run(JobRunState::Running);
    let state = state_with_child_dispatch(&run);

    let value = job_run_to_json(&run, Some(&state));

    assert_eq!(value["waiting_on_deps"], json!(["ORB-1"]));
    assert_eq!(value["waiting_on_locks"], json!(["file:src/lib.rs"]));
    assert_eq!(
        value["child_dispatches"][0]["child_run_id"],
        json!("jrun-child")
    );
}

#[test]
fn terminal_run_keeps_child_lineage_but_drops_waiting_reasons() {
    let run = test_run(JobRunState::Cancelled);
    let state = state_with_child_dispatch(&run);

    let value = job_run_to_json(&run, Some(&state));

    assert_eq!(value["waiting_on_deps"], Value::Null);
    assert_eq!(value["waiting_on_locks"], Value::Null);
    assert_eq!(
        value["child_dispatches"][0]["child_run_id"],
        json!("jrun-child")
    );
}
