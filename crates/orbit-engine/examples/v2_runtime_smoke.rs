//! Phase 2b end-to-end smoke: dispatch all three v2 reference activities
//! through the real v2 runtime, exercise tool-denial enforcement, and persist
//! §7 envelope events to disk.
//!
//! Covers T20260418-2052 ACs 2, 3, 4, 7, 8 and re-covers T20260418-2010
//! ACs 4, 5, 6 that were open at Phase 2a close.
//!
//! Usage:
//!     cargo run -p orbit-engine --example v2_runtime_smoke

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use orbit_agent::loop_engine::{
    AgentLoop, AgentLoopConfig, AgentLoopError, InMemorySink, LoopAuditEvent, LoopOutcome,
    ReplayTransport, ReplayTurn, Session, StopReason, TerminateReason, TurnUsage,
};
use orbit_engine::v2::{
    DispatchError, EnforcedAuditSink, V2AuditWriter, V2DispatchInput, V2JsonlSink, V2RuntimeHost,
    dispatch_v2_activity,
};
use orbit_tools::{ToolContext, ToolRegistry};
use orbit_types::v2::{
    ActivityAsset, ActivityV2Spec, AgentLoopSpec, V2AuditEventKind, load_activity_asset,
};
use serde_json::Value;

fn main() -> ExitCode {
    let mut failures: Vec<String> = Vec::new();

    let references_dir = workspace_root().join("crates/orbit-core/assets/activities/v2_reference");
    let tmp_audit_root = std::env::temp_dir().join("orbit-v2-smoke");
    let _ = std::fs::create_dir_all(&tmp_audit_root);

    // --- 1. Shell reference: self-contained via std::process::Command.
    {
        let path = references_dir.join("v2_shell_reference.yaml");
        match smoke_dispatch_shell(&path, &tmp_audit_root) {
            Ok(()) => println!("shell reference: OK"),
            Err(err) => failures.push(format!("shell reference: {err}")),
        }
    }

    // --- 2. Deterministic reference: uses a stub V2RuntimeHost that echoes.
    {
        let path = references_dir.join("v2_deterministic_reference.yaml");
        match smoke_dispatch_deterministic(&path, &tmp_audit_root) {
            Ok(()) => println!("deterministic reference: OK"),
            Err(err) => failures.push(format!("deterministic reference: {err}")),
        }
    }

    // --- 3. Agent_loop reference: ReplayTransport + EnforcedAuditSink.
    //      Allowlist is [fs.read]; the replay returns a tool_use for
    //      `fs.write`. EnforcedAuditSink must emit `tool.denied` with
    //      populated run_id / session_id.
    {
        let path = references_dir.join("v2_agent_loop_reference.yaml");
        match smoke_dispatch_agent_loop(&path, &tmp_audit_root) {
            Ok(()) => println!("agent_loop reference (tool-denial): OK"),
            Err(err) => failures.push(format!("agent_loop reference: {err}")),
        }
    }

    if failures.is_empty() {
        println!("\nall v2 runtime smokes passed");
        ExitCode::SUCCESS
    } else {
        eprintln!("\n{} failure(s):", failures.len());
        for f in &failures {
            eprintln!("  - {f}");
        }
        ExitCode::FAILURE
    }
}

// ============================================================================
// Per-type smoke helpers.
// ============================================================================

fn smoke_dispatch_shell(
    path: &std::path::Path,
    audit_root: &std::path::Path,
) -> Result<(), String> {
    let yaml = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let asset = load_v2(&yaml)?;

    let run_id = "smoke-shell-001";
    let (writer, envelope, _inner) = build_writer_and_sinks_static(audit_root, run_id);

    let _ = writer
        .emit(V2AuditEventKind::RunStarted {
            job_name: "smoke_shell".into(),
        })
        .map_err(|e| format!("audit: {e:?}"))?;

    let outcome = dispatch_v2_activity(V2DispatchInput {
        activity_name: &asset.name,
        spec: &asset.spec_discriminator(),
        input: Value::Null,
        audit: writer.clone(),
        run_id,
        host: None,
    })
    .map_err(|e| format!("dispatch: {e}"))?;

    let _ = writer.emit(V2AuditEventKind::RunFinished {
        outcome: if outcome.success { "success" } else { "failed" }.into(),
    });

    if !outcome.success {
        return Err(format!("shell returned non-success: {outcome:?}"));
    }
    assert_jsonl_nonempty(envelope.log_path())?;
    Ok(())
}

fn smoke_dispatch_deterministic(
    path: &std::path::Path,
    audit_root: &std::path::Path,
) -> Result<(), String> {
    let yaml = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let asset = load_v2(&yaml)?;

    let run_id = "smoke-det-001";
    let (writer, envelope, _inner) = build_writer_and_sinks_static(audit_root, run_id);

    let host = EchoHost;
    let outcome = dispatch_v2_activity(V2DispatchInput {
        activity_name: &asset.name,
        spec: &asset.spec_discriminator(),
        input: Value::Null,
        audit: writer.clone(),
        run_id,
        host: Some(&host),
    })
    .map_err(|e| format!("dispatch: {e}"))?;

    if !outcome.success {
        return Err(format!("deterministic returned non-success: {outcome:?}"));
    }
    assert_jsonl_nonempty(envelope.log_path())?;
    Ok(())
}

fn smoke_dispatch_agent_loop(
    path: &std::path::Path,
    audit_root: &std::path::Path,
) -> Result<(), String> {
    let yaml = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let asset = load_v2(&yaml)?;

    let run_id = "smoke-agent-001";
    let (writer, envelope, inner) = build_writer_and_sinks_static(audit_root, run_id);

    let host = ReplayAgentLoopHost {
        inner_sink: inner.clone(),
    };

    let outcome = dispatch_v2_activity(V2DispatchInput {
        activity_name: &asset.name,
        spec: &asset.spec_discriminator(),
        input: Value::Null,
        audit: writer.clone(),
        run_id,
        host: Some(&host),
    })
    .map_err(|e| format!("dispatch: {e}"))?;

    // Inspect the events: must contain a tool.denied with populated context.
    let events = writer.events_snapshot().map_err(|e| format!("{e:?}"))?;
    let denied = events
        .iter()
        .find(|e| matches!(e.kind, V2AuditEventKind::ToolDenied { .. }));
    if denied.is_none() {
        return Err(format!(
            "no tool.denied envelope event emitted; events: {:#?}",
            events
                .iter()
                .map(|e| &e.envelope.event_type)
                .collect::<Vec<_>>()
        ));
    }

    // Also verify loop-level PolicyDenial carries populated run_id/session_id.
    let loop_events = inner.events();
    let denial = loop_events.iter().find_map(|e| match e {
        LoopAuditEvent::PolicyDenial {
            run_id,
            session_id,
            tool_name,
            ..
        } => Some((run_id.clone(), session_id.clone(), tool_name.clone())),
        _ => None,
    });
    match denial {
        Some((r, s, t)) if !r.is_empty() && !s.is_empty() => {
            println!("  tool.denied: run_id={} session_id={} tool={}", r, s, t);
        }
        Some((r, s, t)) => {
            return Err(format!(
                "PolicyDenial emitted but fields empty: run_id={:?} session_id={:?} tool={:?}",
                r, s, t
            ));
        }
        None => {
            return Err("no loop-level PolicyDenial emitted".to_string());
        }
    }

    // outcome.success is true because we wrapped the LoopOutcome — the
    // semantic "non-success" is that the loop terminated via MaxIterations
    // after the denial. For this smoke the audit trail is what matters.
    let _ = outcome;
    assert_jsonl_nonempty(envelope.log_path())?;
    Ok(())
}

// ============================================================================
// Stub V2RuntimeHost impls.
// ============================================================================

struct EchoHost;

impl V2RuntimeHost for EchoHost {
    fn run_deterministic(
        &self,
        action: &str,
        config: &Value,
        input: &Value,
    ) -> Result<Value, DispatchError> {
        Ok(serde_json::json!({
            "action": action,
            "config": config,
            "input": input,
            "echo": "deterministic smoke stub"
        }))
    }

    fn run_agent_loop(
        &self,
        _spec: &AgentLoopSpec,
        _run_id: &str,
        _audit: Arc<V2AuditWriter>,
        _input: &Value,
    ) -> Result<LoopOutcome, DispatchError> {
        Err(DispatchError::AgentLoopFailed(
            "EchoHost does not support agent_loop".into(),
        ))
    }
}

struct ReplayAgentLoopHost {
    inner_sink: Arc<InMemorySink>,
}

impl V2RuntimeHost for ReplayAgentLoopHost {
    fn run_deterministic(
        &self,
        _action: &str,
        _config: &Value,
        _input: &Value,
    ) -> Result<Value, DispatchError> {
        Err(DispatchError::DeterministicActionNotRegistered(
            "n/a".into(),
        ))
    }

    fn run_agent_loop(
        &self,
        spec: &AgentLoopSpec,
        run_id: &str,
        audit: Arc<V2AuditWriter>,
        _input: &Value,
    ) -> Result<LoopOutcome, DispatchError> {
        let transport = ReplayTransport::single_tool_use(
            "replay",
            spec.model.clone().unwrap_or_else(|| "replay-model".into()),
            "fs.write",
            "toolu_smoke_denied",
            serde_json::json!({"path": "/tmp/smoke.txt", "content": "blocked"}),
        );
        let registry = ToolRegistry::new();
        let ctx = ToolContext::default();

        let cfg = AgentLoopConfig::new_for_run(run_id)
            .with_allowlist(spec.tools.clone())
            .with_advertised_tools(vec!["fs.read".into(), "fs.write".into()])
            .with_max_iterations(2)
            .with_max_total_tokens(u64::MAX)
            .with_wall_clock_timeout(Duration::from_secs(10));

        let mut session = Session::new(
            "replay",
            spec.model.clone().unwrap_or_else(|| "replay-model".into()),
            &spec.instruction,
            None,
        );
        let session_id = session.id().to_string();

        let enforced = EnforcedAuditSink::new(
            self.inner_sink.clone(),
            spec.tools.clone(),
            audit,
            run_id,
            session_id,
        );

        let res = AgentLoop::run(
            &mut session,
            &cfg,
            &transport,
            &registry,
            &ctx,
            &enforced,
            &spec.instruction,
        );
        // PolicyDenied under on_denial: terminate is the EXPECTED outcome of
        // the tool-denial smoke — translate it to an Ok(LoopOutcome) carrying
        // the termination reason so the dispatcher reports `success: true`
        // (dispatch itself completed) while the audit trail records the
        // denial. Other loop errors propagate.
        match res {
            Ok(outcome) => Ok(outcome),
            Err(AgentLoopError::PolicyDenied {
                tool_name,
                iteration,
            }) => Ok(LoopOutcome {
                final_message: format!("terminated: tool `{tool_name}` denied at iter {iteration}"),
                usage: TurnUsage::default(),
                terminate_reason: TerminateReason::Other,
                trace: Vec::new(),
            }),
            Err(e) => Err(DispatchError::AgentLoopFailed(format!("{e:?}"))),
        }
    }
}

// ============================================================================
// Helpers.
// ============================================================================

fn build_writer_and_sinks_static(
    audit_root: &std::path::Path,
    run_id: &str,
) -> (Arc<V2AuditWriter>, Arc<V2JsonlSink>, Arc<InMemorySink>) {
    let blob_dir = audit_root.join("blobs");
    let _ = std::fs::create_dir_all(&blob_dir);
    let inner = Arc::new(InMemorySink::new(blob_dir));
    let envelope = Arc::new(V2JsonlSink::open(audit_root, run_id).expect("open v2 jsonl sink"));
    let writer = Arc::new(
        V2AuditWriter::new(run_id, "smoke-agent", inner.clone())
            .with_envelope_sink(envelope.clone()),
    );
    (writer, envelope, inner)
}

fn load_v2(yaml: &str) -> Result<V2ReferenceAsset, String> {
    match load_activity_asset(yaml) {
        Ok(ActivityAsset::V2(a)) => Ok(V2ReferenceAsset {
            name: a.name,
            spec: a.spec,
        }),
        Ok(ActivityAsset::V1(_)) => Err("parsed as v1, expected v2".into()),
        Err(err) => Err(format!("load: {err}")),
    }
}

struct V2ReferenceAsset {
    name: String,
    spec: orbit_types::ActivityV2,
}

impl V2ReferenceAsset {
    fn spec_discriminator(&self) -> ActivityV2Spec {
        self.spec.spec.clone()
    }
}

fn assert_jsonl_nonempty(path: &std::path::Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read jsonl: {e}"))?;
    if bytes.is_empty() {
        return Err(format!("jsonl at {} is empty", path.display()));
    }
    Ok(())
}

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

// Silence unused warnings from imports we only need in conditional paths.
#[allow(dead_code)]
fn _unused(_r: StopReason, _t: ReplayTurn) {}
