//! `orbit job run <yaml-path>` — schemaVersion 2 job entrypoint.
//!
//! Mirrors `activity_v2::run_activity_v2_from_yaml`: reads the YAML, routes
//! through the two-pass loader, and dispatches via the Phase 3 DAG executor.
//! orbit-core never names orbit-agent types — transport/session construction
//! lives below the boundary in `orbit_engine::job_executor`.

use std::path::Path;

use orbit_common::types::activity_job::{
    Backend, V2AuditEventKind, load_job_asset, resolve_job_backends,
    validate_job_loop_session_backends,
};
use orbit_common::types::{
    JobRun, JobRunState, JobTargetType, NotFoundKind, OrbitError, OrbitEvent, PipelineState,
};
use orbit_engine::{
    JobOutcome, V2AuditWriter, dispatch_error_to_orbit, execute_job_with_resume,
    resolve_job_catalog_refs_for_execution,
};
use orbit_store::{JobRunStepParams, TaskReservationReleaseReason};
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::command::SYSTEM_AUDIT_IDENTITY;

#[derive(Debug, Clone)]
pub struct V2JobRunResult {
    pub run_id: String,
    pub job_name: String,
    pub success: bool,
    pub pipeline: Value,
    pub message: Option<String>,
    pub events_emitted: usize,
    /// Resolved backend applied at load time to every `agent_loop` step in
    /// the DAG. Recorded so smokes can inspect the precedence outcome.
    pub resolved_backend: Backend,
}

impl OrbitRuntime {
    /// Execute a v2 Job from a YAML file. Returns a structural result. The
    /// file must declare `schemaVersion: 2` and `kind: Job`; v1 files are
    /// rejected.
    pub fn run_job_v2_from_yaml(
        &self,
        yaml_path: &Path,
        input: Value,
        backend_flag: Option<Backend>,
    ) -> Result<V2JobRunResult, OrbitError> {
        self.run_job_v2_from_yaml_with_retry_source(yaml_path, input, backend_flag, None, 1, None)
    }

    /// Re-run a completed or historical job run from step 0 using the current
    /// catalog definition and the source run's persisted input.
    pub fn replay_job_run(&self, source_run_id: &str) -> Result<V2JobRunResult, OrbitError> {
        let source = self.show_job_run(source_run_id)?;
        let input = source.input.clone().unwrap_or_else(|| json!({}));
        let (job_path, _) = self.load_v2_job_asset_by_name(&source.job_id)?;
        self.run_job_v2_from_yaml_with_retry_source(
            &job_path,
            input,
            None,
            Some(source.run_id.clone()),
            1,
            None,
        )
    }

    /// [ORB-10002] Resume an interrupted (or failed / timed-out) job run from
    /// its persisted step checkpoints.
    ///
    /// Creates a new run linked via `retry_source_run_id`, seeds its
    /// `PipelineState` from the source run's checkpoints, and executes the
    /// job skipping every top-level step already recorded as `success` —
    /// their outputs are fed back into the pipeline so later steps see them.
    /// If the source run has no successful checkpoints this degrades to a
    /// full replay.
    pub fn resume_job_run(&self, source_run_id: &str) -> Result<V2JobRunResult, OrbitError> {
        // `show_job_run` reconciles a stale Running owner first, so a run
        // orphaned by SIGKILL flips to Interrupted before the state guard.
        let source = self.show_job_run(source_run_id)?;
        if !matches!(
            source.state,
            JobRunState::Interrupted | JobRunState::Failed | JobRunState::Timeout
        ) {
            return Err(OrbitError::JobValidation(format!(
                "job run '{}' is {} — resume requires an interrupted, failed, or timed-out run",
                source_run_id, source.state
            )));
        }
        let input = source.input.clone().unwrap_or_else(|| json!({}));
        let resume_state = self.read_run_state(source_run_id)?.filter(|state| {
            state
                .step_states
                .values()
                .any(|step_state| *step_state == JobRunState::Success)
        });
        let (job_path, _) = self.load_v2_job_asset_by_name(&source.job_id)?;
        self.run_job_v2_from_yaml_with_retry_source(
            &job_path,
            input,
            None,
            Some(source.run_id.clone()),
            source.attempt.saturating_add(1),
            resume_state,
        )
    }

    fn run_job_v2_from_yaml_with_retry_source(
        &self,
        yaml_path: &Path,
        input: Value,
        backend_flag: Option<Backend>,
        retry_source_run_id: Option<String>,
        attempt: u32,
        resume: Option<PipelineState>,
    ) -> Result<V2JobRunResult, OrbitError> {
        let job_name = load_job_name(yaml_path)?;
        let scheduled_at = chrono::Utc::now();
        let run = self.stores().jobs().insert_job_run(
            &job_name,
            attempt,
            scheduled_at,
            Some(input.clone()),
            retry_source_run_id.clone(),
        )?;
        // [ORB-10002] A resumed run starts from the source run's checkpoint
        // state (re-keyed to the new run) instead of a blank pipeline.
        let initial_state = match &resume {
            Some(source_state) => {
                let mut seeded = source_state.clone();
                seeded.run_id = run.run_id.clone();
                seeded.job_id = run.job_id.clone();
                seeded.updated_at = chrono::Utc::now();
                seeded
            }
            None => PipelineState::new(run.run_id.clone(), run.job_id.clone(), input.clone()),
        };
        self.stores()
            .jobs()
            .write_run_state(&run.run_id, &initial_state)?;

        let started_at = chrono::Utc::now();
        let changed = self.stores().jobs().mark_job_run_running(
            &run.run_id,
            started_at,
            std::process::id(),
        )?;
        if !changed {
            return Err(OrbitError::not_found(NotFoundKind::JobRun, run.run_id));
        }
        self.record_run_crew_from_input(&run.run_id, &input)?;
        self.record_event(OrbitEvent::JobRunStarted {
            job_id: run.job_id.clone(),
            run_id: run.run_id.clone(),
            attempt: run.attempt,
        })?;

        let outcome = self.run_job_v2_from_yaml_with_run_context(
            yaml_path,
            input.clone(),
            backend_flag,
            Some(run.run_id.clone()),
            retry_source_run_id,
            resume.as_ref(),
        );
        let finished_at = chrono::Utc::now();
        let duration_ms = Some(
            finished_at
                .signed_duration_since(started_at)
                .num_milliseconds()
                .max(0) as u64,
        );

        match outcome {
            Ok(result) => {
                let final_state = if result.success {
                    JobRunState::Success
                } else {
                    JobRunState::Failed
                };
                self.persist_direct_v2_run_state(&run, &input, &result, final_state)?;
                if result.success {
                    self.record_direct_v2_success_step(&run, started_at, finished_at, &result)?;
                } else {
                    let fallback = "job completed with success=false but emitted no failure detail";
                    let message = result.message.as_deref().unwrap_or(fallback);
                    let _ =
                        self.record_pipeline_failure_step(&run, started_at, finished_at, message);
                }
                self.finalize_job_run_with_reservation_cleanup(
                    &run.run_id,
                    final_state,
                    finished_at,
                    duration_ms,
                    TaskReservationReleaseReason::RunTerminal,
                )?;
                self.record_event(OrbitEvent::JobRunCompleted {
                    job_id: run.job_id.clone(),
                    run_id: run.run_id.clone(),
                    state: final_state.to_string(),
                })?;
                Ok(result)
            }
            Err(error) => {
                let _ = self.record_pipeline_failure_step(
                    &run,
                    started_at,
                    finished_at,
                    &error.to_string(),
                );
                self.finalize_job_run_with_reservation_cleanup(
                    &run.run_id,
                    JobRunState::Failed,
                    finished_at,
                    duration_ms,
                    TaskReservationReleaseReason::RunTerminal,
                )?;
                self.record_event(OrbitEvent::JobRunCompleted {
                    job_id: run.job_id.clone(),
                    run_id: run.run_id.clone(),
                    state: JobRunState::Failed.to_string(),
                })?;
                Err(error)
            }
        }
    }

    pub fn run_job_v2_from_yaml_with_run_id(
        &self,
        yaml_path: &Path,
        input: Value,
        backend_flag: Option<Backend>,
        run_id_override: Option<String>,
    ) -> Result<V2JobRunResult, OrbitError> {
        self.run_job_v2_from_yaml_with_run_context(
            yaml_path,
            input,
            backend_flag,
            run_id_override,
            None,
            None,
        )
    }

    fn run_job_v2_from_yaml_with_run_context(
        &self,
        yaml_path: &Path,
        input: Value,
        backend_flag: Option<Backend>,
        run_id_override: Option<String>,
        retry_source_run_id: Option<String>,
        resume: Option<&PipelineState>,
    ) -> Result<V2JobRunResult, OrbitError> {
        let yaml = std::fs::read_to_string(yaml_path).map_err(|err| {
            OrbitError::InvalidInput(format!("read {}: {err}", yaml_path.display()))
        })?;
        let mut asset = load_job_asset(&yaml).map_err(|err| {
            OrbitError::InvalidInput(format!("load {}: {err}", yaml_path.display()))
        })?;

        // Phase 4: resolve `target: activity:<name>` refs before any other
        // pass, so backend-resolution + loader-rejection see concrete specs.
        let catalog = self
            .v2_activity_catalog()
            .map_err(|err| OrbitError::InvalidInput(format!("build activity catalog: {err}")))?;
        resolve_job_catalog_refs_for_execution(&mut asset.spec, &catalog)
            .map_err(dispatch_error_to_orbit)?;

        // §3.1 resolution: replace every `Auto` with a concrete backend.
        let resolution = self.resolve_v2_backend(backend_flag);
        resolve_job_backends(&mut asset.spec, resolution.backend);

        // §3.2 loader rejection: any `loop:`-nested step with `session:`
        // binding must resolve to `backend: http`. We reject at load time so
        // CLI-mode runs never start a DAG they can't finish.
        validate_job_loop_session_backends(&asset.spec, &yaml_path.display().to_string())
            .map_err(|err| OrbitError::InvalidInput(format!("{err}")))?;
        let run_id = run_id_override.unwrap_or_else(|| {
            format!(
                "job-{}-{}",
                asset.name,
                chrono::Utc::now().format("%Y%m%dT%H%M%S%.3f")
            )
        });

        let audit_root = self.paths().audit_dir.clone();
        let workspace_path = self.paths().repo_root.clone();
        let writer = V2AuditWriter::with_disk_sinks(
            &audit_root,
            self.sqlite_store()?,
            self.workspace_id()?,
            &run_id,
            SYSTEM_AUDIT_IDENTITY,
            Some(workspace_path.as_path()),
        )
        .map_err(|err| OrbitError::Execution(format!("audit sinks: {err}")))?;
        self.record_event(OrbitEvent::ActivityRunStarted {
            id: asset.name.clone(),
        })?;
        let _ = writer.emit(V2AuditEventKind::RunStarted {
            job_name: format!("cli:{}", asset.name),
            retry_source_run_id,
        });

        let outcome_res: Result<JobOutcome, OrbitError> =
            execute_job_with_resume(&asset.spec, input, &run_id, writer.clone(), self, resume)
                .map_err(|err| OrbitError::Execution(format!("v2 job dispatch: {err}")));

        let (outcome_str, error_message) = match &outcome_res {
            Ok(o) if o.success => ("success", None),
            Ok(o) => ("failed", o.message.clone()),
            Err(err) => ("error", Some(err.to_string())),
        };
        let _ = writer.emit(V2AuditEventKind::RunFinished {
            outcome: outcome_str.to_string(),
            error_message,
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
                run_id,
                job_name: asset.name,
                success: o.success,
                pipeline: o.pipeline,
                message: o.message,
                events_emitted: events_count,
                resolved_backend: resolution.backend,
            }),
            Err(err) => Err(err),
        }
    }

    fn persist_direct_v2_run_state(
        &self,
        run: &JobRun,
        input: &Value,
        result: &V2JobRunResult,
        final_state: JobRunState,
    ) -> Result<(), OrbitError> {
        let mut state = self.read_run_state(&run.run_id)?.unwrap_or_else(|| {
            PipelineState::new(run.run_id.clone(), run.job_id.clone(), input.clone())
        });
        state.sync_pipeline(result.pipeline.clone());
        // [ORB-10002] Per-step checkpoints already maintain step records for
        // this run; only fall back to the legacy single-summary step record
        // when no checkpoint was ever written, so a later `resume` never sees
        // a whole-run summary clobbering step 0's real checkpoint.
        if state.step_states.is_empty() {
            state.record_step(0, final_state, Some(result.pipeline.clone()), None);
        }
        self.stores().jobs().write_run_state(&run.run_id, &state)
    }

    fn record_direct_v2_success_step(
        &self,
        run: &JobRun,
        started_at: chrono::DateTime<chrono::Utc>,
        finished_at: chrono::DateTime<chrono::Utc>,
        result: &V2JobRunResult,
    ) -> Result<(), OrbitError> {
        let duration_ms = Some(
            finished_at
                .signed_duration_since(started_at)
                .num_milliseconds()
                .max(0) as u64,
        );
        self.stores().jobs().complete_job_run_step(
            &run.run_id,
            &JobRunStepParams {
                step_index: 0,
                target_type: JobTargetType::Job,
                target_id: run.job_id.clone(),
                started_at,
                finished_at,
                duration_ms,
                exit_code: Some(0),
                agent_response_json: Some(result.pipeline.clone()),
                state: JobRunState::Success,
                error_code: None,
                error_message: None,
            },
        )?;
        Ok(())
    }
}

fn load_job_name(yaml_path: &Path) -> Result<String, OrbitError> {
    let yaml = std::fs::read_to_string(yaml_path)
        .map_err(|err| OrbitError::InvalidInput(format!("read {}: {err}", yaml_path.display())))?;
    let asset = load_job_asset(&yaml)
        .map_err(|err| OrbitError::InvalidInput(format!("load {}: {err}", yaml_path.display())))?;
    Ok(asset.name)
}
