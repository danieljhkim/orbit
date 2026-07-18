use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;
use orbit_common::types::{
    AuditEventStatus, JobRun, JobRunState, JobScheduleState, JobTargetType, NotFoundKind,
    OrbitError, OrbitEvent, PipelineState, audit_execution_id,
};
use orbit_store::{AuditEventInsertParams, JobRunStepParams, TaskReservationReleaseReason};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::OrbitRuntime;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const PIPELINE_WAIT_DEFAULT_TIMEOUT_SECONDS: u64 = 3600;
const PIPELINE_WAIT_MAX_TIMEOUT_SECONDS: u64 = 7200;
const PIPELINE_WAIT_DEFAULT_POLL_SECONDS: u64 = 5;
const PIPELINE_WAIT_MIN_POLL_SECONDS: u64 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct PipelineInvokeResult {
    pub run_id: String,
    pub job_name: String,
    pub submitted_at: String,
    pub queued: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineWaitResult {
    pub results: Vec<PipelineWaitEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineWaitEntry {
    pub run_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl OrbitRuntime {
    /// Submit a `ship` workflow run (the `task_auto_pipeline` job).
    ///
    /// Shared entry point for every non-interactive submission surface
    /// (dashboard HTTP endpoint, `orbit run ship-sweep`). `base_branch`
    /// falls back to the workspace's `[workflow] base_branch`; an empty
    /// `task_ids` slice selects auto (backlog-discovery) mode. One-shot:
    /// returns as soon as the run is persisted and its worker spawned.
    /// `review_crew` is required and applied only when `review` is enabled.
    pub fn submit_ship_run(
        &self,
        mode: crate::command::workflow::ShipMode,
        base_branch: Option<&str>,
        task_ids: &[String],
        review: bool,
        review_crew: Option<&str>,
        actor: Option<&str>,
    ) -> Result<PipelineInvokeResult, OrbitError> {
        let workflow =
            crate::command::workflow::find_workflow(crate::command::workflow::SHIP_WORKFLOW_ALIAS)
                .ok_or_else(|| OrbitError::InvalidInput("unknown workflow 'ship'".to_string()))?;
        let base = base_branch.unwrap_or_else(|| self.workflow_base_branch());
        let input =
            crate::command::workflow::build_ship_input(mode, base, task_ids, review, review_crew)?;
        self.submit_pipeline_run(workflow.job_id, input, None, actor)
    }

    pub fn submit_pipeline_run(
        &self,
        job_name: &str,
        input: Value,
        priority: Option<&str>,
        actor: Option<&str>,
    ) -> Result<PipelineInvokeResult, OrbitError> {
        let result = (|| {
            let (_, spec) = self.load_v2_job_asset_by_name(job_name)?;
            if spec.state != JobScheduleState::Enabled {
                return Err(OrbitError::InvalidInput(format!(
                    "job '{job_name}' is disabled"
                )));
            }

            let submitted_at = Utc::now();
            let run = self.stores().jobs().insert_run(
                job_name,
                1,
                submitted_at,
                Some(input.clone()),
                None,
            )?;
            let initial_state =
                PipelineState::new(run.run_id.clone(), run.job_id.clone(), input.clone());
            self.stores()
                .jobs()
                .write_run_state(&run.run_id, &initial_state)?;

            self.reconcile_stale_job_runs(Some(job_name))?;
            let active_runs = self.stores().jobs().list_pending_or_running(job_name)?;
            let queued = !pipeline_run_is_runnable(&active_runs, &run.run_id, spec.max_active_runs);

            if let Err(error) = self.spawn_pipeline_worker(&run.run_id, actor) {
                let message = format!(
                    "pipeline worker for run '{}' could not start from registered workspace '{}': {error}",
                    run.run_id,
                    self.paths().repo_root.display(),
                );
                let _ = self.finalize_pipeline_worker_startup_failure(&run, &message, actor);
                return Err(error);
            }

            Ok(PipelineInvokeResult {
                run_id: run.run_id,
                job_name: job_name.to_string(),
                submitted_at: submitted_at.to_rfc3339(),
                queued,
            })
        })();

        self.record_pipeline_audit(
            "pipeline.invoke",
            result.as_ref().ok().map(|value| value.run_id.as_str()),
            actor,
            match &result {
                Ok(_) => AuditEventStatus::Success,
                Err(_) => AuditEventStatus::Failure,
            },
            json!({
                "actor": actor,
                "job_name": job_name,
                "priority": priority,
                "run_id": result.as_ref().ok().map(|value| value.run_id.clone()),
                "input_hash": input_hash(&input),
            }),
            result.as_ref().err().map(|error| error.to_string()),
        )?;

        result
    }

    pub fn wait_pipeline_runs(
        &self,
        run_ids: &[String],
        timeout_seconds: u64,
        poll_interval_seconds: u64,
        actor: Option<&str>,
    ) -> Result<PipelineWaitResult, OrbitError> {
        let started_payload = json!({
            "actor": actor,
            "run_ids": run_ids,
            "timeout_seconds": timeout_seconds,
        });
        self.record_pipeline_audit(
            "pipeline.wait.started",
            None,
            actor,
            AuditEventStatus::Success,
            started_payload,
            None,
        )?;

        let started_at = Instant::now();
        let timeout = Duration::from_secs(timeout_seconds);
        let poll = Duration::from_secs(poll_interval_seconds.max(PIPELINE_WAIT_MIN_POLL_SECONDS));

        loop {
            let snapshot = self.collect_pipeline_wait_entries(run_ids, false)?;
            if snapshot.iter().all(|entry| {
                matches!(
                    entry.status.as_str(),
                    "succeeded" | "failed" | "cancelled" | "interrupted"
                )
            }) {
                let result = PipelineWaitResult { results: snapshot };
                self.record_pipeline_wait_finished(actor, &result)?;
                return Ok(result);
            }

            if started_at.elapsed() >= timeout {
                let result = PipelineWaitResult {
                    results: self.collect_pipeline_wait_entries(run_ids, true)?,
                };
                self.record_pipeline_wait_finished(actor, &result)?;
                return Ok(result);
            }

            thread::sleep(poll);
        }
    }

    pub fn execute_pipeline_run_worker(&self, run_id: &str) -> Result<(), OrbitError> {
        // [ORB-10070] Claim the queued run for this worker process so orphan
        // reconciliation can tell a pending run whose worker is alive and
        // polling for its admission slot apart from one whose worker died
        // (crash, SIGKILL, host reboot). Best-effort: the run may already be
        // running/terminal, and a claim failure must never block execution.
        if let Err(error) = self
            .stores()
            .jobs()
            .claim_pending_run_owner(run_id, std::process::id())
        {
            tracing::warn!(
                target: "orbit.core.job_run",
                run_id,
                error = %error,
                "pipeline worker could not claim its pending run; orphan \
                 detection falls back to the unclaimed-run grace window",
            );
        }
        loop {
            let run = self.show_job_run(run_id)?;
            match run.state {
                JobRunState::Pending => {}
                JobRunState::Running
                | JobRunState::Success
                | JobRunState::Failed
                | JobRunState::Timeout
                | JobRunState::Cancelled
                | JobRunState::Interrupted => return Ok(()),
                other => {
                    return Err(OrbitError::Execution(format!(
                        "pipeline worker cannot execute run '{}' from state '{}'",
                        run_id, other
                    )));
                }
            }

            let (yaml_path, spec) = self.load_v2_job_asset_by_name(&run.job_id)?;
            if spec.state != JobScheduleState::Enabled {
                let _ = self.cancel_job_run(&run.run_id);
                return Err(OrbitError::InvalidInput(format!(
                    "job '{}' is disabled",
                    run.job_id
                )));
            }

            self.reconcile_stale_job_runs(Some(&run.job_id))?;
            let active_runs = self.stores().jobs().list_pending_or_running(&run.job_id)?;
            if !pipeline_run_is_runnable(&active_runs, &run.run_id, spec.max_active_runs) {
                thread::sleep(Duration::from_secs(PIPELINE_WAIT_MIN_POLL_SECONDS));
                continue;
            }

            return self.execute_pipeline_run_now(&run, &yaml_path);
        }
    }

    pub fn normalize_pipeline_wait_timeout(raw: Option<u64>) -> Result<u64, OrbitError> {
        let timeout_seconds = raw.unwrap_or(PIPELINE_WAIT_DEFAULT_TIMEOUT_SECONDS);
        if timeout_seconds > PIPELINE_WAIT_MAX_TIMEOUT_SECONDS {
            return Err(OrbitError::InvalidInput(format!(
                "`timeout_seconds` must be <= {PIPELINE_WAIT_MAX_TIMEOUT_SECONDS}"
            )));
        }
        Ok(timeout_seconds)
    }

    pub fn normalize_pipeline_wait_poll_interval(raw: Option<u64>) -> u64 {
        raw.unwrap_or(PIPELINE_WAIT_DEFAULT_POLL_SECONDS)
            .max(PIPELINE_WAIT_MIN_POLL_SECONDS)
    }

    fn execute_pipeline_run_now(&self, run: &JobRun, yaml_path: &Path) -> Result<(), OrbitError> {
        let started_at = Utc::now();
        let changed =
            self.stores()
                .jobs()
                .mark_run_running(&run.run_id, started_at, std::process::id())?;
        if !changed {
            return Ok(());
        }
        let input = run
            .input
            .clone()
            .unwrap_or_else(|| Value::Object(Default::default()));
        self.record_run_crew_from_input(&run.run_id, &input)?;

        self.record_event(OrbitEvent::JobRunStarted {
            job_id: run.job_id.clone(),
            run_id: run.run_id.clone(),
            attempt: run.attempt,
        })?;

        let outcome = self.run_job_v2_from_yaml_with_run_id(
            yaml_path,
            input.clone(),
            None,
            Some(run.run_id.clone()),
        );
        let finished_at = Utc::now();
        let duration_ms = Some(
            finished_at
                .signed_duration_since(started_at)
                .num_milliseconds()
                .max(0) as u64,
        );

        match outcome {
            Ok(result) => {
                let mut state = self.read_run_state(&run.run_id)?.unwrap_or_else(|| {
                    PipelineState::new(run.run_id.clone(), run.job_id.clone(), input)
                });
                state.sync_pipeline(result.pipeline.clone());
                self.stores().jobs().write_run_state(&run.run_id, &state)?;

                let final_state = if result.success {
                    JobRunState::Success
                } else {
                    let fallback = "job completed with success=false but emitted no failure detail";
                    let message = result.message.as_deref().unwrap_or(fallback);
                    let _ =
                        self.record_pipeline_failure_step(run, started_at, finished_at, message);
                    JobRunState::Failed
                };
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
                Ok(())
            }
            Err(error) => {
                let _ = self.record_pipeline_failure_step(
                    run,
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

    pub(crate) fn record_pipeline_failure_step(
        &self,
        run: &JobRun,
        started_at: chrono::DateTime<Utc>,
        finished_at: chrono::DateTime<Utc>,
        message: &str,
    ) -> Result<(), OrbitError> {
        self.record_pipeline_diagnostic_step(
            run,
            started_at,
            finished_at,
            message,
            JobRunState::Failed,
        )
    }

    /// [ORB-10002] Record a terminal diagnostic step with an explicit state
    /// (`failed` for job errors, `interrupted` for orphan reconciliation).
    pub(crate) fn record_pipeline_diagnostic_step(
        &self,
        run: &JobRun,
        started_at: chrono::DateTime<Utc>,
        finished_at: chrono::DateTime<Utc>,
        message: &str,
        state: JobRunState,
    ) -> Result<(), OrbitError> {
        let current = self.show_job_run(&run.run_id)?;
        let already_has_error = current
            .steps
            .iter()
            .any(|step| step.error_code.is_some() || step.error_message.is_some());
        if already_has_error {
            return Ok(());
        }

        let step_index = current
            .steps
            .iter()
            .map(|step| step.step_index)
            .max()
            .map(|index| index.saturating_add(1) as usize)
            .unwrap_or(0);
        let duration_ms = Some(
            finished_at
                .signed_duration_since(started_at)
                .num_milliseconds()
                .max(0) as u64,
        );
        let params = JobRunStepParams {
            step_index,
            target_type: JobTargetType::Job,
            target_id: run.job_id.clone(),
            started_at,
            finished_at,
            duration_ms,
            exit_code: None,
            agent_response_json: None,
            state,
            error_code: None,
            error_message: Some(message.to_string()),
        };
        let _ = self
            .stores()
            .jobs()
            .complete_run_step(&run.run_id, &params)?;
        Ok(())
    }

    fn collect_pipeline_wait_entries(
        &self,
        run_ids: &[String],
        timeout_incomplete: bool,
    ) -> Result<Vec<PipelineWaitEntry>, OrbitError> {
        run_ids
            .iter()
            .map(|run_id| {
                let run = match self.show_job_run(run_id) {
                    Ok(run) => run,
                    Err(OrbitError::NotFound {
                        kind: NotFoundKind::JobRun,
                        ..
                    }) => {
                        return Ok(PipelineWaitEntry {
                            run_id: run_id.clone(),
                            status: "failed".to_string(),
                            finished_at: None,
                            pipeline: None,
                            error: Some("unknown run".to_string()),
                        });
                    }
                    Err(error) => return Err(error),
                };

                let terminal = match run.state {
                    JobRunState::Success => Some("succeeded"),
                    JobRunState::Failed => Some("failed"),
                    JobRunState::Cancelled => Some("cancelled"),
                    JobRunState::Interrupted => Some("interrupted"),
                    _ => None,
                };
                let status = match (terminal, timeout_incomplete) {
                    (Some(status), _) => status.to_string(),
                    (None, true) => "timeout".to_string(),
                    (None, false) => run.state.to_string(),
                };
                let pipeline = if matches!(status.as_str(), "timeout") {
                    None
                } else {
                    self.read_run_state(run_id)?.map(|state| state.pipeline)
                };
                Ok(PipelineWaitEntry {
                    run_id: run_id.clone(),
                    status,
                    finished_at: run.finished_at.map(|value| value.to_rfc3339()),
                    pipeline,
                    error: None,
                })
            })
            .collect()
    }

    fn spawn_pipeline_worker(&self, run_id: &str, actor: Option<&str>) -> Result<(), OrbitError> {
        let current_exe = std::env::current_exe().map_err(|error| {
            OrbitError::Execution(format!("resolve current orbit executable: {error}"))
        })?;
        let mut command = Command::new(resolve_pipeline_worker_executable(current_exe));
        configure_pipeline_worker_command(&mut command, &self.paths().repo_root, run_id);
        #[cfg(unix)]
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        // L-0090: discover detached workers by registered cwd, never explicit --root.
        // Start the observer before the process so every successfully spawned
        // worker has a parent-side path that can terminalize a pre-claim exit.
        // Passing `--root` here used to pin both the workspace and global roots
        // to `.orbit/`, which disconnected the worker from the global registry
        // database that contains the persisted run. Cwd discovery preserves
        // the registered workspace context for top-level and nested workers.
        let (sender, receiver) = mpsc::sync_channel::<Child>(1);
        let runtime = self.clone();
        let run_id_for_observer = run_id.to_string();
        let actor_for_observer = actor.map(ToOwned::to_owned);
        let workspace_for_observer = self.paths().repo_root.clone();
        thread::Builder::new()
            .name(format!("pipeline-start-{run_id}"))
            .spawn(move || {
                let Ok(child) = receiver.recv() else {
                    return;
                };
                if let Err(error) = runtime.monitor_pipeline_worker_startup(
                    &run_id_for_observer,
                    child,
                    &workspace_for_observer,
                    actor_for_observer.as_deref(),
                ) {
                    tracing::error!(
                        target: "orbit.core.job_run",
                        run_id = run_id_for_observer,
                        error = %error,
                        "failed to observe pipeline worker startup",
                    );
                }
            })
            .map_err(|error| {
                OrbitError::Execution(format!("spawn pipeline worker observer: {error}"))
            })?;

        let child = command
            .spawn()
            .map_err(|error| OrbitError::Execution(format!("spawn pipeline worker: {error}")))?;
        sender.send(child).map_err(|error| {
            OrbitError::Execution(format!("hand pipeline worker to startup observer: {error}"))
        })
    }

    pub(crate) fn monitor_pipeline_worker_startup(
        &self,
        run_id: &str,
        mut child: Child,
        workspace: &Path,
        actor: Option<&str>,
    ) -> Result<(), OrbitError> {
        let child_pid = child.id();
        loop {
            let run = self.show_job_run(run_id)?;
            if let Some(owner_pid) = run.pid {
                let _ = self.record_pipeline_audit(
                    "pipeline.worker.claimed",
                    Some(run_id),
                    actor,
                    AuditEventStatus::Success,
                    json!({
                        "run_id": run_id,
                        "worker_pid": child_pid,
                        "owner_pid": owner_pid,
                        "workspace": workspace,
                        "state": run.state.to_string(),
                    }),
                    None,
                );
                return Ok(());
            }
            if run.state != JobRunState::Pending {
                return Ok(());
            }

            if let Some(status) = child.try_wait().map_err(|error| {
                OrbitError::Execution(format!(
                    "observe pipeline worker process for run '{run_id}': {error}"
                ))
            })? {
                let message = format!(
                    "pipeline worker for run '{run_id}' exited with status {status} before claiming the persisted run from registered workspace '{}'; verify workspace registration and worker root discovery",
                    workspace.display(),
                );
                self.finalize_pipeline_worker_startup_failure(&run, &message, actor)?;
                return Ok(());
            }

            thread::sleep(Duration::from_millis(25));
        }
    }

    fn finalize_pipeline_worker_startup_failure(
        &self,
        run: &JobRun,
        message: &str,
        actor: Option<&str>,
    ) -> Result<(), OrbitError> {
        let current = self.show_job_run(&run.run_id)?;
        if current.state != JobRunState::Pending || current.pid.is_some() {
            return Ok(());
        }

        let finished_at = Utc::now();
        let changed = self.finalize_job_run_with_reservation_cleanup(
            &run.run_id,
            JobRunState::Interrupted,
            finished_at,
            None,
            TaskReservationReleaseReason::RunTerminal,
        )?;
        if changed {
            self.record_pipeline_diagnostic_step(
                run,
                run.scheduled_at,
                finished_at,
                message,
                JobRunState::Interrupted,
            )?;
            self.record_event(OrbitEvent::JobRunCompleted {
                job_id: run.job_id.clone(),
                run_id: run.run_id.clone(),
                state: JobRunState::Interrupted.to_string(),
            })?;
        }
        self.record_pipeline_audit(
            "pipeline.worker.startup",
            Some(&run.run_id),
            actor,
            AuditEventStatus::Failure,
            json!({
                "run_id": run.run_id,
                "workspace": self.paths().repo_root,
            }),
            Some(message.to_string()),
        )
    }

    fn record_pipeline_wait_finished(
        &self,
        actor: Option<&str>,
        result: &PipelineWaitResult,
    ) -> Result<(), OrbitError> {
        let mut succeeded = 0usize;
        let mut failed = 0usize;
        let mut cancelled = 0usize;
        let mut timeout = 0usize;
        for entry in &result.results {
            match entry.status.as_str() {
                "succeeded" => succeeded += 1,
                "failed" => failed += 1,
                "cancelled" => cancelled += 1,
                "timeout" => timeout += 1,
                _ => {}
            }
        }

        self.record_pipeline_audit(
            "pipeline.wait.finished",
            None,
            actor,
            AuditEventStatus::Success,
            json!({
                "actor": actor,
                "results_summary": {
                    "succeeded": succeeded,
                    "failed": failed,
                    "cancelled": cancelled,
                    "timeout": timeout,
                },
            }),
            None,
        )
    }

    fn record_pipeline_audit(
        &self,
        tool_name: &str,
        target_id: Option<&str>,
        actor: Option<&str>,
        status: AuditEventStatus,
        arguments: Value,
        error_message: Option<String>,
    ) -> Result<(), OrbitError> {
        let arguments_json = serde_json::to_string(&arguments).map_err(|error| {
            OrbitError::Store(format!("serialize pipeline audit args: {error}"))
        })?;
        let execution_id = audit_execution_id("exec");
        self.record_audit_event(&AuditEventInsertParams {
            execution_id,
            command: "tool".to_string(),
            subcommand: Some("run".to_string()),
            tool_name: Some(tool_name.to_string()),
            target_type: Some("job_run".to_string()),
            target_id: target_id.map(ToOwned::to_owned),
            role: "admin".to_string(),
            status,
            exit_code: if status == AuditEventStatus::Success {
                0
            } else {
                1
            },
            duration_ms: 0,
            working_directory: self.paths().repo_root.display().to_string(),
            arguments_json: Some(arguments_json),
            stdout_truncated: None,
            stderr_truncated: None,
            error_message,
            host: actor.map(ToOwned::to_owned),
            pid: std::process::id(),
            session_id: None,
            task_id: None,
            job_run_id: target_id.map(ToOwned::to_owned),
            activity_id: None,
            step_index: None,
        })
    }
}

/// Return a stable path suitable for launching a fresh worker process.
///
/// Linux exposes a process whose executable inode was unlinked as
/// `/installed/path (deleted)`. That pseudo-path cannot be executed, but after
/// an atomic upgrade the original installed path names the replacement binary.
/// Preserve ordinary paths, including real filenames ending in ` (deleted)`.
pub(crate) fn resolve_pipeline_worker_executable(current_exe: PathBuf) -> PathBuf {
    // L-0084: deleted Linux executable paths must resolve through the installed replacement.
    #[cfg(target_os = "linux")]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let current_path_is_missing = matches!(
            std::fs::metadata(&current_exe),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        );
        if current_path_is_missing
            && let Some(installed_path) = current_exe
                .as_os_str()
                .as_bytes()
                .strip_suffix(b" (deleted)")
        {
            return PathBuf::from(OsString::from_vec(installed_path.to_vec()));
        }
    }

    current_exe
}

pub(crate) fn configure_pipeline_worker_command(
    command: &mut Command,
    workspace: &Path,
    run_id: &str,
) {
    command
        .arg("job")
        .arg("run-pipeline-worker")
        .arg(run_id)
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
}

fn pipeline_run_is_runnable(runs: &[JobRun], run_id: &str, max_active_runs: u32) -> bool {
    let mut ordered = runs.to_vec();
    ordered.sort_by(|left, right| {
        left.scheduled_at
            .cmp(&right.scheduled_at)
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.run_id.cmp(&right.run_id))
    });
    ordered
        .iter()
        .take(max_active_runs.max(1) as usize)
        .any(|run| run.run_id == run_id)
}

fn input_hash(input: &Value) -> String {
    let encoded = serde_json::to_vec(input).unwrap_or_default();
    format!("{:x}", Sha256::digest(encoded))
}
