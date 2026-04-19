//! `orbit job run-v2 <yaml-path>` — v2 job entrypoint.
//!
//! Mirrors `activity_v2::run_activity_v2_from_yaml`: reads the YAML, routes
//! through the two-pass loader, and dispatches via the Phase 3 DAG executor.
//! orbit-core never names orbit-agent types — transport/session construction
//! lives below the boundary in `orbit_engine::v2::job_executor`.

use std::path::{Path, PathBuf};

use orbit_engine::v2::{JobOutcome, V2AuditWriter, execute_job};
use orbit_types::v2::{
    Backend, JobAsset, V2AuditEventKind, load_job_asset, resolve_job_backends,
    validate_job_loop_session_backends,
};
use orbit_types::{OrbitError, OrbitEvent};
use serde_json::Value;

use crate::OrbitRuntime;

pub struct V2JobRunResult {
    pub job_name: String,
    pub success: bool,
    pub pipeline: Value,
    pub message: Option<String>,
    pub audit_jsonl: Option<PathBuf>,
    pub events_emitted: usize,
    /// Resolved backend applied at load time to every `agent_loop` step in
    /// the DAG. Recorded so smokes can inspect the precedence outcome.
    pub resolved_backend: Backend,
}

impl OrbitRuntime {
    /// Execute a v2 Job from a YAML file. Returns a structural result and the
    /// path to the persisted §7 envelope JSONL. The file must declare
    /// `schemaVersion: 2` and `kind: Job`; v1 files are rejected.
    pub fn run_job_v2_from_yaml(
        &self,
        yaml_path: &Path,
        input: Value,
        backend_flag: Option<Backend>,
    ) -> Result<V2JobRunResult, OrbitError> {
        let yaml = std::fs::read_to_string(yaml_path).map_err(|err| {
            OrbitError::InvalidInput(format!("read {}: {err}", yaml_path.display()))
        })?;
        let mut asset = match load_job_asset(&yaml).map_err(|err| {
            OrbitError::InvalidInput(format!("load {}: {err}", yaml_path.display()))
        })? {
            JobAsset::V2(a) => a,
            JobAsset::V1(_) => {
                return Err(OrbitError::InvalidInput(format!(
                    "{} is a v1 asset; use `orbit job run <id>` instead",
                    yaml_path.display()
                )));
            }
        };

        // §3.1 resolution: replace every `Auto` with a concrete backend.
        let resolution = self.resolve_v2_backend(backend_flag);
        resolve_job_backends(&mut asset.spec, resolution.backend);

        // §3.2 loader rejection: any `loop:`-nested step with `session:`
        // binding must resolve to `backend: http`. We reject at load time so
        // CLI-mode runs never start a DAG they can't finish.
        validate_job_loop_session_backends(&asset.spec, &yaml_path.display().to_string())
            .map_err(|err| OrbitError::InvalidInput(format!("{err}")))?;
        let run_id = format!(
            "v2job-{}-{}",
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

        self.record_event(OrbitEvent::ActivityRunStarted {
            id: asset.name.clone(),
        })?;
        let _ = writer.emit(V2AuditEventKind::RunStarted {
            job_name: format!("cli-v2:{}", asset.name),
        });

        let outcome_res: Result<JobOutcome, OrbitError> =
            execute_job(&asset.spec, input, &run_id, writer.clone(), self)
                .map_err(|err| OrbitError::Execution(format!("v2 job dispatch: {err}")));

        let outcome_str = match &outcome_res {
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

        match outcome_res {
            Ok(o) => Ok(V2JobRunResult {
                job_name: asset.name,
                success: o.success,
                pipeline: o.pipeline,
                message: o.message,
                audit_jsonl,
                events_emitted: events_count,
                resolved_backend: resolution.backend,
            }),
            Err(err) => Err(err),
        }
    }
}
