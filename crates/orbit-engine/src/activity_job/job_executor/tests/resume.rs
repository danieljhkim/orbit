#![allow(missing_docs)]

//! [ORB-10002] Checkpoint/resume behavior of the v2 DAG executor:
//! per-step checkpoints flow through `V2RuntimeHost::checkpoint_step`, and
//! `execute_job_with_resume` skips checkpointed steps while feeding their
//! recorded outputs into the pipeline for later steps.

use std::sync::Mutex as StdMutex;

use orbit_common::types::{JobRunState, PipelineState};

use super::*;

/// Host wrapper that records `checkpoint_step` calls (and can inject
/// checkpoint failures) while delegating dispatch to a `ScriptedHost`.
struct CheckpointHost {
    inner: ScriptedHost,
    checkpoints: StdMutex<Vec<(u32, String, Value, Value)>>,
    fail_checkpoints: bool,
}

impl CheckpointHost {
    fn new(inner: ScriptedHost) -> Self {
        Self {
            inner,
            checkpoints: StdMutex::new(Vec::new()),
            fail_checkpoints: false,
        }
    }

    fn failing(inner: ScriptedHost) -> Self {
        Self {
            fail_checkpoints: true,
            ..Self::new(inner)
        }
    }

    fn checkpoints(&self) -> Vec<(u32, String, Value, Value)> {
        self.checkpoints.lock().expect("checkpoints").clone()
    }
}

impl V2RuntimeHost for CheckpointHost {
    fn run_deterministic(
        &self,
        action: &str,
        config: &Value,
        input: &Value,
        tool_context: orbit_tools::ToolContext,
    ) -> Result<Value, DispatchError> {
        self.inner
            .run_deterministic(action, config, input, tool_context)
    }

    fn api_key_for(&self, provider: &str) -> Result<String, DispatchError> {
        self.inner.api_key_for(provider)
    }

    fn resolve_cli_executor(
        &self,
        provider: &str,
    ) -> Result<crate::activity_job::dispatcher::ResolvedCliExecutor, DispatchError> {
        self.inner.resolve_cli_executor(provider)
    }

    fn tool_context_for_activity(
        &self,
        run_id: Option<&str>,
        fs_profile: Option<&str>,
        fs_audit: Option<std::sync::Arc<dyn orbit_tools::FsAuditLogger>>,
        proc_allowed_programs: Option<&[String]>,
    ) -> orbit_tools::ToolContext {
        self.inner
            .tool_context_for_activity(run_id, fs_profile, fs_audit, proc_allowed_programs)
    }

    fn checkpoint_step(
        &self,
        _run_id: &str,
        step_index: u32,
        step_id: &str,
        output: &Value,
        pipeline_snapshot: &Value,
    ) -> Result<(), DispatchError> {
        if self.fail_checkpoints {
            return Err(DispatchError::JobExecution(
                "injected checkpoint failure".to_string(),
            ));
        }
        self.checkpoints.lock().expect("checkpoints").push((
            step_index,
            step_id.to_string(),
            output.clone(),
            pipeline_snapshot.clone(),
        ));
        Ok(())
    }
}

fn resume_state_with_completed_steps(steps: &[(u32, &str, Value)]) -> PipelineState {
    let mut state = PipelineState::new(
        "jrun-source".to_string(),
        "qa_resume".to_string(),
        Value::Object(Default::default()),
    );
    let mut pipeline = serde_json::Map::new();
    for (index, step_id, output) in steps {
        state.record_step(*index, JobRunState::Success, Some(output.clone()), None);
        pipeline.insert((*step_id).to_string(), output.clone());
    }
    state.sync_pipeline(Value::Object(pipeline));
    state
}

#[test]
fn completed_top_level_steps_checkpoint_through_host() {
    let host = CheckpointHost::new(ScriptedHost::new([
        ("a0", vec![Action::Ok(json!({"v": 0}))]),
        ("a1", vec![Action::Ok(json!({"v": 1}))]),
    ]));
    let job = job_with_steps(vec![target_step("s0", "a0"), target_step("s1", "a1")]);
    let writer = std::sync::Arc::new(test_writer("run-ckpt"));

    let outcome =
        execute_job(&job, Value::Null, "run-ckpt", writer, &host).expect("execute_job ok");

    assert!(outcome.success);
    let checkpoints = host.checkpoints();
    assert_eq!(checkpoints.len(), 2, "one checkpoint per top-level step");
    assert_eq!(checkpoints[0].0, 0);
    assert_eq!(checkpoints[0].1, "s0");
    assert_eq!(checkpoints[0].2, json!({"v": 0}));
    assert_eq!(checkpoints[0].3, json!({"s0": {"v": 0}}));
    assert_eq!(checkpoints[1].0, 1);
    assert_eq!(checkpoints[1].1, "s1");
    assert_eq!(
        checkpoints[1].3,
        json!({"s0": {"v": 0}, "s1": {"v": 1}}),
        "snapshot is cumulative"
    );
}

#[test]
fn failing_step_is_not_checkpointed() {
    let host = CheckpointHost::new(ScriptedHost::new([
        ("a0", vec![Action::Ok(json!({"v": 0}))]),
        (
            "a1",
            vec![Action::Err(DispatchError::JobExecution("boom".into()))],
        ),
    ]));
    let job = job_with_steps(vec![target_step("s0", "a0"), target_step("s1", "a1")]);
    let writer = std::sync::Arc::new(test_writer("run-ckpt-fail"));

    let result = execute_job(&job, Value::Null, "run-ckpt-fail", writer, &host);

    assert!(result.is_err(), "failing step propagates");
    let checkpoints = host.checkpoints();
    assert_eq!(checkpoints.len(), 1, "only the completed step checkpointed");
    assert_eq!(checkpoints[0].1, "s0");
}

#[test]
fn checkpoint_write_failure_is_non_fatal() {
    let host = CheckpointHost::failing(ScriptedHost::new([(
        "a0",
        vec![Action::Ok(json!({"v": 0}))],
    )]));
    let job = job_with_steps(vec![target_step("s0", "a0")]);
    let writer = std::sync::Arc::new(test_writer("run-ckpt-nonfatal"));

    let outcome =
        execute_job(&job, Value::Null, "run-ckpt-nonfatal", writer, &host).expect("execute_job ok");

    assert!(
        outcome.success,
        "a checkpoint persistence failure must not fail the run"
    );
}

#[test]
fn resume_skips_checkpointed_steps_and_feeds_their_outputs() {
    // Simulates the post-crash second executor: step 0 completed before the
    // interruption, steps 1..2 still pending. Step 0's action must NOT run
    // again, and its recorded output must stay visible in the pipeline.
    let host = CheckpointHost::new(ScriptedHost::new([
        ("a0", vec![Action::Ok(json!({"v": "fresh-0"}))]),
        ("a1", vec![Action::Ok(json!({"v": 1}))]),
        ("a2", vec![Action::Ok(json!({"v": 2}))]),
    ]));
    let job = job_with_steps(vec![
        target_step("s0", "a0"),
        target_step("s1", "a1"),
        target_step("s2", "a2"),
    ]);
    let resume = resume_state_with_completed_steps(&[(0, "s0", json!({"v": "checkpointed-0"}))]);
    let writer = std::sync::Arc::new(test_writer("run-resume"));

    let outcome = execute_job_with_resume(
        &job,
        Value::Null,
        "run-resume",
        writer,
        &host,
        Some(&resume),
    )
    .expect("execute_job_with_resume ok");

    assert!(outcome.success);
    assert_eq!(host.inner.call_count("a0"), 0, "step 0 must not re-execute");
    assert_eq!(host.inner.call_count("a1"), 1);
    assert_eq!(host.inner.call_count("a2"), 1);
    let pipeline = outcome.pipeline.as_object().expect("pipeline obj");
    assert_eq!(
        pipeline.get("s0"),
        Some(&json!({"v": "checkpointed-0"})),
        "skipped step's checkpointed output is fed into the pipeline"
    );
    assert_eq!(pipeline.get("s1"), Some(&json!({"v": 1})));
    assert_eq!(pipeline.get("s2"), Some(&json!({"v": 2})));
    // Remaining steps checkpoint on top of the seeded snapshot.
    let checkpoints = host.checkpoints();
    assert_eq!(checkpoints.len(), 2);
    assert_eq!(checkpoints[0].0, 1);
    assert_eq!(
        checkpoints[0].3.get("s0"),
        Some(&json!({"v": "checkpointed-0"}))
    );
}

#[test]
fn resume_with_no_completed_steps_runs_everything() {
    let host = CheckpointHost::new(ScriptedHost::new([
        ("a0", vec![Action::Ok(json!({"v": 0}))]),
        ("a1", vec![Action::Ok(json!({"v": 1}))]),
    ]));
    let job = job_with_steps(vec![target_step("s0", "a0"), target_step("s1", "a1")]);
    let resume = PipelineState::new(
        "jrun-source".to_string(),
        "qa_resume".to_string(),
        Value::Object(Default::default()),
    );
    let writer = std::sync::Arc::new(test_writer("run-resume-empty"));

    let outcome = execute_job_with_resume(
        &job,
        Value::Null,
        "run-resume-empty",
        writer,
        &host,
        Some(&resume),
    )
    .expect("execute_job_with_resume ok");

    assert!(outcome.success);
    assert_eq!(host.inner.call_count("a0"), 1);
    assert_eq!(host.inner.call_count("a1"), 1);
}

#[test]
fn resume_ignores_non_success_step_states() {
    // A step recorded as `failed` in the source run must re-execute.
    let host = CheckpointHost::new(ScriptedHost::new([(
        "a0",
        vec![Action::Ok(json!({"v": 0}))],
    )]));
    let job = job_with_steps(vec![target_step("s0", "a0")]);
    let mut resume = PipelineState::new(
        "jrun-source".to_string(),
        "qa_resume".to_string(),
        Value::Object(Default::default()),
    );
    resume.record_step(0, JobRunState::Failed, Some(json!({"v": "stale"})), None);
    let writer = std::sync::Arc::new(test_writer("run-resume-failed-step"));

    let outcome = execute_job_with_resume(
        &job,
        Value::Null,
        "run-resume-failed-step",
        writer,
        &host,
        Some(&resume),
    )
    .expect("execute_job_with_resume ok");

    assert!(outcome.success);
    assert_eq!(
        host.inner.call_count("a0"),
        1,
        "failed step must re-execute on resume"
    );
}
