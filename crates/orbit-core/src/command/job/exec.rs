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
        let run = self.stores().jobs().insert_run(
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
        let changed =
            self.stores()
                .jobs()
                .mark_run_running(&run.run_id, started_at, std::process::id())?;
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
        self.stores().jobs().complete_run_step(
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use chrono::Utc;
    use orbit_common::types::{
        AuditEventStatus, ExecutorDef, ExecutorType, JobRunState, TaskPriority, TaskStatus,
        TaskType,
    };
    use orbit_engine::{DispatchError, ResolvedCliExecutor, V2RuntimeHost};
    use orbit_store::{InvocationQuery, V2AuditEventFilter};
    use orbit_tools::{FsAuditLogger, ToolContext};
    use serde_json::json;
    use tempfile::tempdir;

    use crate::command::activity::seed_default_activities;
    use crate::command::job::seed_default_jobs;
    use crate::command::task::{TaskAddParams, TaskUpdateParams};

    fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf, PathBuf) {
        let root = tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("create global root");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
        (root, runtime, repo_root, global_root)
    }

    fn seed_default_catalogs(global_root: &Path) {
        seed_default_activities(&global_root.join("resources/activities"), true)
            .expect("seed default activities");
        seed_default_jobs(&global_root.join("resources/jobs"), true).expect("seed default jobs");
    }

    fn write_context_file(repo_root: &Path, relative_path: &str) {
        let path = repo_root.join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create context parent");
        }
        std::fs::write(path, "fixture\n").expect("write context file");
    }

    fn seed_gate_task(runtime: &OrbitRuntime, repo_root: &Path, status: TaskStatus) -> String {
        write_context_file(repo_root, "src/lib.rs");
        runtime
            .add_task(TaskAddParams {
                title: format!("Gate fixture {status}"),
                description: "Fixture task for task_gate_pipeline admission.".to_string(),
                acceptance_criteria: vec!["Gate behavior is observable.".to_string()],
                plan: "Fixture execution plan.".to_string(),
                context_files: vec!["src/lib.rs".to_string()],
                workspace_path: Some(".".to_string()),
                priority: TaskPriority::Medium,
                task_type: Some(TaskType::Chore),
                status: Some(status),
                ..Default::default()
            })
            .expect("seed gate task")
            .id
    }

    fn resolved_gate_job(runtime: &OrbitRuntime) -> orbit_common::types::activity_job::JobV2 {
        let (_path, mut job) = runtime
            .load_v2_job_asset_by_name("task_gate_pipeline")
            .expect("load task gate pipeline");
        let catalog = runtime.v2_activity_catalog().expect("activity catalog");
        resolve_job_catalog_refs_for_execution(&mut job, &catalog)
            .expect("resolve task gate activities");
        job
    }

    fn execute_gate_job(
        runtime: &OrbitRuntime,
        repo_root: &Path,
        host: &dyn V2RuntimeHost,
        input: Value,
        run_id: &str,
    ) -> JobOutcome {
        try_execute_gate_job(runtime, repo_root, host, input, run_id).expect("execute gate job")
    }

    fn try_execute_gate_job(
        runtime: &OrbitRuntime,
        repo_root: &Path,
        host: &dyn V2RuntimeHost,
        input: Value,
        run_id: &str,
    ) -> Result<JobOutcome, DispatchError> {
        let job = resolved_gate_job(runtime);
        let writer = V2AuditWriter::with_disk_sinks(
            &runtime.paths().audit_dir,
            runtime
                .sqlite_store()
                .map_err(|err| DispatchError::JobExecution(format!("open audit store: {err}")))?,
            runtime.workspace_id().map_err(|err| {
                DispatchError::JobExecution(format!("resolve workspace id: {err}"))
            })?,
            run_id,
            SYSTEM_AUDIT_IDENTITY,
            Some(repo_root),
        )
        .expect("audit writer");
        execute_job_with_resume(&job, input, run_id, writer, host, None)
    }

    struct ReserveThenStatusHost<'a> {
        runtime: &'a OrbitRuntime,
        task_id: String,
        status_after_reserve: TaskStatus,
        reserve_calls: AtomicUsize,
    }

    impl ReserveThenStatusHost<'_> {
        fn reserve_calls(&self) -> usize {
            self.reserve_calls.load(Ordering::SeqCst)
        }
    }

    impl V2RuntimeHost for ReserveThenStatusHost<'_> {
        fn run_deterministic(
            &self,
            action: &str,
            config: &Value,
            input: &Value,
            tool_context: ToolContext,
        ) -> Result<Value, DispatchError> {
            let output = <OrbitRuntime as V2RuntimeHost>::run_deterministic(
                self.runtime,
                action,
                config,
                input,
                tool_context,
            )?;
            if action == "reserve_locks" && output["reserved"] == json!(true) {
                self.reserve_calls.fetch_add(1, Ordering::SeqCst);
                self.runtime
                    .update_task(
                        &self.task_id,
                        TaskUpdateParams {
                            status: Some(self.status_after_reserve),
                            execution_summary: (self.status_after_reserve == TaskStatus::Review)
                                .then(|| "Fixture reached review in a competing run.".to_string()),
                            ..Default::default()
                        },
                    )
                    .map_err(|err| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: format!("test status flip failed: {err}"),
                    })?;
            }
            Ok(output)
        }

        fn api_key_for(&self, provider: &str) -> Result<String, DispatchError> {
            <OrbitRuntime as V2RuntimeHost>::api_key_for(self.runtime, provider)
        }

        fn resolve_cli_executor(
            &self,
            provider: &str,
        ) -> Result<ResolvedCliExecutor, DispatchError> {
            <OrbitRuntime as V2RuntimeHost>::resolve_cli_executor(self.runtime, provider)
        }

        fn tool_context_for_activity(
            &self,
            run_id: Option<&str>,
            fs_profile: Option<&str>,
            fs_audit: Option<Arc<dyn FsAuditLogger>>,
            proc_allowed_programs: Option<&[String]>,
        ) -> ToolContext {
            <OrbitRuntime as V2RuntimeHost>::tool_context_for_activity(
                self.runtime,
                run_id,
                fs_profile,
                fs_audit,
                proc_allowed_programs,
            )
        }
    }

    struct ScriptedGateHost<'a> {
        runtime: &'a OrbitRuntime,
        child_status: &'static str,
        call_log: Mutex<Vec<String>>,
    }

    impl ScriptedGateHost<'_> {
        fn new<'a>(runtime: &'a OrbitRuntime, child_status: &'static str) -> ScriptedGateHost<'a> {
            ScriptedGateHost {
                runtime,
                child_status,
                call_log: Mutex::new(Vec::new()),
            }
        }

        fn call_count(&self, action: &str) -> usize {
            self.call_log
                .lock()
                .expect("call log")
                .iter()
                .filter(|recorded| recorded.as_str() == action)
                .count()
        }
    }

    impl V2RuntimeHost for ScriptedGateHost<'_> {
        fn run_deterministic(
            &self,
            action: &str,
            config: &Value,
            input: &Value,
            tool_context: ToolContext,
        ) -> Result<Value, DispatchError> {
            self.call_log
                .lock()
                .expect("call log")
                .push(action.to_string());
            match action {
                "reserve_locks" => Ok(json!({
                    "reserved": true,
                    "reservation_id": "reservation-scripted",
                    "reserved_files": ["file:src/lib.rs"],
                })),
                "invoke_and_wait" => Ok(json!({
                    "run_id": "jrun-scripted-child",
                    "status": self.child_status,
                    "error": (self.child_status != "succeeded")
                        .then_some("scripted child failure"),
                })),
                "release_locks" => Ok(json!({ "released": true })),
                "pipeline_success_guard" => <OrbitRuntime as V2RuntimeHost>::run_deterministic(
                    self.runtime,
                    action,
                    config,
                    input,
                    tool_context,
                ),
                other => Err(DispatchError::DeterministicActionNotRegistered(
                    other.to_string(),
                )),
            }
        }

        fn api_key_for(&self, provider: &str) -> Result<String, DispatchError> {
            <OrbitRuntime as V2RuntimeHost>::api_key_for(self.runtime, provider)
        }

        fn resolve_cli_executor(
            &self,
            provider: &str,
        ) -> Result<ResolvedCliExecutor, DispatchError> {
            <OrbitRuntime as V2RuntimeHost>::resolve_cli_executor(self.runtime, provider)
        }

        fn tool_context_for_activity(
            &self,
            run_id: Option<&str>,
            fs_profile: Option<&str>,
            fs_audit: Option<Arc<dyn FsAuditLogger>>,
            proc_allowed_programs: Option<&[String]>,
        ) -> ToolContext {
            <OrbitRuntime as V2RuntimeHost>::tool_context_for_activity(
                self.runtime,
                run_id,
                fs_profile,
                fs_audit,
                proc_allowed_programs,
            )
        }
    }

    fn write_job(path: &Path, name: &str, action: &str) {
        let yaml = format!(
            r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  steps:
    - id: nap
      spec:
        type: deterministic
        action: {action}
        config: {{}}
"#
        );
        std::fs::write(path, yaml).expect("write job yaml");
    }

    fn write_cli_metrics_job(path: &Path, name: &str, step_id: &str, provider: &str, model: &str) {
        let yaml = format!(
            r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  steps:
    - id: {step_id}
      spec:
        type: agent_loop
        instruction: "emit a successful Orbit envelope"
        tools: [fs.read]
        on_denial: terminate
        max_iterations: 1
        model: {model}
        backend: cli
        provider: {provider}
        wall_clock_timeout_seconds: 30
"#
        );
        std::fs::write(path, yaml).expect("write cli metrics job yaml");
    }

    fn v2_events(
        runtime: &OrbitRuntime,
        run_id: &str,
        event_type: &str,
    ) -> Vec<orbit_store::V2AuditEventRow> {
        runtime
            .list_v2_audit_events(V2AuditEventFilter {
                workspace_id: String::new(),
                run_id: Some(run_id.to_string()),
                event_type: Some(event_type.to_string()),
                ..Default::default()
            })
            .expect("list v2 audit events")
    }

    #[test]
    fn task_gate_noops_when_task_reaches_review_after_reservation() {
        let (_root, runtime, repo_root, global_root) = test_runtime();
        seed_default_catalogs(&global_root);
        let task_id = seed_gate_task(&runtime, &repo_root, TaskStatus::Backlog);
        let host = ReserveThenStatusHost {
            runtime: &runtime,
            task_id: task_id.clone(),
            status_after_reserve: TaskStatus::Review,
            reserve_calls: AtomicUsize::new(0),
        };

        let outcome = execute_gate_job(
            &runtime,
            &repo_root,
            &host,
            json!({
                "task_ids": [task_id.clone()],
                "mode": "pr",
            }),
            "jrun-gate-review-stale",
        );

        assert!(outcome.success);
        assert_eq!(host.reserve_calls(), 1);
        assert!(
            runtime
                .job_history("task_pr_pipeline")
                .expect("child history")
                .is_empty(),
            "stale gate must not submit a child PR pipeline"
        );
        let dispatch = &outcome.pipeline["dispatch_child"];
        assert_eq!(dispatch["status"], json!("succeeded"));
        assert_eq!(dispatch["skipped"], json!(true));
        let reason = dispatch["reason"].as_str().expect("stale reason");
        assert!(reason.contains(&task_id), "{reason}");
        assert!(reason.contains("review"), "{reason}");

        let audit_events = runtime
            .list_audit_events(None, None, Some(AuditEventStatus::Success), None, 32)
            .expect("audit events");
        assert!(audit_events.iter().any(|event| {
            event.command == "gate.stale_noop"
                && event
                    .arguments_json
                    .as_deref()
                    .is_some_and(|payload| payload.contains(&task_id) && payload.contains("review"))
        }));
    }

    #[test]
    fn task_gate_noops_done_task_and_releases_reservation() {
        let (_root, runtime, repo_root, global_root) = test_runtime();
        seed_default_catalogs(&global_root);
        let task_id = seed_gate_task(&runtime, &repo_root, TaskStatus::Done);

        let outcome = execute_gate_job(
            &runtime,
            &repo_root,
            &runtime,
            json!({
                "task_ids": [task_id.clone()],
                "mode": "pr",
            }),
            "jrun-gate-done-stale",
        );

        assert!(outcome.success);
        assert!(
            runtime
                .job_history("task_pr_pipeline")
                .expect("child history")
                .is_empty(),
            "done stale gate must not submit a child PR pipeline"
        );
        assert_eq!(outcome.pipeline["dispatch_child"]["skipped"], json!(true));
        let reason = outcome.pipeline["dispatch_child"]["reason"]
            .as_str()
            .expect("done stale reason");
        assert!(reason.contains(&task_id), "{reason}");
        assert!(reason.contains("done"), "{reason}");

        let locks = runtime
            .run_tool_with_context_and_role(
                "orbit.task.locks",
                json!({}),
                orbit_common::types::Role::Admin,
                ToolContext::default(),
            )
            .expect("list locks");
        assert_eq!(locks["total_reservations"], json!(0));
    }

    #[test]
    fn task_gate_dispatches_child_for_admissible_task() {
        let (_root, runtime, repo_root, global_root) = test_runtime();
        seed_default_catalogs(&global_root);
        let host = ScriptedGateHost::new(&runtime, "succeeded");

        let outcome = execute_gate_job(
            &runtime,
            &repo_root,
            &host,
            json!({
                "task_ids": ["ORB-SCRIPTED"],
                "mode": "pr",
            }),
            "jrun-gate-admissible",
        );

        assert!(outcome.success);
        assert_eq!(host.call_count("reserve_locks"), 1);
        assert_eq!(host.call_count("invoke_and_wait"), 1);
        assert_eq!(host.call_count("release_locks"), 1);
        assert_eq!(host.call_count("pipeline_success_guard"), 1);
        assert_eq!(
            outcome.pipeline["dispatch_child"]["status"],
            json!("succeeded")
        );
    }

    #[test]
    fn task_gate_child_failure_still_fails_success_guard() {
        let (_root, runtime, repo_root, global_root) = test_runtime();
        seed_default_catalogs(&global_root);
        let host = ScriptedGateHost::new(&runtime, "failed");

        let err = try_execute_gate_job(
            &runtime,
            &repo_root,
            &host,
            json!({
                "task_ids": ["ORB-SCRIPTED"],
                "mode": "pr",
            }),
            "jrun-gate-child-failed",
        )
        .expect_err("failed child should fail the gate");

        assert_eq!(host.call_count("invoke_and_wait"), 1);
        assert_eq!(host.call_count("release_locks"), 1);
        assert_eq!(host.call_count("pipeline_success_guard"), 1);
        let message = err.to_string();
        assert!(
            message.contains("task_gate_pipeline child run"),
            "{message}"
        );
        assert!(message.contains("jrun-scripted-child"), "{message}");
        assert!(message.contains("status failed"), "{message}");
    }

    #[cfg(unix)]
    fn write_fake_codex(path: &Path) {
        write_fake_cli_response(
            path,
            r#"{"type":"thread.started","thread_id":"fake"}
{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"orbit graph","aggregated_output":"","exit_code":null,"status":"in_progress"}}
{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"orbit graph","aggregated_output":"ok","exit_code":0,"status":"completed"}}
{"schemaVersion":1,"status":"success","result":{"ok":true},"error":null}
{"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":25,"output_tokens":12}}
"#,
        );
    }

    #[cfg(unix)]
    fn write_fake_cli_response(path: &Path, stdout: &str) {
        use std::os::unix::fs::PermissionsExt;

        let output_path = path.with_extension("stdout");
        std::fs::write(&output_path, stdout).expect("write fake cli stdout");
        std::fs::write(path, "#!/bin/sh\ncat >/dev/null\ncat \"$0.stdout\"\n")
            .expect("write fake cli");
        let mut permissions = std::fs::metadata(path)
            .expect("fake cli metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod fake cli");
    }

    fn seed_failed_triage_candidate(runtime: &OrbitRuntime, title: &str) -> String {
        let task = runtime
            .add_task(TaskAddParams {
                title: title.to_string(),
                description: "Fixture task blocked by a failed pipeline.".to_string(),
                status: Some(TaskStatus::Backlog),
                ..Default::default()
            })
            .expect("seed triage task");
        let run = runtime
            .stores()
            .jobs()
            .insert_run("task_pr_pipeline", 1, Utc::now(), None, None)
            .expect("insert failed pipeline run");
        runtime
            .stores()
            .jobs()
            .mark_run_running(&run.run_id, Utc::now(), std::process::id())
            .expect("mark failed pipeline run running");
        runtime
            .finalize_job_run_with_reservation_cleanup(
                &run.run_id,
                JobRunState::Failed,
                Utc::now(),
                Some(1),
                TaskReservationReleaseReason::RunTerminal,
            )
            .expect("finalize failed pipeline run");
        runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    status: Some(TaskStatus::Blocked),
                    job_run_id: Some(Some(run.run_id)),
                    ..Default::default()
                },
            )
            .expect("couple blocked task to failed run");
        task.id
    }

    #[test]
    fn direct_yaml_run_persists_history_and_run_state() {
        let (_root, runtime, repo_root, _global_root) = test_runtime();
        let yaml_path = repo_root.join("qa_sleep.yaml");
        write_job(&yaml_path, "qa_sleep", "sleep");

        let result = runtime
            .run_job_v2_from_yaml(&yaml_path, json!({ "seconds": 0 }), None)
            .expect("direct job run succeeds");

        let run = runtime.show_job_run(&result.run_id).expect("stored run");
        assert_eq!(run.job_id, "qa_sleep");
        assert_eq!(run.state, JobRunState::Success);
        assert_eq!(run.steps.len(), 1);

        let history = runtime.job_history("qa_sleep").expect("job history");
        assert!(history.iter().any(|run| run.run_id == result.run_id));

        let state = runtime
            .read_run_state(&result.run_id)
            .expect("read run state")
            .expect("persisted run state");
        assert_eq!(state.run_id, result.run_id);
        assert!(state.pipeline.get("nap").is_some());
        assert!(state.step_outputs.contains_key(&0));

        let first_event: serde_json::Value = serde_json::from_str(
            &v2_events(&runtime, &result.run_id, "run.started")
                .first()
                .expect("run.started audit event")
                .payload_json,
        )
        .expect("parse first audit event");
        assert_eq!(
            first_event
                .get("agent_identity")
                .and_then(serde_json::Value::as_str),
            Some(SYSTEM_AUDIT_IDENTITY)
        );
        assert!(!repo_root.join(".orbit/audit").exists());
    }

    #[test]
    fn direct_catalog_run_is_visible_in_history() {
        let (_root, runtime, _repo_root, global_root) = test_runtime();
        let jobs_dir = global_root.join("resources/jobs");
        std::fs::create_dir_all(&jobs_dir).expect("create jobs dir");
        let yaml_path = jobs_dir.join("qa_catalog_sleep.yaml");
        write_job(&yaml_path, "qa_catalog_sleep", "sleep");

        let catalog = runtime
            .show_job_catalog_entry("qa_catalog_sleep")
            .expect("catalog entry");
        let result = runtime
            .run_job_v2_from_yaml(&catalog.path, json!({ "seconds": 0 }), None)
            .expect("catalog job run succeeds");

        let history = runtime
            .job_history("qa_catalog_sleep")
            .expect("catalog history");
        assert!(history.iter().any(|run| run.run_id == result.run_id));
        assert!(
            runtime
                .show_job_run(&result.run_id)
                .expect("stored run")
                .run_id
                == result.run_id
        );
    }

    #[test]
    fn replay_job_run_records_lineage_and_preserves_source_bundle() {
        let (_root, runtime, _repo_root, global_root) = test_runtime();
        let jobs_dir = global_root.join("resources/jobs");
        std::fs::create_dir_all(&jobs_dir).expect("create jobs dir");
        let yaml_path = jobs_dir.join("qa_replay_sleep.yaml");
        write_job(&yaml_path, "qa_replay_sleep", "sleep");

        let catalog = runtime
            .show_job_catalog_entry("qa_replay_sleep")
            .expect("catalog entry");
        let input = json!({ "seconds": 0, "marker": "source-input" });
        let source_result = runtime
            .run_job_v2_from_yaml(&catalog.path, input.clone(), None)
            .expect("source run succeeds");
        let source_run = runtime
            .show_job_run(&source_result.run_id)
            .expect("show source");
        let before = source_run.clone();

        let replay_result = runtime
            .replay_job_run(&source_result.run_id)
            .expect("replay succeeds");

        assert_ne!(replay_result.run_id, source_result.run_id);
        assert!(replay_result.success);
        let replay_run = runtime
            .show_job_run(&replay_result.run_id)
            .expect("show replay");
        assert_eq!(replay_run.job_id, source_run.job_id);
        assert_eq!(replay_run.input, Some(input));
        assert_eq!(
            replay_run.retry_source_run_id.as_deref(),
            Some(source_result.run_id.as_str())
        );
        assert_eq!(
            runtime
                .show_job_run(&source_run.run_id)
                .expect("show source after replay"),
            before
        );

        let event: serde_json::Value = serde_json::from_str(
            &v2_events(&runtime, &replay_result.run_id, "run.started")
                .first()
                .expect("run_started audit event")
                .payload_json,
        )
        .expect("parse run_started");
        assert_eq!(
            event
                .get("retry_source_run_id")
                .and_then(serde_json::Value::as_str),
            Some(source_result.run_id.as_str())
        );
    }

    #[cfg(unix)]
    #[test]
    fn v2_cli_agent_loop_persists_invocation_metrics() {
        let (_root, runtime, repo_root, _global_root) = test_runtime();
        let fake_bin = repo_root.join("codex");
        write_fake_codex(&fake_bin);

        let now = Utc::now();
        runtime
            .upsert_executor_def(&ExecutorDef {
                name: "codex".to_string(),
                executor_type: ExecutorType::DirectAgent,
                command: Some(fake_bin.display().to_string()),
                args: Vec::new(),
                stdout_format: None,
                model_pair_override: None,
                model_flag: None,
                timeout_seconds: None,
                env: HashMap::new(),
                sandbox: None,
                allow_fallback: false,
                created_at: now,
                updated_at: now,
            })
            .expect("seed fake codex executor");

        let yaml_path = repo_root.join("qa_cli_metrics.yaml");
        write_cli_metrics_job(
            &yaml_path,
            "qa_cli_metrics",
            "codex_metrics",
            "codex",
            "gpt-test",
        );
        let task = runtime
            .add_task(TaskAddParams {
                title: "Metrics fixture".to_string(),
                description: "Task fixture for CLI invocation metrics.".to_string(),
                ..Default::default()
            })
            .expect("seed task for CLI envelope");

        let result = runtime
            .run_job_v2_from_yaml(
                &yaml_path,
                json!({"prompt": "collect metrics", "task_id": task.id.clone()}),
                None,
            )
            .expect("cli metrics job succeeds");

        let records = runtime
            .invocation_records(InvocationQuery {
                job_run_id: Some(result.run_id.clone()),
                limit: 10,
                ..InvocationQuery::default()
            })
            .expect("query invocation records");
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.activity_id, "codex_metrics");
        assert_eq!(record.agent, "codex");
        assert_eq!(record.model.as_deref(), Some("gpt-test"));
        assert_eq!(record.input_tokens, 100);
        assert_eq!(record.cache_read_tokens, 25);
        assert_eq!(record.output_tokens, 12);
        assert_eq!(record.task_ids, vec![task.id]);
        assert_eq!(record.tool_call_count, 1);
        assert_eq!(record.tool_calls[0].tool_name, "command_execution");

        let activity = runtime
            .activity_invocation_metrics()
            .expect("activity metrics");
        assert!(activity.iter().any(|row| {
            row.activity_id == "codex_metrics"
                && row.agent == "codex"
                && row.model.as_deref() == Some("gpt-test")
                && row.total_input_tokens == 100
                && row.total_output_tokens == 12
                && row.total_tool_calls == 1
        }));

        let tools = runtime.tool_invocation_metrics().expect("tool metrics");
        assert!(tools.iter().any(|row| {
            row.activity_id == "codex_metrics"
                && row.tool_name == "command_execution"
                && row.call_count == 1
        }));
    }

    #[cfg(unix)]
    #[test]
    fn v2_claude_fable_alias_persists_provider_reported_model_and_cost() {
        let (_root, runtime, repo_root, _global_root) = test_runtime();
        let fake_bin = repo_root.join("claude");
        write_fake_cli_response(
            &fake_bin,
            r#"{"type":"result","subtype":"success","result":"{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null}","total_cost_usd":0.286169,"modelUsage":{"claude-haiku-4-5-20251001":{"costUSD":0.000598,"canonicalModel":"claude-haiku-4-5"},"claude-fable-5":{"costUSD":0.285571,"canonicalModel":"claude-fable-5"}},"usage":{"input_tokens":2,"output_tokens":102}}"#,
        );

        let now = Utc::now();
        runtime
            .upsert_executor_def(&ExecutorDef {
                name: "claude".to_string(),
                executor_type: ExecutorType::DirectAgent,
                command: Some(fake_bin.display().to_string()),
                args: Vec::new(),
                stdout_format: None,
                model_pair_override: None,
                model_flag: None,
                timeout_seconds: None,
                env: HashMap::new(),
                sandbox: None,
                allow_fallback: false,
                created_at: now,
                updated_at: now,
            })
            .expect("seed fake Claude executor");

        let yaml_path = repo_root.join("qa_fable_metrics.yaml");
        write_cli_metrics_job(
            &yaml_path,
            "qa_fable_metrics",
            "fable_metrics",
            "claude",
            "fable",
        );
        let result = runtime
            .run_job_v2_from_yaml(&yaml_path, json!({"prompt": "collect metrics"}), None)
            .expect("Claude fable metrics job succeeds");

        let records = runtime
            .invocation_records(InvocationQuery {
                job_run_id: Some(result.run_id),
                limit: 1,
                ..InvocationQuery::default()
            })
            .expect("query invocation records");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].activity_id, "fable_metrics");
        assert_eq!(records[0].agent, "claude");
        assert_eq!(records[0].model.as_deref(), Some("claude-fable-5"));
        assert_eq!(records[0].provider_cost_usd, Some(0.286169));
    }

    #[cfg(unix)]
    #[test]
    fn task_triage_pipeline_applies_multiple_cli_envelope_dispositions() {
        let (_root, runtime, repo_root, global_root) = test_runtime();
        seed_default_catalogs(&global_root);
        let environmental = seed_failed_triage_candidate(&runtime, "Environmental triage fixture");
        let code_defect = seed_failed_triage_candidate(&runtime, "Code-defect triage fixture");

        let agent_envelope = json!({
            "schemaVersion": 1,
            "status": "success",
            "result": {
                "dispositions": [
                    {
                        "task_id": environmental.clone(),
                        "classification": "environmental",
                        "disposition": "rebacklog",
                        "diagnosis": "the runner lost its workspace lease",
                        "mitigation": "stale lease cleared"
                    },
                    {
                        "task_id": code_defect.clone(),
                        "classification": "code_defect",
                        "disposition": "stay_blocked",
                        "diagnosis": "the task still has failing tests"
                    }
                ],
                "summary": "one task can retry and one needs a code fix"
            },
            "error": null
        });
        let provider_stdout = json!({
            "type": "result",
            "subtype": "success",
            "result": format!("Triage complete.\n{agent_envelope}"),
            "usage": {
                "input_tokens": 12,
                "output_tokens": 8
            }
        })
        .to_string();
        let fake_bin = repo_root.join("claude");
        write_fake_cli_response(&fake_bin, &provider_stdout);

        let now = Utc::now();
        runtime
            .upsert_executor_def(&ExecutorDef {
                name: "claude".to_string(),
                executor_type: ExecutorType::DirectAgent,
                command: Some(fake_bin.display().to_string()),
                args: Vec::new(),
                stdout_format: None,
                model_pair_override: None,
                model_flag: None,
                timeout_seconds: None,
                env: HashMap::new(),
                sandbox: None,
                allow_fallback: false,
                created_at: now,
                updated_at: now,
            })
            .expect("seed fake claude executor");

        let result = runtime
            .run_job_v2_from_yaml(
                &global_root.join("resources/jobs/task_triage_pipeline.yaml"),
                json!({
                    "task_ids": [environmental.clone(), code_defect.clone()],
                    "max_tasks": 20,
                    "max_rebacklogs": 2,
                }),
                Some(Backend::Cli),
            )
            .expect("task triage pipeline succeeds");

        assert!(result.success);
        assert_eq!(
            result.pipeline["triage"]["dispositions"]
                .as_array()
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(
            result.pipeline["apply_dispositions"]["rebacklogged_count"],
            json!(1)
        );
        assert_eq!(
            result.pipeline["apply_dispositions"]["diagnosed_count"],
            json!(1)
        );
        assert_eq!(
            runtime
                .get_task(&environmental)
                .expect("environmental task")
                .status,
            TaskStatus::Backlog
        );
        assert_eq!(
            runtime
                .get_task(&code_defect)
                .expect("code defect task")
                .status,
            TaskStatus::Blocked
        );
    }

    fn write_three_step_job(path: &Path, name: &str) {
        let yaml = format!(
            r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  steps:
    - id: nap0
      spec:
        type: deterministic
        action: sleep
        config: {{}}
    - id: nap1
      spec:
        type: deterministic
        action: sleep
        config: {{}}
    - id: nap2
      spec:
        type: deterministic
        action: sleep
        config: {{}}
"#
        );
        std::fs::write(path, yaml).expect("write three-step job yaml");
    }

    /// [ORB-10002] Host checkpoint persistence: `checkpoint_step` records the
    /// step into the run's `PipelineState` (state, output, snapshot, cursor).
    #[test]
    fn checkpoint_step_records_into_run_state() {
        let (_root, runtime, _repo_root, _global_root) = test_runtime();
        let run = runtime
            .stores()
            .jobs()
            .insert_run("qa_ckpt", 1, Utc::now(), Some(json!({"seconds": 0})), None)
            .expect("insert run");
        let initial = orbit_common::types::PipelineState::new(
            run.run_id.clone(),
            run.job_id.clone(),
            json!({"seconds": 0}),
        );
        runtime
            .stores()
            .jobs()
            .write_run_state(&run.run_id, &initial)
            .expect("write initial state");

        <OrbitRuntime as V2RuntimeHost>::checkpoint_step(
            &runtime,
            &run.run_id,
            0,
            "nap0",
            &json!({"ok": 0}),
            &json!({"nap0": {"ok": 0}}),
        )
        .expect("checkpoint step 0");
        <OrbitRuntime as V2RuntimeHost>::checkpoint_step(
            &runtime,
            &run.run_id,
            1,
            "nap1",
            &json!({"ok": 1}),
            &json!({"nap0": {"ok": 0}, "nap1": {"ok": 1}}),
        )
        .expect("checkpoint step 1");

        let state = runtime
            .read_run_state(&run.run_id)
            .expect("read state")
            .expect("state exists");
        assert_eq!(
            state.step_states.get(&0),
            Some(&orbit_common::types::JobRunState::Success)
        );
        assert_eq!(
            state.step_states.get(&1),
            Some(&orbit_common::types::JobRunState::Success)
        );
        assert_eq!(state.step_outputs.get(&0), Some(&json!({"ok": 0})));
        assert_eq!(state.next_step_index, 2);
        assert_eq!(
            state.pipeline,
            json!({"nap0": {"ok": 0}, "nap1": {"ok": 1}})
        );
    }

    /// [ORB-10002] A checkpoint against a run that was never persisted is a
    /// silent no-op (direct `execute_job` callers without a run row).
    #[test]
    fn checkpoint_step_without_run_row_is_noop() {
        let (_root, runtime, _repo_root, _global_root) = test_runtime();
        <OrbitRuntime as V2RuntimeHost>::checkpoint_step(
            &runtime,
            "jrun-never-persisted",
            0,
            "nap0",
            &json!({}),
            &json!({}),
        )
        .expect("no-op checkpoint");
    }

    /// [ORB-10002] Acceptance-shaped test: a run is "SIGKILLed" between steps
    /// (a real child worker process is killed, the run row stays `running`,
    /// step 0's checkpoint is already persisted), then a fresh scan marks it
    /// `interrupted` and `resume_job_run` completes only the remaining steps.
    ///
    /// The interruption itself is state-simulated (checkpoint written through
    /// the real host path, run left running against a real dead pid) rather
    /// than SIGKILLing a live orbit process mid-run; the orphan-liveness and
    /// resume paths exercised are the production ones.
    #[cfg(unix)]
    #[test]
    fn interrupted_run_resumes_skipping_checkpointed_steps() {
        let (_root, runtime, _repo_root, global_root) = test_runtime();
        let jobs_dir = global_root.join("resources/jobs");
        std::fs::create_dir_all(&jobs_dir).expect("create jobs dir");
        write_three_step_job(&jobs_dir.join("qa_resume_ckpt.yaml"), "qa_resume_ckpt");

        // Simulate the interrupted first attempt: a real worker process owns
        // the run, step 0's checkpoint lands, then the worker dies hard.
        let input = json!({"seconds": 0});
        let run = runtime
            .stores()
            .jobs()
            .insert_run("qa_resume_ckpt", 1, Utc::now(), Some(input.clone()), None)
            .expect("insert run");
        let initial = orbit_common::types::PipelineState::new(
            run.run_id.clone(),
            run.job_id.clone(),
            input.clone(),
        );
        runtime
            .stores()
            .jobs()
            .write_run_state(&run.run_id, &initial)
            .expect("write initial state");
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn fake worker");
        runtime
            .stores()
            .jobs()
            .mark_run_running(&run.run_id, Utc::now(), child.id())
            .expect("mark running under fake worker pid");
        <OrbitRuntime as V2RuntimeHost>::checkpoint_step(
            &runtime,
            &run.run_id,
            0,
            "nap0",
            &json!({"checkpointed": true}),
            &json!({"nap0": {"checkpointed": true}}),
        )
        .expect("persist step 0 checkpoint");
        child.kill().expect("SIGKILL fake worker");
        child.wait().expect("reap fake worker");

        // Orphan scan (also runs at workspace open / every run query): the
        // dead owner flips the stuck `running` run to `interrupted`.
        let shown = runtime.show_job_run(&run.run_id).expect("show run");
        assert_eq!(shown.state, JobRunState::Interrupted);
        assert!(shown.steps.iter().any(|step| {
            step.state == JobRunState::Interrupted
                && step.error_message.as_deref().is_some_and(|message| {
                    message.contains("recorded worker process is no longer alive")
                })
        }));

        // Resume: a new linked run completes only the remaining steps.
        let result = runtime
            .resume_job_run(&run.run_id)
            .expect("resume interrupted run");
        assert!(result.success);
        assert_ne!(result.run_id, run.run_id);
        assert_eq!(
            result.pipeline.get("nap0"),
            Some(&json!({"checkpointed": true})),
            "checkpointed step 0 output must be fed into the resumed pipeline"
        );
        assert!(result.pipeline.get("nap1").is_some());
        assert!(result.pipeline.get("nap2").is_some());

        let resumed = runtime.show_job_run(&result.run_id).expect("show resumed");
        assert_eq!(resumed.state, JobRunState::Success);
        assert_eq!(resumed.attempt, 2);
        assert_eq!(
            resumed.retry_source_run_id.as_deref(),
            Some(run.run_id.as_str())
        );
        let resumed_state = runtime
            .read_run_state(&result.run_id)
            .expect("read resumed checkpoint")
            .expect("resumed checkpoint exists");
        assert_eq!(resumed_state.step_states.len(), 3);
        assert_eq!(resumed_state.step_outputs.len(), 3);
        assert_eq!(
            resumed_state.pipeline.get("nap0"),
            Some(&json!({"checkpointed": true}))
        );
        assert!(resumed_state.pipeline.get("nap1").is_some());
        assert!(resumed_state.pipeline.get("nap2").is_some());

        // Step 0 was NOT re-executed: it is audited as skipped-for-resume and
        // never started; steps 1 and 2 started for real.
        let skipped = v2_events(&runtime, &result.run_id, "step.skipped");
        assert!(skipped.iter().any(|row| {
            let payload: serde_json::Value =
                serde_json::from_str(&row.payload_json).expect("payload");
            payload["step_id"] == json!("nap0")
                && payload["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("resume"))
        }));
        let started_ids: Vec<String> = v2_events(&runtime, &result.run_id, "step.started")
            .iter()
            .map(|row| {
                let payload: serde_json::Value =
                    serde_json::from_str(&row.payload_json).expect("payload");
                payload["step_id"].as_str().unwrap_or_default().to_string()
            })
            .collect();
        assert!(!started_ids.contains(&"nap0".to_string()));
        assert!(started_ids.contains(&"nap1".to_string()));
        assert!(started_ids.contains(&"nap2".to_string()));

        // Source run stays interrupted; resume never mutates its history.
        let source_after = runtime.show_job_run(&run.run_id).expect("show source");
        assert_eq!(source_after.state, JobRunState::Interrupted);
        let source_state_after = runtime
            .read_run_state(&run.run_id)
            .expect("read source checkpoint")
            .expect("source checkpoint exists");
        assert_eq!(source_state_after.step_states.len(), 1);
        assert!(source_state_after.pipeline.get("nap1").is_none());
        assert!(source_state_after.pipeline.get("nap2").is_none());
    }

    /// [ORB-10002] Resume refuses runs that are not interrupted / failed /
    /// timed-out.
    #[test]
    fn resume_rejects_successful_runs() {
        let (_root, runtime, repo_root, _global_root) = test_runtime();
        let yaml_path = repo_root.join("qa_resume_guard.yaml");
        write_job(&yaml_path, "qa_resume_guard", "sleep");
        let result = runtime
            .run_job_v2_from_yaml(&yaml_path, json!({"seconds": 0}), None)
            .expect("run succeeds");

        let error = runtime
            .resume_job_run(&result.run_id)
            .expect_err("resume of a successful run must fail");
        assert!(
            error
                .to_string()
                .contains("resume requires an interrupted, failed, or timed-out run"),
            "{error}"
        );
    }

    /// [ORB-10385] An unregistered action is now caught by validation before
    /// the first step dispatches rather than by the dispatcher, so the
    /// diagnostic names the activity and the runtime skew. The durable
    /// bookkeeping this test guards — `Failed` run state, a persisted step
    /// error, and an `error` run.finished audit event — is unchanged.
    #[test]
    fn failing_direct_run_records_failure_state() {
        let (_root, runtime, repo_root, _global_root) = test_runtime();
        let yaml_path = repo_root.join("qa_failing.yaml");
        write_job(&yaml_path, "qa_failing", "missing_action");

        let err = runtime
            .run_job_v2_from_yaml(&yaml_path, json!({}), None)
            .expect_err("direct job run should fail");
        assert!(
            err.to_string().contains("missing_action")
                && err.to_string().contains("not registered"),
            "{err}"
        );

        let history = runtime.job_history("qa_failing").expect("failure history");
        let run = history.first().expect("failed run");
        assert_eq!(run.state, JobRunState::Failed);
        assert!(run.steps.iter().any(|step| {
            step.error_message
                .as_deref()
                .is_some_and(|message| message.contains("not registered"))
        }));
        let event: serde_json::Value = serde_json::from_str(
            &v2_events(&runtime, &run.run_id, "run.finished")
                .first()
                .expect("run_finished audit event")
                .payload_json,
        )
        .expect("parse run_finished");
        assert_eq!(
            event.get("outcome").and_then(serde_json::Value::as_str),
            Some("error")
        );
        assert!(
            event
                .get("error_message")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|message| message.contains("not registered"))
        );
        assert!(
            runtime
                .read_run_state(&run.run_id)
                .expect("read run state")
                .is_some()
        );
    }
}
