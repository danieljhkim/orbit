use std::sync::Arc;

use orbit_agent::loop_engine::audit::{AuditSink, NullSink};
use orbit_agent::loop_engine::transport::MessageRole;
use orbit_common::types::activity_job::{Backend, OnDenial, Provider};
use orbit_common::types::{LearningInjectionCaps, LearningReminder};
use tempfile::NamedTempFile;

use super::super::*;
use super::super::{REPLAY_TEST_ENV_LOCK, reset_replay_transport};

use crate::activity_job::V2AuditWriter;
use crate::activity_job::dispatcher::DispatchError;

struct ReplayEnvGuard {
    fixture_prior: Option<String>,
}

impl ReplayEnvGuard {
    fn set_fixture(path: &std::path::Path) -> Self {
        let fixture_prior = std::env::var("ORBIT_V2_REPLAY_FIXTURE").ok();
        // SAFETY: these tests serialize all replay-env mutation with
        // REPLAY_TEST_ENV_LOCK and restore the previous value on drop.
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

struct ReplayHost;

impl V2RuntimeHost for ReplayHost {
    fn run_deterministic(
        &self,
        _action: &str,
        _config: &Value,
        _input: &Value,
        _tool_context: ToolContext,
    ) -> Result<Value, DispatchError> {
        Err(DispatchError::DeterministicActionNotRegistered(
            "replay host: not used".to_string(),
        ))
    }

    fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::AgentLoopFailed(
            "replay host: no credentials".to_string(),
        ))
    }

    fn resolve_cli_executor(
        &self,
        _provider: &str,
    ) -> Result<super::super::super::dispatcher::ResolvedCliExecutor, DispatchError> {
        Err(DispatchError::CliInvocationFailed(
            "replay host: no CLI mapping".to_string(),
        ))
    }

    fn tool_context_for_activity(
        &self,
        _run_id: Option<&str>,
        _fs_profile: Option<&str>,
        _fs_audit: Option<Arc<dyn orbit_tools::FsAuditLogger>>,
    ) -> ToolContext {
        ToolContext::default()
    }
}

struct LearningReplayHost {
    reminders: Vec<LearningReminder>,
}

impl V2RuntimeHost for LearningReplayHost {
    fn run_deterministic(
        &self,
        _action: &str,
        _config: &Value,
        _input: &Value,
        _tool_context: ToolContext,
    ) -> Result<Value, DispatchError> {
        Err(DispatchError::DeterministicActionNotRegistered(
            "learning replay host: not used".to_string(),
        ))
    }

    fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::AgentLoopFailed(
            "learning replay host: no credentials".to_string(),
        ))
    }

    fn resolve_cli_executor(
        &self,
        _provider: &str,
    ) -> Result<super::super::super::dispatcher::ResolvedCliExecutor, DispatchError> {
        Err(DispatchError::CliInvocationFailed(
            "learning replay host: no CLI mapping".to_string(),
        ))
    }

    fn learning_reminders_for_task(
        &self,
        _input: &Value,
        caps: LearningInjectionCaps,
    ) -> Result<Vec<LearningReminder>, DispatchError> {
        Ok(self.reminders.iter().take(caps.per_call).cloned().collect())
    }

    fn tool_context_for_activity(
        &self,
        _run_id: Option<&str>,
        _fs_profile: Option<&str>,
        _fs_audit: Option<Arc<dyn orbit_tools::FsAuditLogger>>,
    ) -> ToolContext {
        ToolContext::default()
    }
}

fn replay_spec(on_denial: OnDenial) -> AgentLoopSpec {
    AgentLoopSpec {
        instruction: "test".to_string(),
        tools: Vec::new(),
        on_denial,
        model: Some("test-model".to_string()),
        max_iterations: 4,
        backend: Backend::Http,
        provider: Provider::Claude,
        wall_clock_timeout_seconds: 30,
        role: None,
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

fn replay_done_fixture() -> NamedTempFile {
    write_fixture(serde_json::json!({
        "turns": [{
            "content": [{ "kind": "text", "text": "done" }],
            "stop_reason": "end_turn"
        }]
    }))
}

fn first_user_text(session: &Session) -> &str {
    let message = session.history().first().expect("first message");
    assert_eq!(message.role, MessageRole::User);
    match message.content.first().expect("first content") {
        ContentBlock::Text { text } => text,
        _ => panic!("expected text content"),
    }
}

#[test]
fn replay_denial_terminate_surfaces_structural_tool_denied() {
    let _lock = REPLAY_TEST_ENV_LOCK.lock().expect("replay env lock");
    let fixture = write_fixture(serde_json::json!({
        "turns": [{
            "content": [{
                "kind": "tool_use",
                "id": "denied-1",
                "name": "fs.delete",
                "input": { "path": "/tmp/blocked.txt" }
            }],
            "stop_reason": "tool_use"
        }]
    }));
    let _guard = ReplayEnvGuard::set_fixture(fixture.path());
    let host = ReplayHost;

    let err = drive_agent_loop(
        &replay_spec(OnDenial::Terminate),
        None,
        "run-denial-terminate",
        audit_writer("run-denial-terminate"),
        &serde_json::json!({ "prompt": "try a denied tool" }),
        &host,
        None,
    )
    .expect_err("terminate should surface structural denial");

    assert!(matches!(
        err,
        DispatchError::ToolDenied {
            ref tool_name,
            iteration: 1,
        } if tool_name == "fs.delete"
    ));
}

#[test]
fn replay_denial_continue_runs_until_normal_stop() {
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
                "content": [{ "kind": "text", "text": "done" }],
                "stop_reason": "end_turn"
            }
        ]
    }));
    let _guard = ReplayEnvGuard::set_fixture(fixture.path());
    let host = ReplayHost;

    let outcome = drive_agent_loop(
        &replay_spec(OnDenial::Continue),
        None,
        "run-denial-continue",
        audit_writer("run-denial-continue"),
        &serde_json::json!({ "prompt": "try a denied tool" }),
        &host,
        None,
    )
    .expect("continue should let replay reach final turn");

    assert_eq!(outcome.final_message, "done");
    assert_eq!(outcome.trace.len(), 2);
    assert_eq!(
        outcome.trace[0].policy_denials,
        vec!["fs.delete".to_string()]
    );
    assert!(outcome.trace[1].policy_denials.is_empty());
}

#[test]
fn l1_learning_reminder_prepends_prompt_for_matching_task() {
    let _lock = REPLAY_TEST_ENV_LOCK.lock().expect("replay env lock");
    let fixture = replay_done_fixture();
    let _guard = ReplayEnvGuard::set_fixture(fixture.path());
    let host = LearningReplayHost {
        reminders: vec![LearningReminder {
            id: "L-0001".to_string(),
            summary: "Remember to validate the output.".to_string(),
            comments: Vec::new(),
        }],
    };
    let mut session = Session::new("replay", "test-model", "test", None);

    drive_agent_loop_with_session(
        &replay_spec(OnDenial::Terminate),
        None,
        "run-learning-positive",
        audit_writer("run-learning-positive"),
        &mut session,
        &serde_json::json!({"prompt": "baseline prompt"}),
        &host,
        None,
    )
    .expect("replay should finish");

    assert_eq!(
        first_user_text(&session),
        "<system-reminder>\n\
Project learnings relevant to this task:\n\n\
- [L-0001] Remember to validate the output.\n\n\
Read full body via `orbit.learning.show <id>` if needed.\n\
</system-reminder>\n\n\
baseline prompt"
    );
}

#[test]
fn l1_learning_reminder_leaves_prompt_unchanged_without_matches() {
    let _lock = REPLAY_TEST_ENV_LOCK.lock().expect("replay env lock");
    let fixture = replay_done_fixture();
    let _guard = ReplayEnvGuard::set_fixture(fixture.path());
    let host = LearningReplayHost {
        reminders: Vec::new(),
    };
    let mut session = Session::new("replay", "test-model", "test", None);

    drive_agent_loop_with_session(
        &replay_spec(OnDenial::Terminate),
        None,
        "run-learning-negative",
        audit_writer("run-learning-negative"),
        &mut session,
        &serde_json::json!({"prompt": "baseline prompt"}),
        &host,
        None,
    )
    .expect("replay should finish");

    assert_eq!(first_user_text(&session), "baseline prompt");
}

#[test]
fn l1_learning_reminder_applies_default_per_call_cap() {
    let _lock = REPLAY_TEST_ENV_LOCK.lock().expect("replay env lock");
    let fixture = replay_done_fixture();
    let _guard = ReplayEnvGuard::set_fixture(fixture.path());
    let host = LearningReplayHost {
        reminders: (0..7)
            .map(|idx| LearningReminder {
                id: format!("L-{idx:04}"),
                summary: format!("Learning {idx}"),
                comments: Vec::new(),
            })
            .collect(),
    };
    let mut session = Session::new("replay", "test-model", "test", None);

    drive_agent_loop_with_session(
        &replay_spec(OnDenial::Terminate),
        None,
        "run-learning-cap",
        audit_writer("run-learning-cap"),
        &mut session,
        &serde_json::json!({"prompt": "baseline prompt"}),
        &host,
        None,
    )
    .expect("replay should finish");

    let text = first_user_text(&session);
    assert!(text.contains("[L-0004] Learning 4"));
    assert!(!text.contains("L-0005"));
    assert_eq!(session.learning_injection_state().count, 5);
}
