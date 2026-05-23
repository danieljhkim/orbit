use std::sync::Arc;

use orbit_agent::loop_engine::audit::{AuditSink, NullSink};
use orbit_common::groundhog::Chronicle;
use orbit_common::types::activity_job::{GroundhogSpec, OnDenial, Provider};
use orbit_common::types::{TaskPlanCheckpoint, TaskPlanSuccessCriterion};
use orbit_tools::ToolContext;
use serde_json::{Value, json};
use tempfile::NamedTempFile;

use super::super::super::agent_loop_driver::{REPLAY_TEST_ENV_LOCK, reset_replay_transport};
use super::super::super::dispatcher::V2RuntimeHost;
use super::super::attempt::{AttemptGroundhogHost, AttemptResult, run_attempt};

use crate::activity_job::V2AuditWriter;
use crate::activity_job::dispatcher::DispatchError;

struct ReplayEnvGuard {
    fixture_prior: Option<String>,
}

impl ReplayEnvGuard {
    fn set_fixture(path: &std::path::Path) -> Self {
        let fixture_prior = std::env::var("ORBIT_V2_REPLAY_FIXTURE").ok();
        // SAFETY: this test serializes replay-env mutation with
        // REPLAY_TEST_ENV_LOCK and restores the previous value on drop.
        unsafe {
            std::env::set_var("ORBIT_V2_REPLAY_FIXTURE", path);
        }
        reset_replay_transport();
        Self { fixture_prior }
    }
}

impl Drop for ReplayEnvGuard {
    fn drop(&mut self) {
        reset_replay_transport();
        // SAFETY: see ReplayEnvGuard::set_fixture.
        unsafe {
            match &self.fixture_prior {
                Some(value) => std::env::set_var("ORBIT_V2_REPLAY_FIXTURE", value),
                None => std::env::remove_var("ORBIT_V2_REPLAY_FIXTURE"),
            }
        }
    }
}

struct AttemptHost;

impl V2RuntimeHost for AttemptHost {
    fn run_deterministic(
        &self,
        _action: &str,
        _config: &Value,
        _input: &Value,
        _tool_context: ToolContext,
    ) -> Result<Value, DispatchError> {
        Err(DispatchError::DeterministicActionNotRegistered(
            "attempt host: not used".to_string(),
        ))
    }

    fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::AgentLoopFailed(
            "attempt host: no credentials".to_string(),
        ))
    }

    fn resolve_cli_executor(
        &self,
        _provider: &str,
    ) -> Result<crate::activity_job::dispatcher::ResolvedCliExecutor, DispatchError> {
        Err(DispatchError::CliInvocationFailed(
            "attempt host: no CLI mapping".to_string(),
        ))
    }

    fn tool_context_for_activity(
        &self,
        _run_id: Option<&str>,
        _fs_profile: Option<&str>,
        _fs_audit: Option<Arc<dyn orbit_tools::FsAuditLogger>>,
        _proc_allowed_programs: Option<&[String]>,
    ) -> ToolContext {
        ToolContext::default()
    }
}

fn audit_writer(run_id: &str) -> Arc<V2AuditWriter> {
    let inner: Arc<dyn AuditSink> = Arc::new(NullSink);
    Arc::new(V2AuditWriter::new(run_id, "test-agent", inner))
}

fn write_fixture(value: Value) -> NamedTempFile {
    let file = NamedTempFile::new().expect("fixture temp file");
    std::fs::write(
        file.path(),
        serde_json::to_vec(&value).expect("serialize fixture"),
    )
    .expect("write fixture");
    file
}

fn groundhog_spec(on_denial: OnDenial) -> GroundhogSpec {
    GroundhogSpec {
        instruction: String::new(),
        tools: Vec::new(),
        on_denial,
        model: Some("test-model".to_string()),
        max_iterations: 5,
        provider: Provider::Claude,
        wall_clock_timeout_seconds: 30,
        attempt_budget_default: 1,
        role: None,
        proc_allowed_programs: None,
    }
}

fn checkpoint() -> TaskPlanCheckpoint {
    TaskPlanCheckpoint {
        id: "ckpt_1".to_string(),
        spec: "finish the checkpoint".to_string(),
        success_criteria: vec![TaskPlanSuccessCriterion::Semantic {
            statement: "checkpoint is complete".to_string(),
        }],
        attempt_budget: 1,
    }
}

#[test]
fn groundhog_attempt_continue_on_denial_reaches_terminal_tool() {
    let _lock = REPLAY_TEST_ENV_LOCK.lock().expect("replay env lock");
    let fixture = write_fixture(serde_json::json!({
        "turns": [
            {
                "content": [{
                    "kind": "tool_use",
                    "id": "denied-1",
                    "name": "fs.delete",
                    "input": { "path": "/tmp/blocked.txt" }
                }],
                "stop_reason": "tool_use"
            },
            {
                "content": [{
                    "kind": "tool_use",
                    "id": "success-1",
                    "name": "orbit.groundhog.checkpoint_success",
                    "input": {
                        "summary": "checkpoint complete",
                        "side_effects": []
                    }
                }],
                "stop_reason": "tool_use"
            },
            {
                "content": [{ "kind": "text", "text": "done" }],
                "stop_reason": "end_turn"
            }
        ]
    }));
    let _guard = ReplayEnvGuard::set_fixture(fixture.path());
    let host = AttemptHost;
    let checkpoint = checkpoint();
    let groundhog_host = Arc::new(AttemptGroundhogHost::new("T-test", &checkpoint.id));

    let result = run_attempt(
        &host,
        &groundhog_spec(OnDenial::Continue),
        "run-groundhog-denial-continue",
        audit_writer("run-groundhog-denial-continue"),
        &json!({}),
        None,
        "checkpoint plan",
        &Chronicle::new("T-test".to_string(), "plan-test".to_string()),
        &checkpoint,
        None,
        groundhog_host,
    )
    .expect("denial continue should let Groundhog attempt finish");

    match result {
        AttemptResult::Success {
            summary,
            side_effects,
        } => {
            assert_eq!(summary, "checkpoint complete");
            assert!(side_effects.is_empty());
        }
        AttemptResult::Failure(report) => {
            panic!("expected success, got failure: {}", report.what_happened);
        }
    }
}
