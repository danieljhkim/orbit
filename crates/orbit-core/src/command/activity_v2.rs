//! `orbit activity run-v2 <yaml-path>` command — v2 entrypoint.
//!
//! Reads a YAML file from disk, parses it through the two-pass loader at
//! `orbit_types::v2::load_activity_asset`, and invokes the dispatcher with
//! `OrbitRuntime` as the `V2RuntimeHost` (impl lives in
//! `crate::runtime::v2_host`).
//!
//! Loop + envelope audit sink construction is delegated to
//! `V2AuditWriter::with_disk_sinks` — this file never names orbit-agent types.
//!
//! The existing `orbit activity run <id>` handler is untouched — it still
//! drives v1 assets via `orbit_engine::run_activity_direct`.

use std::path::{Path, PathBuf};

use orbit_engine::v2::{V2AuditWriter, V2DispatchInput, dispatch_v2_activity};
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
    pub audit_jsonl: Option<PathBuf>,
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

        let audit_root = self.data_root().join("audit");
        let agent_identity = self.actor().label.clone();
        let workspace_path = self.paths().repo_root.clone();
        let writer = V2AuditWriter::with_disk_sinks(
            &audit_root,
            &run_id,
            agent_identity,
            Some(workspace_path.as_path()),
        )
        .map_err(|err| OrbitError::Execution(format!("audit sinks: {err}")))?;
        let audit_jsonl = writer.envelope_log_path();

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
                audit_jsonl,
                events_emitted: events_count,
            }),
            Err(err) => Err(OrbitError::Execution(format!("v2 dispatch: {err}"))),
        }
    }
}
