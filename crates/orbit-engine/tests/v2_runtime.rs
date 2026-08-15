#![allow(missing_docs)]
// Integration fixtures exercise public behavior and unwrap setup invariants.
#![allow(
    clippy::expect_used,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::unwrap_used
)]

//! v2 runtime integration coverage: the deterministic reference activity
//! dispatches through a stub `RuntimeHost` and persists its §7 envelope
//! events.
//!
//! Runs under `cargo nextest run -p orbit-engine --test v2_runtime`.

use std::path::PathBuf;
use std::sync::Arc;

use orbit_agent::loop_engine::InMemorySink;
use orbit_common::types::activity_job::{ActivityV2, load_activity_asset};
use orbit_engine::{
    DispatchError, ResolvedCliExecutor, RuntimeHost, V2AuditWriter, V2DispatchInput, V2SqliteSink,
    dispatch_v2_activity,
};
use serde_json::Value;

#[test]
fn deterministic_reference_dispatches_and_persists_audit_events() -> Result<(), String> {
    let references_dir = workspace_root().join("crates/orbit-core/assets/activities/examples");
    let tmp_audit = tempfile::tempdir().map_err(|err| err.to_string())?;
    smoke_dispatch_deterministic(
        &references_dir.join("deterministic_reference.yaml"),
        tmp_audit.path(),
    )
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
        fs_profile: asset.spec.fs_profile.as_deref(),
        input: Value::Null,
        audit: writer.clone(),
        run_id,
        host: Some(&host),
    })
    .map_err(|e| format!("dispatch: {e}"))?;

    if !outcome.success {
        return Err(format!("deterministic returned non-success: {outcome:?}"));
    }
    assert_sqlite_nonempty(&envelope)?;
    Ok(())
}

struct EchoHost;

impl RuntimeHost for EchoHost {
    fn run_deterministic(
        &self,
        action: &str,
        config: &Value,
        input: &Value,
        _tool_context: orbit_tools::ToolContext,
    ) -> Result<Value, DispatchError> {
        Ok(serde_json::json!({
            "action": action,
            "config": config,
            "input": input,
            "echo": "deterministic smoke stub"
        }))
    }

    fn resolve_cli_executor(&self, _provider: &str) -> Result<ResolvedCliExecutor, DispatchError> {
        Err(DispatchError::CliInvocationFailed(
            "EchoHost has no CLI provider mapping".into(),
        ))
    }

    fn tool_context_for_activity(
        &self,
        _run_id: Option<&str>,
        _fs_profile: Option<&str>,
        _fs_audit: Option<std::sync::Arc<dyn orbit_tools::FsAuditLogger>>,
        _proc_allowed_programs: Option<&[String]>,
    ) -> orbit_tools::ToolContext {
        orbit_tools::ToolContext::default()
    }
}

fn build_writer_and_sinks(
    audit_root: &std::path::Path,
    run_id: &str,
) -> (Arc<V2AuditWriter>, Arc<V2SqliteSink>, Arc<InMemorySink>) {
    let blob_dir = audit_root.join("blobs");
    let _ = std::fs::create_dir_all(&blob_dir);
    let inner = Arc::new(InMemorySink::new(blob_dir));
    let envelope = Arc::new(V2SqliteSink::for_audit_root(
        orbit_store::Store::open_in_memory().expect("open sqlite sink"),
        "ws_smoke",
        run_id,
        "smoke-agent",
        None,
        audit_root,
    ));
    let writer = Arc::new(
        V2AuditWriter::new(run_id, "smoke-agent", inner.clone())
            .with_envelope_sink(envelope.clone()),
    );
    (writer, envelope, inner)
}

fn load_v2(yaml: &str) -> Result<V2ReferenceAsset, String> {
    match load_activity_asset(yaml) {
        Ok(a) => Ok(V2ReferenceAsset {
            name: a.name,
            spec: a.spec,
        }),
        Err(err) => Err(format!("load: {err}")),
    }
}

struct V2ReferenceAsset {
    name: String,
    spec: ActivityV2,
}

fn assert_sqlite_nonempty(sink: &V2SqliteSink) -> Result<(), String> {
    let count = sink
        .persisted_event_count()
        .map_err(|e| format!("read audit sqlite rows: {e}"))?;
    if count == 0 {
        return Err("audit sqlite rows are empty".to_string());
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
