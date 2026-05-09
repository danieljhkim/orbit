//! Free-function driver for v2 agent_loop dispatch.
//!
//! Owns the transport / session / `AgentLoop::run` construction so orbit-core
//! never names orbit-agent types. Phase 3 adds a session-reuse sibling
//! (`drive_agent_loop_with_session`) for loop-body steps that need a `Session`
//! to persist across iterations, and surfaces `ToolDenied` as a structural
//! `DispatchError` so the DAG retry wrapper can classify denials as
//! non-retryable.
//!
//! Offline replay: `ORBIT_V2_REPLAY=tool_denial` replays a single canned
//! tool_use on turn 1 (Phase 2 denial smoke). `ORBIT_V2_REPLAY_FIXTURE=<path>`
//! reads a JSON array of `ReplayTurn`-shaped objects and scripts an arbitrary
//! multi-turn sequence — used by the Phase 3 loop sample to drive convergence
//! across iterations without credentials.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use orbit_agent::loop_engine::{
    AgentLoop, AgentLoopConfig, AgentLoopError, ContentBlock, LoopOutcome, LoopTransport,
    ReplayTransport, ReplayTurn, Session, StopReason,
};
use orbit_agent::providers::anthropic::AnthropicMessagesTransport;
use orbit_common::types::activity_job::AgentLoopSpec;
use orbit_tools::ToolContext;
use serde_json::Value;

use super::audit_writer::V2AuditWriter;
use super::dispatcher::{DispatchError, V2RuntimeHost, v2_fs_audit_logger};
use super::tool_enforcement::EnforcedAuditSink;

const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-5";

/// Drive a v2 agent_loop activity end-to-end with a fresh `Session`.
///
/// Compatibility signature preserved from Phase 2b — callers that don't need
/// session persistence use this entry. Construct a `Session`, dispatch, drop.
pub fn drive_agent_loop(
    spec: &AgentLoopSpec,
    api_key: Option<&str>,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    input: &Value,
    host: &dyn V2RuntimeHost,
    fs_profile: Option<&str>,
) -> Result<LoopOutcome, DispatchError> {
    let model = resolve_model(spec);
    let provider = expected_provider();
    let mut session = Session::new(provider, model.clone(), &spec.instruction, None);
    let tool_ctx = host.tool_context_for_activity(
        Some(run_id),
        fs_profile,
        Some(v2_fs_audit_logger(audit.clone())),
    );
    drive_inner(spec, api_key, run_id, audit, &mut session, input, tool_ctx)
}

/// Drive a v2 agent_loop activity reusing an existing `Session`.
///
/// Phase 3 loop bodies pass the same `Session` across iterations so the
/// provider conversation history persists (§2: named `session:` bindings).
/// The caller owns the `Session`'s lifetime; this function never drops it.
#[allow(clippy::too_many_arguments)]
pub fn drive_agent_loop_with_session(
    spec: &AgentLoopSpec,
    api_key: Option<&str>,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    session: &mut Session,
    input: &Value,
    host: &dyn V2RuntimeHost,
    fs_profile: Option<&str>,
) -> Result<LoopOutcome, DispatchError> {
    let tool_ctx = host.tool_context_for_activity(
        Some(run_id),
        fs_profile,
        Some(v2_fs_audit_logger(audit.clone())),
    );
    drive_inner(spec, api_key, run_id, audit, session, input, tool_ctx)
}

/// Drive a v2 agent_loop activity with a caller-supplied ToolContext.
///
/// Groundhog uses this entry to attach an in-memory Groundhog verb host to the
/// per-attempt tool context while reusing the shared HTTP loop driver.
pub fn drive_agent_loop_with_tool_context(
    spec: &AgentLoopSpec,
    api_key: Option<&str>,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    input: &Value,
    tool_ctx: ToolContext,
) -> Result<LoopOutcome, DispatchError> {
    let model = resolve_model(spec);
    let provider = expected_provider();
    let mut session = Session::new(provider, model.clone(), &spec.instruction, None);
    drive_inner(spec, api_key, run_id, audit, &mut session, input, tool_ctx)
}

fn drive_inner(
    spec: &AgentLoopSpec,
    api_key: Option<&str>,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    session: &mut Session,
    input: &Value,
    tool_ctx: ToolContext,
) -> Result<LoopOutcome, DispatchError> {
    let model = resolve_model(spec);
    let user_prompt = user_prompt_from_input(input)?;

    if replay_active() {
        // Reuse the same ReplayTransport across calls so the cursor advances
        // through the scripted turns. Loop-body steps reuse the Session and
        // need the transport's state to persist too, else every iteration
        // would replay the same first turn.
        let transport = acquire_replay_transport(&model)?;
        run_loop(
            spec,
            run_id,
            audit,
            session,
            &*transport,
            &user_prompt,
            tool_ctx,
        )
    } else {
        let key = api_key.ok_or_else(|| {
            DispatchError::AgentLoopFailed(
                "no provider credential available — host.api_key_for returned an error".to_string(),
            )
        })?;
        if key.is_empty() {
            return Err(DispatchError::AgentLoopFailed(
                "api_key is empty".to_string(),
            ));
        }
        let transport = AnthropicMessagesTransport::new(key.to_string(), model.clone())
            .map_err(|err| DispatchError::AgentLoopFailed(format!("transport: {err}")))?;
        run_loop(
            spec,
            run_id,
            audit,
            session,
            &transport,
            &user_prompt,
            tool_ctx,
        )
    }
}

fn user_prompt_from_input(input: &Value) -> Result<String, DispatchError> {
    match input {
        Value::Object(map) => match map.get("prompt") {
            Some(prompt) => prompt_text(prompt),
            None => prompt_text(input),
        },
        _ => prompt_text(input),
    }
}

fn prompt_text(value: &Value) -> Result<String, DispatchError> {
    match value {
        Value::Null => Ok(String::new()),
        Value::String(text) => Ok(text.clone()),
        other => serde_json::to_string(other)
            .map_err(|err| DispatchError::AgentLoopFailed(format!("serialize prompt: {err}"))),
    }
}

fn resolve_model(spec: &AgentLoopSpec) -> String {
    spec.model
        .clone()
        .unwrap_or_else(|| DEFAULT_ANTHROPIC_MODEL.to_string())
}

fn expected_provider() -> &'static str {
    if replay_active() {
        "replay"
    } else {
        "anthropic"
    }
}

fn replay_active() -> bool {
    std::env::var("ORBIT_V2_REPLAY").is_ok() || std::env::var("ORBIT_V2_REPLAY_FIXTURE").is_ok()
}

/// Process-global replay transport. Constructed lazily from env on first
/// use so turn cursor persists across multiple `drive_*` calls from the
/// same job run (required by loop-body steps scripted over multi-turn
/// fixtures). Cleared by `reset_replay_transport` in tests.
static REPLAY_TRANSPORT: OnceLock<Mutex<Option<Arc<ReplayTransport>>>> = OnceLock::new();

#[cfg(test)]
pub(crate) static REPLAY_TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

fn acquire_replay_transport(model: &str) -> Result<Arc<ReplayTransport>, DispatchError> {
    let cell = REPLAY_TRANSPORT.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().expect("replay mutex poisoned");
    if let Some(t) = guard.as_ref() {
        return Ok(Arc::clone(t));
    }
    let t = Arc::new(build_replay_transport(model)?);
    *guard = Some(Arc::clone(&t));
    Ok(t)
}

/// Clear the cached replay transport. Call from smokes that run multiple
/// fixture-backed jobs back-to-back under the same process.
pub fn reset_replay_transport() {
    if let Some(cell) = REPLAY_TRANSPORT.get() {
        *cell.lock().expect("replay mutex poisoned") = None;
    }
}

fn build_replay_transport(model: &str) -> Result<ReplayTransport, DispatchError> {
    if let Ok(path) = std::env::var("ORBIT_V2_REPLAY_FIXTURE") {
        let turns = load_replay_fixture(Path::new(&path))?;
        return Ok(ReplayTransport::new("replay", model.to_string(), turns));
    }
    if std::env::var("ORBIT_V2_REPLAY").ok().as_deref() == Some("tool_denial") {
        let turns = vec![ReplayTurn {
            content: vec![ContentBlock::ToolUse {
                id: "toolu_orbit_v2_replay".to_string(),
                name: "fs.delete".to_string(),
                input: serde_json::json!({"path": "/tmp/blocked.txt"}),
            }],
            stop_reason: StopReason::ToolUse,
        }];
        return Ok(ReplayTransport::new("replay", model.to_string(), turns));
    }
    Err(DispatchError::AgentLoopFailed(
        "replay env vars not set".into(),
    ))
}

fn load_replay_fixture(path: &Path) -> Result<Vec<ReplayTurn>, DispatchError> {
    let bytes = std::fs::read(path).map_err(|err| {
        DispatchError::AgentLoopFailed(format!("read replay fixture {}: {err}", path.display()))
    })?;
    // Fixture shape: { "turns": [ { "content": [...], "stop_reason": "..." }, ... ] }
    let raw: FixtureFile = serde_json::from_slice(&bytes)
        .map_err(|err| DispatchError::AgentLoopFailed(format!("parse replay fixture: {err}")))?;
    raw.turns
        .into_iter()
        .map(|t| t.into_replay_turn())
        .collect()
}

#[derive(serde::Deserialize)]
struct FixtureFile {
    turns: Vec<FixtureTurn>,
}

#[derive(serde::Deserialize)]
struct FixtureTurn {
    /// List of content blocks. Each must be one of:
    ///   { "kind": "text", "text": "..." }
    ///   { "kind": "tool_use", "id": "...", "name": "...", "input": {...} }
    content: Vec<FixtureBlock>,
    /// "end_turn" | "tool_use" | "max_tokens".
    stop_reason: String,
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum FixtureBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

impl FixtureTurn {
    fn into_replay_turn(self) -> Result<ReplayTurn, DispatchError> {
        let stop_reason = match self.stop_reason.as_str() {
            "end_turn" => StopReason::EndTurn,
            "tool_use" => StopReason::ToolUse,
            "max_tokens" => StopReason::MaxTokens,
            other => {
                return Err(DispatchError::AgentLoopFailed(format!(
                    "unknown replay stop_reason: {other}"
                )));
            }
        };
        let content = self
            .content
            .into_iter()
            .map(|b| match b {
                FixtureBlock::Text { text } => ContentBlock::Text { text },
                FixtureBlock::ToolUse { id, name, input } => {
                    ContentBlock::ToolUse { id, name, input }
                }
            })
            .collect();
        Ok(ReplayTurn {
            content,
            stop_reason,
        })
    }
}

fn run_loop<T: LoopTransport>(
    spec: &AgentLoopSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    session: &mut Session,
    transport: &T,
    user_prompt: &str,
    tool_ctx: ToolContext,
) -> Result<LoopOutcome, DispatchError> {
    let mut registry = orbit_tools::ToolRegistry::new();
    registry.register_builtins();

    let cfg = AgentLoopConfig::new_for_run(run_id)
        .with_allowlist(spec.tools.clone())
        .with_advertised_tools(spec.tools.clone())
        .with_max_iterations(spec.max_iterations.max(1))
        .with_max_total_tokens(u64::MAX)
        .with_wall_clock_timeout(Duration::from_secs(spec.wall_clock_timeout_seconds.max(1)))
        .with_on_denial(spec.on_denial);

    let session_id = session.id().to_string();

    let inner = audit.inner_sink();
    let enforced =
        EnforcedAuditSink::new(inner, spec.tools.clone(), audit.clone(), run_id, session_id);

    let res = AgentLoop::run(
        session,
        &cfg,
        transport,
        &registry,
        &tool_ctx,
        &enforced,
        user_prompt,
    );
    match res {
        Ok(outcome) => Ok(outcome),
        // §4.3 classifies denial as non-retryable. Surface structurally so the
        // Phase 3 retry wrapper can skip retry. The §7 `tool.denied` audit
        // event was already emitted by `EnforcedAuditSink` before this point.
        Err(AgentLoopError::PolicyDenied {
            tool_name,
            iteration,
        }) => Err(DispatchError::ToolDenied {
            tool_name,
            iteration,
        }),
        Err(err) => Err(DispatchError::AgentLoopFailed(format!("{err:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use orbit_agent::loop_engine::audit::{AuditSink, NullSink};
    use orbit_common::types::activity_job::{Backend, OnDenial, Provider};
    use tempfile::NamedTempFile;

    use super::*;

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
        ) -> Result<super::super::dispatcher::ResolvedCliExecutor, DispatchError> {
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
}
