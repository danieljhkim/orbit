//! `orbit activity run-v2 <yaml-path>` command — v2 entrypoint.
//!
//! The v2 runtime dispatches per-type via `orbit_engine::v2::dispatch_v2_activity`.
//! This module reads a YAML file from disk, parses it through the two-pass
//! loader at `orbit_types::v2::load_activity_asset`, and invokes the
//! dispatcher with `OrbitRuntime` as the `V2RuntimeHost` (the impl lives in
//! `crate::runtime::v2_host`).
//!
//! The existing `orbit activity run <id>` handler is untouched — it still
//! drives v1 assets via `orbit_engine::run_activity_direct`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use orbit_engine::v2::agent_reexports::{AuditSink, InMemorySink};
use orbit_engine::v2::{V2AuditWriter, V2DispatchInput, V2JsonlSink, dispatch_v2_activity};
use orbit_types::v2::{ActivityAsset, V2AuditEventKind, load_activity_asset};
use orbit_types::{OrbitError, OrbitEvent};
use serde_json::Value;

use crate::OrbitRuntime;

pub struct V2ActivityRunResult {
    pub activity_name: String,
    pub activity_type: &'static str,
    pub success: bool,
    pub output: Value,
    pub message: Option<String>,
    pub audit_jsonl: PathBuf,
    pub events_emitted: usize,
}

impl OrbitRuntime {
    /// Execute a v2 activity from a YAML path. Returns a structural result
    /// plus the path to the persisted §7 envelope JSONL.
    pub fn run_activity_v2_from_yaml(
        &self,
        yaml_path: &Path,
        input: Value,
    ) -> Result<V2ActivityRunResult, OrbitError> {
        let yaml = std::fs::read_to_string(yaml_path).map_err(|err| {
            OrbitError::InvalidInput(format!("read {}: {err}", yaml_path.display()))
        })?;
        let asset = match load_activity_asset(&yaml).map_err(|err| {
            OrbitError::InvalidInput(format!("load {}: {err}", yaml_path.display()))
        })? {
            ActivityAsset::V2(a) => a,
            ActivityAsset::V1(_) => {
                return Err(OrbitError::InvalidInput(format!(
                    "{} is a v1 asset; use `orbit activity run <id>` instead",
                    yaml_path.display()
                )));
            }
        };

        let run_id = format!(
            "v2-{}-{}",
            asset.name,
            chrono::Utc::now().format("%Y%m%dT%H%M%S%.3f")
        );

        // Audit sinks: loop-level events go to an in-memory sink backed by a
        // blob store on disk; §7 envelope events go to a JSONL sink under
        // the workspace audit root.
        let audit_root = self.data_root().join("audit");
        let blob_dir = audit_root.join("blobs");
        std::fs::create_dir_all(&blob_dir)
            .map_err(|err| OrbitError::Execution(format!("create blob dir: {err}")))?;
        let loop_sink: Arc<dyn AuditSink> = Arc::new(InMemorySink::new(blob_dir));
        let envelope_sink = Arc::new(
            V2JsonlSink::open(&audit_root, &run_id)
                .map_err(|err| OrbitError::Execution(format!("open v2 jsonl: {err}")))?,
        );
        let audit_jsonl_path = envelope_sink.log_path().to_path_buf();
        let agent_identity = self.actor().label.clone();
        let writer = Arc::new(
            V2AuditWriter::new(&run_id, agent_identity, loop_sink.clone())
                .with_envelope_sink(envelope_sink.clone()),
        );

        // Record the standard orbit-core activity-run lifecycle events so v2
        // runs appear in the same audit stream v1 runs use.
        self.record_event(OrbitEvent::ActivityRunStarted {
            id: asset.name.clone(),
        })?;
        let _ = writer.emit(V2AuditEventKind::RunStarted {
            job_name: format!("cli-v2:{}", asset.name),
        });

        let activity_type = match &asset.spec.spec {
            orbit_types::v2::ActivityV2Spec::AgentLoop(_) => "agent_loop",
            orbit_types::v2::ActivityV2Spec::Deterministic(_) => "deterministic",
            orbit_types::v2::ActivityV2Spec::Shell(_) => "shell",
        };

        let dispatch = dispatch_v2_activity(V2DispatchInput {
            activity_name: &asset.name,
            spec: &asset.spec.spec,
            input,
            audit: writer.clone(),
            run_id: &run_id,
            host: Some(self),
        });

        let outcome_str = match &dispatch {
            Ok(o) if o.success => "success",
            Ok(_) => "failed",
            Err(_) => "error",
        };
        let _ = writer.emit(V2AuditEventKind::RunFinished {
            outcome: outcome_str.to_string(),
        });
        self.record_event(OrbitEvent::ActivityRunCompleted {
            id: asset.name.clone(),
            state: outcome_str.to_string(),
        })?;

        let events_count = writer
            .events_snapshot()
            .map(|s| s.len())
            .unwrap_or_default();

        match dispatch {
            Ok(o) => Ok(V2ActivityRunResult {
                activity_name: asset.name,
                activity_type,
                success: o.success,
                output: o.output,
                message: o.message,
                audit_jsonl: audit_jsonl_path,
                events_emitted: events_count,
            }),
            Err(err) => Err(OrbitError::Execution(format!("v2 dispatch: {err}"))),
        }
    }
}
