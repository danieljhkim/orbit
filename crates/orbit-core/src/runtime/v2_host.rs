//! `impl V2RuntimeHost for OrbitRuntime` — the orbit-core side of the v2
//! dispatch boundary landed in Phase 2b (T20260418-2052).
//!
//! The trait itself is defined in `orbit_engine::v2::dispatcher`. This module
//! supplies the three pieces the dispatcher delegates back to orbit-core:
//!
//! 1. `run_agent_loop` — construct an `AnthropicMessagesTransport` (api key
//!    from `ANTHROPIC_API_KEY`), a `Session`, and an `EnforcedAuditSink`; then
//!    drive `AgentLoop::run`.
//! 2. `run_deterministic` — dispatch the named action. For `orbit_tool_call`
//!    (the canonical deterministic action shipped with Phase 2a), route
//!    through `OrbitRuntime::run_tool`, which reuses the same ToolContext
//!    builder v1 uses (AC5).
//! 3. `run_shell` is self-contained in the dispatcher (Phase 2b) and needs
//!    no host involvement — the trait has no method for it.

use std::sync::Arc;
use std::time::Duration;

use orbit_engine::v2::agent_reexports::{
    AgentLoop, AgentLoopConfig, AgentLoopError, AnthropicMessagesTransport, ContentBlock,
    LoopOutcome, LoopTransport, ReplayTransport, ReplayTurn, Session, StopReason, TerminateReason,
    TurnUsage,
};
use orbit_engine::v2::{DispatchError, EnforcedAuditSink, V2AuditWriter, V2RuntimeHost};
use orbit_tools::ToolRegistry;
use orbit_types::Role;
use orbit_types::v2::AgentLoopSpec;
use serde_json::Value;

use crate::OrbitRuntime;

const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-6";

impl V2RuntimeHost for OrbitRuntime {
    fn run_deterministic(
        &self,
        action: &str,
        config: &Value,
        input: &Value,
    ) -> Result<Value, DispatchError> {
        match action {
            "orbit_tool_call" => {
                // The `config` block shape (see v2_deterministic_reference.yaml):
                //   config:
                //     tool_name: orbit.graph.search
                //     args: { query: "Activity" }
                //
                // We also accept the tool_name / args inline in `input` so the
                // caller can override per-invocation. Input wins.
                let tool_name = input
                    .get("tool_name")
                    .or_else(|| config.get("tool_name"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: "missing `tool_name` in config or input".to_string(),
                    })?;
                let args = input
                    .get("args")
                    .or_else(|| config.get("args"))
                    .cloned()
                    .unwrap_or(Value::Null);

                self.run_tool_with_role(tool_name, args, Role::Admin)
                    .map_err(|err| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: format!("{err}"),
                    })
            }
            other => {
                // No other deterministic actions are registered today. When
                // they are (Phase 4 migration porting v1 `automation` specs),
                // this arm will look them up in
                // `self.activity_executor_registry()`.
                Err(DispatchError::DeterministicActionNotRegistered(
                    other.to_string(),
                ))
            }
        }
    }

    fn run_agent_loop(
        &self,
        spec: &AgentLoopSpec,
        run_id: &str,
        audit: Arc<V2AuditWriter>,
        _input: &Value,
    ) -> Result<LoopOutcome, DispatchError> {
        let model = spec
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_ANTHROPIC_MODEL.to_string());

        // Transport: build either a real Anthropic HTTP transport or a
        // deterministic `ReplayTransport` when `ORBIT_V2_REPLAY` is set
        // (offline smoke path). The replay path returns a single tool_use
        // block requesting `fs.write`, which exercises the allowlist-denial
        // trail without network or credentials.
        let outcome = if std::env::var("ORBIT_V2_REPLAY").ok().as_deref() == Some("tool_denial") {
            drive_loop_with_replay(spec, run_id, audit, model)?
        } else {
            drive_loop_with_anthropic(spec, run_id, audit, model)?
        };
        Ok(outcome)
    }
}

fn drive_loop_with_anthropic(
    spec: &AgentLoopSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    model: String,
) -> Result<LoopOutcome, DispatchError> {
    let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
        DispatchError::AgentLoopFailed(
            "ANTHROPIC_API_KEY not set — export it before running a v2 agent_loop activity"
                .to_string(),
        )
    })?;
    if api_key.is_empty() {
        return Err(DispatchError::AgentLoopFailed(
            "ANTHROPIC_API_KEY is empty".to_string(),
        ));
    }
    let transport = AnthropicMessagesTransport::new(api_key, model.clone())
        .map_err(|err| DispatchError::AgentLoopFailed(format!("transport: {err}")))?;
    drive_loop(spec, run_id, audit, model, &transport)
}

fn drive_loop_with_replay(
    spec: &AgentLoopSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    model: String,
) -> Result<LoopOutcome, DispatchError> {
    let transport = ReplayTransport::new(
        "replay",
        model.clone(),
        vec![ReplayTurn {
            content: vec![ContentBlock::ToolUse {
                id: "toolu_orbit_v2_replay".to_string(),
                name: "fs.write".to_string(),
                input: serde_json::json!({"path": "/tmp/blocked.txt", "content": "x"}),
            }],
            stop_reason: StopReason::ToolUse,
        }],
    );
    drive_loop(spec, run_id, audit, model, &transport)
}

fn drive_loop<T: LoopTransport>(
    spec: &AgentLoopSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    model: String,
    transport: &T,
) -> Result<LoopOutcome, DispatchError> {
    let registry = ToolRegistry::new();
    let tool_ctx = orbit_tools::ToolContext::default();

    let cfg = AgentLoopConfig::new_for_run(run_id)
        .with_allowlist(spec.tools.clone())
        .with_advertised_tools(vec!["fs.read".into(), "fs.write".into()])
        .with_max_iterations(spec.max_iterations.max(1))
        .with_max_total_tokens(u64::MAX)
        .with_wall_clock_timeout(Duration::from_secs(300));

    let mut session = Session::new(transport.provider(), model, &spec.instruction, None);
    let session_id = session.id().to_string();

    let inner = audit.inner_sink();
    let enforced =
        EnforcedAuditSink::new(inner, spec.tools.clone(), audit.clone(), run_id, session_id);

    let res = AgentLoop::run(
        &mut session,
        &cfg,
        transport,
        &registry,
        &tool_ctx,
        &enforced,
        &spec.instruction,
    );
    match res {
        Ok(outcome) => Ok(outcome),
        // Under `on_denial: terminate` the PolicyDenied error IS the expected
        // outcome for a tool-denial smoke. Translate to Ok so the dispatcher
        // reports success (dispatch itself completed); the audit trail
        // preserves the denial.
        Err(AgentLoopError::PolicyDenied {
            tool_name,
            iteration,
        }) => Ok(LoopOutcome {
            final_message: format!("terminated: tool `{tool_name}` denied at iter {iteration}"),
            usage: TurnUsage::default(),
            terminate_reason: TerminateReason::Other,
            trace: Vec::new(),
        }),
        Err(err) => Err(DispatchError::AgentLoopFailed(format!("{err:?}"))),
    }
}
