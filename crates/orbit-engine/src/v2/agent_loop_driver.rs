//! Free-function driver for v2 agent_loop dispatch.
//!
//! Owns the transport / session / `AgentLoop::run` construction that used to
//! live in `orbit-core::runtime::v2_host` (Phase 2b/2c). Moving this code
//! down into orbit-engine — where the orbit-agent dependency already exists —
//! means orbit-core no longer needs to name orbit-agent types, and the
//! `agent_reexports` workaround (Phase 2c) can be deleted.
//!
//! The one piece orbit-core still owns is credential sourcing: we accept an
//! `api_key: &str` here and let the `V2RuntimeHost::api_key_for` trait method
//! pull it from env/config wherever is appropriate.

use std::sync::Arc;
use std::time::Duration;

use orbit_agent::loop_engine::{
    AgentLoop, AgentLoopConfig, AgentLoopError, ContentBlock, LoopOutcome, LoopTransport,
    ReplayTransport, ReplayTurn, Session, StopReason, TerminateReason, TurnUsage,
};
use orbit_agent::providers::anthropic::AnthropicMessagesTransport;
use orbit_types::v2::AgentLoopSpec;
use serde_json::Value;

use super::audit_writer::V2AuditWriter;
use super::dispatcher::DispatchError;
use super::tool_enforcement::EnforcedAuditSink;

const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-5";

/// Drive a v2 agent_loop activity end-to-end: build transport/session, wrap
/// the audit sink with tool-allowlist enforcement, and call `AgentLoop::run`.
///
/// When `ORBIT_V2_REPLAY=tool_denial` is set the transport swaps to a
/// `ReplayTransport` that returns a scripted `fs.write` tool_use on turn 1,
/// exercising the denial path without network or credentials. In that mode
/// `api_key` is ignored and may be `None` — useful for offline smokes where
/// the caller knows no credential is needed.
pub fn drive_agent_loop(
    spec: &AgentLoopSpec,
    api_key: Option<&str>,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    _input: &Value,
) -> Result<LoopOutcome, DispatchError> {
    let model = spec
        .model
        .clone()
        .unwrap_or_else(|| DEFAULT_ANTHROPIC_MODEL.to_string());

    if std::env::var("ORBIT_V2_REPLAY").ok().as_deref() == Some("tool_denial") {
        drive_with_replay(spec, run_id, audit, model)
    } else {
        let key = api_key.ok_or_else(|| {
            DispatchError::AgentLoopFailed(
                "no provider credential available — host.api_key_for returned an error".to_string(),
            )
        })?;
        drive_with_anthropic(spec, key, run_id, audit, model)
    }
}

fn drive_with_anthropic(
    spec: &AgentLoopSpec,
    api_key: &str,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    model: String,
) -> Result<LoopOutcome, DispatchError> {
    if api_key.is_empty() {
        return Err(DispatchError::AgentLoopFailed(
            "api_key is empty".to_string(),
        ));
    }
    let transport = AnthropicMessagesTransport::new(api_key.to_string(), model.clone())
        .map_err(|err| DispatchError::AgentLoopFailed(format!("transport: {err}")))?;
    run_loop(spec, run_id, audit, model, &transport)
}

fn drive_with_replay(
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
    run_loop(spec, run_id, audit, model, &transport)
}

fn run_loop<T: LoopTransport>(
    spec: &AgentLoopSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    model: String,
    transport: &T,
) -> Result<LoopOutcome, DispatchError> {
    let registry = orbit_tools::ToolRegistry::new();
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
        // reports success; the audit trail preserves the denial event.
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
