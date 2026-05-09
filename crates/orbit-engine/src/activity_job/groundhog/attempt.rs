use std::sync::{Arc, Mutex};

use orbit_common::groundhog::{Chronicle, DayOutcome, FailureReport, SideEffect};
use orbit_common::types::activity_job::{AgentLoopSpec, GroundhogSpec};
use orbit_common::types::{OrbitError, TaskPlanCheckpoint};
use orbit_tools::{GroundhogBuiltinAction, GroundhogScope, GroundhogToolHost};
use serde::Deserialize;
use serde_json::{Value, json};

use super::super::agent_loop_driver::drive_agent_loop_with_tool_context;
use super::super::audit_writer::V2AuditWriter;
use super::super::dispatcher::{DispatchError, V2RuntimeHost, v2_fs_audit_logger};

const REQUIRED_GROUNDHOG_TOOLS: [&str; 3] = [
    "orbit.groundhog.checkpoint_success",
    "orbit.groundhog.checkpoint_failure",
    "orbit.groundhog.side_effect",
];

#[allow(clippy::too_many_arguments)]
pub(super) fn run_attempt(
    host: &dyn V2RuntimeHost,
    spec: &GroundhogSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    _input: &Value,
    fs_profile: Option<&str>,
    raw_plan: &str,
    chronicle: &Chronicle,
    checkpoint: &TaskPlanCheckpoint,
    latest_failure_report: Option<&FailureReport>,
    groundhog_host: Arc<AttemptGroundhogHost>,
) -> Result<AttemptResult, DispatchError> {
    let mut tool_ctx = host.tool_context_for_activity(
        Some(run_id),
        fs_profile,
        Some(v2_fs_audit_logger(audit.clone())),
    );
    tool_ctx.groundhog_host = Some(groundhog_host.clone());

    let loop_input = json!({
        "prompt": build_attempt_prompt(raw_plan, chronicle, checkpoint, latest_failure_report),
    });
    let attempt_spec = build_attempt_spec(spec);
    let api_key = host.api_key_for("anthropic").ok();
    let _ = drive_agent_loop_with_tool_context(
        &attempt_spec,
        api_key.as_deref(),
        run_id,
        audit,
        &loop_input,
        tool_ctx,
    )?;

    match groundhog_host.terminal() {
        Some(TerminalVerb::Success {
            summary,
            side_effects,
        }) => Ok(AttemptResult::Success {
            summary,
            side_effects: merge_side_effects(&groundhog_host.side_effects(), &side_effects),
        }),
        Some(TerminalVerb::Failure(report)) => Ok(AttemptResult::Failure(report)),
        Some(TerminalVerb::Unsupported(reason)) => {
            Ok(AttemptResult::Failure(synthetic_failure_report(reason)))
        }
        None => Ok(AttemptResult::Failure(synthetic_failure_report(
            "attempt ended without emitting a Groundhog terminal verb",
        ))),
    }
}

fn build_attempt_spec(spec: &GroundhogSpec) -> AgentLoopSpec {
    let mut attempt_spec = spec.as_agent_loop_spec();
    attempt_spec.tools = merged_tool_allowlist(&spec.tools);
    attempt_spec.instruction = if spec.instruction.trim().is_empty() {
        groundhog_system_instruction().to_string()
    } else {
        format!(
            "{}\n\n{}",
            groundhog_system_instruction(),
            spec.instruction.trim()
        )
    };
    attempt_spec
}

fn groundhog_system_instruction() -> &'static str {
    "You are executing one Groundhog v1 checkpoint attempt. Work only on the current checkpoint, use the provided tools, and terminate the attempt by calling orbit.groundhog.checkpoint_success or orbit.groundhog.checkpoint_failure."
}

fn merged_tool_allowlist(extra_tools: &[String]) -> Vec<String> {
    let mut merged = extra_tools.to_vec();
    for required in REQUIRED_GROUNDHOG_TOOLS {
        if !merged.iter().any(|entry| entry == required) {
            merged.push(required.to_string());
        }
    }
    merged
}

fn build_attempt_prompt(
    raw_plan: &str,
    chronicle: &Chronicle,
    checkpoint: &TaskPlanCheckpoint,
    latest_failure_report: Option<&FailureReport>,
) -> String {
    let mut prompt = String::new();
    prompt.push_str("Task plan:\n");
    prompt.push_str(raw_plan.trim());
    prompt.push_str("\n\nChronicle so far (successful checkpoints only):\n");

    let mut successful = false;
    for day in &chronicle.days {
        if matches!(day.outcome, DayOutcome::Success) {
            successful = true;
            prompt.push_str(&format!(
                "- {}: {}\n",
                day.checkpoint_id,
                day.summary.trim()
            ));
        }
    }
    if !successful {
        prompt.push_str("- none yet\n");
    }

    prompt.push_str("\nCurrent checkpoint:\n");
    prompt.push_str(&format!("id: {}\n", checkpoint.id));
    prompt.push_str(&format!("spec: {}\n", checkpoint.spec));
    prompt.push_str("success_criteria:\n");
    for criterion in &checkpoint.success_criteria {
        prompt.push_str(&format!("- {:?}\n", criterion));
    }

    prompt.push_str("\nRetry context:\n");
    if let Some(report) = latest_failure_report {
        prompt.push_str(&format!(
            "what_tried: {}\nwhat_happened: {}\nnext_attempt_plan: {}\n",
            report.what_tried, report.what_happened, report.next_attempt_plan
        ));
    } else {
        prompt.push_str("none\n");
    }

    prompt.push_str(
        "\nImportant: use orbit.groundhog.checkpoint_success only when the checkpoint is complete. Use orbit.groundhog.checkpoint_failure when the attempt should end failed. Do not continue chatting after your terminal tool call.\n",
    );
    prompt
}

fn merge_side_effects(recorded: &[SideEffect], reported: &[SideEffect]) -> Vec<SideEffect> {
    let mut merged = Vec::new();
    for effect in recorded.iter().chain(reported.iter()) {
        if !merged.iter().any(|existing: &SideEffect| {
            existing.kind == effect.kind
                && existing.target == effect.target
                && existing.reversible == effect.reversible
        }) {
            merged.push(effect.clone());
        }
    }
    merged
}

fn synthetic_failure_report(message: impl Into<String>) -> FailureReport {
    let message = message.into();
    FailureReport {
        what_tried: "completed a Groundhog attempt".to_string(),
        what_happened: message,
        next_attempt_plan:
            "Retry the checkpoint from a clean workspace snapshot with a narrower, more direct plan."
                .to_string(),
    }
}

#[derive(Debug, Clone)]
pub(super) enum AttemptResult {
    Success {
        summary: String,
        side_effects: Vec<SideEffect>,
    },
    Failure(FailureReport),
}

#[derive(Debug, Clone)]
enum TerminalVerb {
    Success {
        summary: String,
        side_effects: Vec<SideEffect>,
    },
    Failure(FailureReport),
    Unsupported(String),
}

#[derive(Debug, Clone, Default, Deserialize)]
struct SuccessPayload {
    summary: String,
    side_effects: Vec<SideEffect>,
}

pub(super) struct AttemptGroundhogHost {
    scope: GroundhogScope,
    state: Mutex<AttemptGroundhogState>,
}

#[derive(Default)]
struct AttemptGroundhogState {
    side_effects: Vec<SideEffect>,
    terminal: Option<TerminalVerb>,
}

impl AttemptGroundhogHost {
    pub(super) fn new(task_id: &str, checkpoint_id: &str) -> Self {
        Self {
            scope: GroundhogScope {
                active_day: true,
                task_id: Some(task_id.to_string()),
                checkpoint_id: Some(checkpoint_id.to_string()),
            },
            state: Mutex::new(AttemptGroundhogState::default()),
        }
    }

    fn terminal(&self) -> Option<TerminalVerb> {
        self.state
            .lock()
            .expect("groundhog attempt mutex poisoned")
            .terminal
            .clone()
    }

    fn side_effects(&self) -> Vec<SideEffect> {
        self.state
            .lock()
            .expect("groundhog attempt mutex poisoned")
            .side_effects
            .clone()
    }

    fn set_terminal(&self, terminal: TerminalVerb) -> Result<(), OrbitError> {
        let mut state = self.state.lock().expect("groundhog attempt mutex poisoned");
        if state.terminal.is_some() {
            return Err(OrbitError::Execution(
                "Groundhog attempt already recorded a terminal verb".to_string(),
            ));
        }
        state.terminal = Some(terminal);
        Ok(())
    }
}

impl GroundhogToolHost for AttemptGroundhogHost {
    fn execute(&self, action: GroundhogBuiltinAction, input: Value) -> Result<Value, OrbitError> {
        match action {
            GroundhogBuiltinAction::SideEffect => {
                let effect: SideEffect = serde_json::from_value(input).map_err(|error| {
                    OrbitError::InvalidInput(format!("parse groundhog side effect: {error}"))
                })?;
                let mut state = self.state.lock().expect("groundhog attempt mutex poisoned");
                state.side_effects.push(effect);
                Ok(json!({ "recorded": true }))
            }
            GroundhogBuiltinAction::CheckpointSuccess => {
                let payload: SuccessPayload = serde_json::from_value(input).map_err(|error| {
                    OrbitError::InvalidInput(format!("parse groundhog success payload: {error}"))
                })?;
                self.set_terminal(TerminalVerb::Success {
                    summary: payload.summary,
                    side_effects: payload.side_effects,
                })?;
                Ok(json!({ "recorded": true }))
            }
            GroundhogBuiltinAction::CheckpointFailure => {
                let report: FailureReport = serde_json::from_value(input).map_err(|error| {
                    OrbitError::InvalidInput(format!("parse groundhog failure payload: {error}"))
                })?;
                self.set_terminal(TerminalVerb::Failure(report))?;
                Ok(json!({ "recorded": true }))
            }
            GroundhogBuiltinAction::CheckpointDeviate => {
                self.set_terminal(TerminalVerb::Unsupported(
                    "checkpoint_deviate is not supported in Groundhog v1".to_string(),
                ))?;
                Ok(json!({ "recorded": true, "supported": false }))
            }
        }
    }

    fn scope(&self) -> GroundhogScope {
        self.scope.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use orbit_agent::loop_engine::audit::{AuditSink, NullSink};
    use orbit_common::types::activity_job::{OnDenial, Provider};
    use orbit_common::types::{TaskPlanCheckpoint, TaskPlanSuccessCriterion};
    use orbit_tools::ToolContext;
    use serde_json::{Value, json};
    use tempfile::NamedTempFile;

    use super::super::super::agent_loop_driver::{REPLAY_TEST_ENV_LOCK, reset_replay_transport};
    use super::*;

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
        ) -> Result<super::super::super::dispatcher::ResolvedCliExecutor, DispatchError> {
            Err(DispatchError::CliInvocationFailed(
                "attempt host: no CLI mapping".to_string(),
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
}
