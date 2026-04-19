//! Phase 2b v2 runtime smoke, updated for Phase 2d + Phase 3:
//!
//! 1. Shell reference — self-contained via std::process::Command.
//! 2. Deterministic reference — a stub `V2RuntimeHost` echoes the action.
//! 3. Agent_loop reference — exercised via `drive_agent_loop` under
//!    `ORBIT_V2_REPLAY=tool_denial`. Phase 3 surfaces `DispatchError::ToolDenied`
//!    structurally, so the expected result is `Err(ToolDenied)` and the §7
//!    `tool.denied` envelope event is present.
//!
//! Usage:
//!     cargo run -p orbit-engine --example v2_runtime_smoke

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use orbit_agent::loop_engine::{InMemorySink, LoopAuditEvent};
use orbit_engine::v2::{
    DispatchError, V2AuditWriter, V2DispatchInput, V2JsonlSink, V2RuntimeHost,
    agent_loop_driver::drive_agent_loop, dispatch_v2_activity,
};
use orbit_types::v2::{
    ActivityAsset, ActivityV2, ActivityV2Spec, V2AuditEventKind, load_activity_asset,
};
use serde_json::Value;
use std::env;

fn main() -> ExitCode {
    let mut failures: Vec<String> = Vec::new();

    let references_dir = workspace_root().join("crates/orbit-core/assets/activities/v2_reference");
    let tmp_audit_root = std::env::temp_dir().join("orbit-v2-smoke");
    let _ = std::fs::create_dir_all(&tmp_audit_root);

    {
        let path = references_dir.join("v2_shell_reference.yaml");
        match smoke_dispatch_shell(&path, &tmp_audit_root) {
            Ok(()) => println!("shell reference: OK"),
            Err(err) => failures.push(format!("shell reference: {err}")),
        }
    }

    {
        let path = references_dir.join("v2_deterministic_reference.yaml");
        match smoke_dispatch_deterministic(&path, &tmp_audit_root) {
            Ok(()) => println!("deterministic reference: OK"),
            Err(err) => failures.push(format!("deterministic reference: {err}")),
        }
    }

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

fn smoke_dispatch_shell(
    path: &std::path::Path,
    audit_root: &std::path::Path,
) -> Result<(), String> {
    let yaml = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let asset = load_v2(&yaml)?;

    let run_id = "smoke-shell-001";
    let (writer, envelope, _inner) = build_writer_and_sinks(audit_root, run_id);

    let _ = writer
        .emit(V2AuditEventKind::RunStarted {
            job_name: "smoke_shell".into(),
        })
        .map_err(|e| format!("audit: {e:?}"))?;

    let outcome = dispatch_v2_activity(V2DispatchInput {
        activity_name: &asset.name,
        spec: &asset.spec.spec,
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
    let (writer, envelope, _inner) = build_writer_and_sinks(audit_root, run_id);

    let host = EchoHost;
    let outcome = dispatch_v2_activity(V2DispatchInput {
        activity_name: &asset.name,
        spec: &asset.spec.spec,
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
    let (writer, envelope, inner) = build_writer_and_sinks(audit_root, run_id);

    let ActivityV2Spec::AgentLoop(agent_spec) = &asset.spec.spec else {
        return Err("not an agent_loop spec".into());
    };

    // Phase 3: ToolDenied is structural — setting the env triggers the replay
    // path, which scripts fs.write → loop denies → driver returns Err(ToolDenied).
    unsafe {
        env::set_var("ORBIT_V2_REPLAY", "tool_denial");
    }
    let result = drive_agent_loop(agent_spec, None, run_id, writer.clone(), &Value::Null);
    unsafe {
        env::remove_var("ORBIT_V2_REPLAY");
    }

    match result {
        Err(DispatchError::ToolDenied {
            tool_name,
            iteration,
        }) => {
            println!("  structural denial: tool={tool_name} iter={iteration}");
        }
        Ok(outcome) => {
            return Err(format!("expected Err(ToolDenied), got Ok: {outcome:?}"));
        }
        Err(other) => {
            return Err(format!("expected Err(ToolDenied), got {other:?}"));
        }
    }

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
            println!("  tool.denied: run_id={r} session_id={s} tool={t}");
        }
        Some((r, s, t)) => {
            return Err(format!(
                "PolicyDenial fields empty: run_id={r:?} session_id={s:?} tool={t:?}"
            ));
        }
        None => return Err("no loop-level PolicyDenial emitted".into()),
    }

    assert_jsonl_nonempty(envelope.log_path())?;
    Ok(())
}

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

    fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::AgentLoopFailed(
            "EchoHost has no credentials".into(),
        ))
    }

    fn resolve_cli_command(&self, _provider: &str) -> Result<String, DispatchError> {
        Err(DispatchError::CliInvocationFailed(
            "EchoHost has no CLI provider mapping".into(),
        ))
    }
}

fn build_writer_and_sinks(
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
    spec: ActivityV2,
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
