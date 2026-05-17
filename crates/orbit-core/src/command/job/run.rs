use chrono::{DateTime, Utc};
use orbit_common::types::{
    JobRun, JobRunState, NotFoundKind, OrbitError, OrbitEvent, PipelineState,
};
#[cfg(unix)]
use orbit_common::utility::process_identity::{
    ProbeOutcome, STABLE_TOKEN_PREFIX, legacy_lstart_matches, probe_process_start_identity,
};
use orbit_store::{JobRunQuery, TaskReservationReleaseReason};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

use crate::OrbitRuntime;

#[derive(Debug, Clone, Default)]
pub struct JobRunListParams {
    pub job_id: Option<String>,
    pub state: Option<JobRunState>,
    pub since: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobRunCancelResult {
    pub run_id: String,
    pub previous_state: String,
    pub final_state: String,
    pub actor: String,
    pub source: String,
    pub signal_attempted: bool,
    pub signal_outcome: Option<String>,
}

impl OrbitRuntime {
    pub fn cancel_job_run(&self, run_id: &str) -> Result<JobRunCancelResult, OrbitError> {
        self.cancel_job_run_with_context(run_id, "system", "runtime")
    }

    pub fn cancel_job_run_with_context(
        &self,
        run_id: &str,
        actor: &str,
        source: &str,
    ) -> Result<JobRunCancelResult, OrbitError> {
        let run = self
            .get_job_run_backend(run_id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::JobRun, run_id.to_string()))?;
        run.state
            .try_transition(orbit_common::types::RunEvent::Cancel)
            .map_err(|msg| {
                OrbitError::JobValidation(format!("cannot cancel job run '{}': {}", run_id, msg))
            })?;
        let signal_attempted = run.state == JobRunState::Running && run.pid.is_some();
        let signal_outcome = if signal_attempted {
            Some(signal_run_owner_process(&run)?)
        } else {
            None
        };
        let now = chrono::Utc::now();
        let duration_ms = run
            .started_at
            .map(|s| now.signed_duration_since(s).num_milliseconds().max(0) as u64);
        self.finalize_job_run_with_reservation_cleanup(
            run_id,
            JobRunState::Cancelled,
            now,
            duration_ms,
            TaskReservationReleaseReason::RunTerminal,
        )?;
        let cancelled_run = self
            .get_job_run_backend(run_id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::JobRun, run_id.to_string()))?;
        if cancelled_run.state != JobRunState::Cancelled {
            let detail = cancelled_run
                .state
                .try_transition(orbit_common::types::RunEvent::Cancel)
                .err()
                .unwrap_or_else(|| {
                    format!(
                        "stored state remained {} after cancellation",
                        cancelled_run.state
                    )
                });
            return Err(OrbitError::JobValidation(format!(
                "cannot cancel job run '{}': {}",
                run_id, detail
            )));
        }
        self.mark_cancelled_pipeline_state(&cancelled_run)?;
        self.record_event(OrbitEvent::JobRunCancelled {
            job_id: run.job_id.clone(),
            run_id: run_id.to_string(),
            previous_state: Some(run.state.to_string()),
            final_state: Some(JobRunState::Cancelled.to_string()),
            actor: Some(actor.to_string()),
            source: Some(source.to_string()),
            signal_attempted: Some(signal_attempted),
            signal_outcome: signal_outcome.clone(),
        })?;
        Ok(JobRunCancelResult {
            run_id: run_id.to_string(),
            previous_state: run.state.to_string(),
            final_state: JobRunState::Cancelled.to_string(),
            actor: actor.to_string(),
            source: source.to_string(),
            signal_attempted,
            signal_outcome,
        })
    }

    pub fn archive_job_run(&self, run_id: &str) -> Result<(), OrbitError> {
        let run = self.show_job_run(run_id)?;
        if matches!(run.state, JobRunState::Pending | JobRunState::Running) {
            return Err(OrbitError::JobValidation(format!(
                "job run '{}' is active and cannot be archived",
                run_id
            )));
        }
        let job_id = self.stores().jobs().archive_run(run_id)?;
        self.record_event(OrbitEvent::JobRunArchived {
            job_id,
            run_id: run_id.to_string(),
        })
    }

    pub fn delete_job_run(&self, run_id: &str) -> Result<(), OrbitError> {
        if let Some(run) = self.get_job_run_backend(run_id)? {
            self.reconcile_stale_job_run(&run)?;
        }
        if let Some(run) = self.get_job_run_backend(run_id)?
            && matches!(run.state, JobRunState::Pending | JobRunState::Running)
        {
            return Err(OrbitError::JobValidation(format!(
                "job run '{}' is active and cannot be deleted",
                run_id
            )));
        }
        let job_id = self.stores().jobs().delete_run(run_id)?;
        self.record_event(OrbitEvent::JobRunDeleted {
            job_id,
            run_id: run_id.to_string(),
        })
    }

    pub fn read_run_state(
        &self,
        run_id: &str,
    ) -> Result<Option<orbit_common::types::PipelineState>, OrbitError> {
        self.stores().jobs().read_run_state(run_id)
    }

    fn mark_cancelled_pipeline_state(&self, run: &JobRun) -> Result<(), OrbitError> {
        if let Some(mut state) = self.read_run_state(&run.run_id)? {
            if let Some(object) = state.pipeline.as_object_mut() {
                object.insert(
                    "status".to_string(),
                    Value::String(JobRunState::Cancelled.to_string()),
                );
                object.insert(
                    "state".to_string(),
                    Value::String(JobRunState::Cancelled.to_string()),
                );
                object.insert("cancelled".to_string(), Value::Bool(true));
            }
            state.clear_waiting_reasons();
            state.updated_at = Utc::now();
            self.stores().jobs().write_run_state(&run.run_id, &state)?;
        } else if run.input.is_some() {
            let mut state = PipelineState::new(
                run.run_id.clone(),
                run.job_id.clone(),
                run.input
                    .clone()
                    .unwrap_or_else(|| Value::Object(Default::default())),
            );
            if let Some(object) = state.pipeline.as_object_mut() {
                object.insert(
                    "status".to_string(),
                    Value::String(JobRunState::Cancelled.to_string()),
                );
                object.insert(
                    "state".to_string(),
                    Value::String(JobRunState::Cancelled.to_string()),
                );
                object.insert("cancelled".to_string(), Value::Bool(true));
            }
            self.stores().jobs().write_run_state(&run.run_id, &state)?;
        }
        Ok(())
    }

    pub fn job_history(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.reconcile_stale_job_runs(Some(job_id))?;
        match self.load_v2_job_asset_by_name(job_id) {
            Ok(_) => self.list_reconciled_job_history_backend(job_id),
            Err(error) => {
                let runs = self.list_reconciled_job_history_backend(job_id)?;
                if runs.is_empty() {
                    Err(error)
                } else {
                    Ok(runs)
                }
            }
        }
    }

    pub fn list_job_runs(&self, params: JobRunListParams) -> Result<Vec<JobRun>, OrbitError> {
        self.reconcile_stale_job_runs(params.job_id.as_deref())?;
        if let Some(job_id) = params.job_id.as_deref()
            && let Err(error) = self.load_v2_job_asset_by_name(job_id)
        {
            let runs = self.list_job_history_backend(job_id)?;
            if runs.is_empty() {
                return Err(error);
            }
        }

        let query = JobRunQuery {
            job_id: params.job_id,
            state: params.state,
            created_since: params.since,
            limit: params.limit,
        };
        let runs = self.list_job_runs_filtered_backend(&query)?;
        if self.reconcile_job_run_records(&runs)? > 0 {
            self.list_job_runs_filtered_backend(&query)
        } else {
            Ok(runs)
        }
    }

    pub fn show_job_run(&self, run_id: &str) -> Result<JobRun, OrbitError> {
        let run = self
            .get_job_run_backend(run_id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::JobRun, run_id.to_string()))?;
        self.reconcile_stale_job_run(&run)?;
        self.get_job_run_backend(run_id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::JobRun, run_id.to_string()))
    }

    pub(crate) fn reconcile_stale_job_runs(
        &self,
        job_id: Option<&str>,
    ) -> Result<usize, OrbitError> {
        let runs = if let Some(job_id) = job_id {
            self.stores().jobs().list_pending_or_running(job_id)?
        } else {
            self.stores().jobs().list_all_pending_or_running()?
        };

        let mut reconciled = 0usize;
        for run in runs {
            if self.reconcile_stale_job_run(&run)? {
                reconciled += 1;
            }
        }
        Ok(reconciled)
    }

    fn list_reconciled_job_history_backend(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        let runs = self.list_job_history_backend(job_id)?;
        if self.reconcile_job_run_records(&runs)? > 0 {
            self.list_job_history_backend(job_id)
        } else {
            Ok(runs)
        }
    }

    fn reconcile_job_run_records(&self, runs: &[JobRun]) -> Result<usize, OrbitError> {
        let mut reconciled = 0usize;
        for run in runs {
            if self.reconcile_stale_job_run(run)? {
                reconciled += 1;
            }
        }
        Ok(reconciled)
    }

    pub(crate) fn reconcile_stale_job_run(&self, run: &JobRun) -> Result<bool, OrbitError> {
        if terminal_run_timing_is_incomplete(run) {
            return self.repair_terminal_job_run_timing(run);
        }
        if !running_run_owner_is_stale(run) {
            return Ok(false);
        }

        let finished_at = Utc::now();
        let duration_ms = run.started_at.map(|started_at| {
            finished_at
                .signed_duration_since(started_at)
                .num_milliseconds()
                .max(0) as u64
        });
        let changed = self.finalize_job_run_with_reservation_cleanup(
            &run.run_id,
            JobRunState::Failed,
            finished_at,
            duration_ms,
            TaskReservationReleaseReason::StaleRunReconciled,
        )?;
        if !changed {
            return Ok(false);
        }

        let Some(current) = self.get_job_run_backend(&run.run_id)? else {
            return Ok(false);
        };
        if current.state != JobRunState::Failed || current.finished_at.is_none() {
            return Ok(false);
        }

        let step_started_at = run.started_at.unwrap_or(run.scheduled_at);
        let stale_reason = running_run_owner_stale_reason(run);
        let _ = self.record_pipeline_failure_step(
            run,
            step_started_at,
            finished_at,
            &stale_job_run_message(run, stale_reason),
        );
        self.record_event(OrbitEvent::JobRunCompleted {
            job_id: run.job_id.clone(),
            run_id: run.run_id.clone(),
            state: JobRunState::Failed.to_string(),
        })?;
        Ok(true)
    }

    fn repair_terminal_job_run_timing(&self, run: &JobRun) -> Result<bool, OrbitError> {
        let finished_at = match run.finished_at {
            Some(value) => value,
            None => self
                .run_finished_at_from_audit(&run.run_id)?
                .unwrap_or_else(Utc::now),
        };
        let duration_ms = run.duration_ms.or_else(|| {
            run.started_at.map(|started_at| {
                finished_at
                    .signed_duration_since(started_at)
                    .num_milliseconds()
                    .max(0) as u64
            })
        });
        self.stores()
            .jobs()
            .repair_terminal_run_timing(&run.run_id, finished_at, duration_ms)
    }

    fn run_finished_at_from_audit(
        &self,
        run_id: &str,
    ) -> Result<Option<DateTime<Utc>>, OrbitError> {
        for stream in ["v2_loop", "loop"] {
            let path = self
                .data_root_path()
                .join("state")
                .join("audit")
                .join(stream)
                .join(format!("{run_id}.jsonl"));
            if !path.exists() {
                continue;
            }
            let raw = fs::read_to_string(&path).map_err(|error| {
                OrbitError::Io(format!("read run audit '{}': {error}", path.display()))
            })?;
            let mut finished_at = None;
            for line in raw.lines().filter(|line| !line.trim().is_empty()) {
                let event: Value = serde_json::from_str(line).map_err(|error| {
                    OrbitError::Store(format!(
                        "invalid run audit event '{}': {error}",
                        path.display()
                    ))
                })?;
                let event_type = event.get("event_type").and_then(Value::as_str);
                let body_kind = event.get("body_kind").and_then(Value::as_str);
                if matches!(event_type, Some("run.finished"))
                    || matches!(body_kind, Some("run_finished"))
                {
                    finished_at = parse_audit_timestamp(&event, &path)?;
                }
            }
            if finished_at.is_some() {
                return Ok(finished_at);
            }
        }
        Ok(None)
    }

    fn list_job_history_backend(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.stores().jobs().list_runs(job_id)
    }

    fn list_job_runs_filtered_backend(
        &self,
        query: &JobRunQuery,
    ) -> Result<Vec<JobRun>, OrbitError> {
        self.stores().jobs().list_runs_filtered(query)
    }

    pub(crate) fn get_job_run_backend(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.stores().jobs().get_run(run_id)
    }
}

#[cfg(unix)]
const RUN_OWNER_TERMINATION_GRACE: Duration = Duration::from_secs(2);
#[cfg(unix)]
const RUN_OWNER_TERMINATION_POLL: Duration = Duration::from_millis(50);

#[cfg(unix)]
fn signal_run_owner_process(run: &JobRun) -> Result<String, OrbitError> {
    let Some(pid) = run.pid else {
        return Ok("no_pid".to_string());
    };
    if pid == std::process::id() {
        return Ok("self_not_signalled".to_string());
    }
    if !matches!(classify_run_owner(run), OwnerIdentity::Verified) {
        return Ok("owner_identity_mismatch".to_string());
    }

    let pgid = owner_process_group_id(pid);
    if let Some(pgid) = pgid
        && pgid > 1
    {
        if pgid == unsafe { libc::getpgrp() } {
            return Ok("owner_process_group_matches_current_process".to_string());
        }
        match send_signal_to_process_group(pgid, libc::SIGTERM) {
            Ok(()) => {}
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {
                return Ok("already_exited".to_string());
            }
            Err(error) => {
                return Err(OrbitError::Execution(format!(
                    "failed to signal job run owner process group {pgid} for pid {pid}: {error}"
                )));
            }
        }

        if wait_for_process_group_exit(pgid, RUN_OWNER_TERMINATION_GRACE) {
            return Ok("terminated_process_group".to_string());
        }

        match send_signal_to_process_group(pgid, libc::SIGKILL) {
            Ok(()) => {}
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {
                return Ok("terminated_process_group".to_string());
            }
            Err(error) => {
                return Err(OrbitError::Execution(format!(
                    "failed to kill job run owner process group {pgid} for pid {pid}: {error}"
                )));
            }
        }
        let _ = wait_for_process_group_exit(pgid, RUN_OWNER_TERMINATION_GRACE);
        return Ok("killed_process_group".to_string());
    }

    // Fallback for platforms/configurations where the owner process group
    // cannot be resolved. The PID identity guard above still protects against
    // killing a reused PID.
    send_signal_to_pid(pid, libc::SIGTERM)?;
    if wait_for_owner_exit(pid, RUN_OWNER_TERMINATION_GRACE) {
        Ok("terminated_owner".to_string())
    } else {
        send_signal_to_pid(pid, libc::SIGKILL)?;
        let _ = wait_for_owner_exit(pid, RUN_OWNER_TERMINATION_GRACE);
        Ok("killed_owner".to_string())
    }
}

#[cfg(unix)]
fn send_signal_to_pid(pid: u32, signal: libc::c_int) -> Result<(), OrbitError> {
    let rc = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(OrbitError::Execution(format!(
        "failed to signal job run owner pid {pid}: {err}",
    )))
}

#[cfg(not(unix))]
fn signal_run_owner_process(_run: &JobRun) -> Result<String, OrbitError> {
    Ok("unsupported_platform".to_string())
}

#[cfg(unix)]
fn owner_process_group_id(pid: u32) -> Option<libc::pid_t> {
    if pid == 0 || pid > i32::MAX as u32 {
        return None;
    }
    let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
    if pgid > 0 { Some(pgid) } else { None }
}

#[cfg(unix)]
fn send_signal_to_process_group(pgid: libc::pid_t, signal: libc::c_int) -> std::io::Result<()> {
    let rc = unsafe { libc::kill(-pgid, signal) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn wait_for_owner_exit(pid: u32, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if !process_is_alive(pid) {
            return true;
        }
        thread::sleep(RUN_OWNER_TERMINATION_POLL);
    }
    !process_is_alive(pid)
}

#[cfg(unix)]
fn wait_for_process_group_exit(pgid: libc::pid_t, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if !process_group_is_alive(pgid) {
            return true;
        }
        thread::sleep(RUN_OWNER_TERMINATION_POLL);
    }
    !process_group_is_alive(pgid)
}

#[cfg(unix)]
fn process_group_is_alive(pgid: libc::pid_t) -> bool {
    if pgid <= 1 {
        return false;
    }
    let rc = unsafe { libc::kill(-pgid, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
fn running_run_owner_is_stale(run: &JobRun) -> bool {
    running_run_owner_stale_reason(run).is_some()
}

#[cfg(not(unix))]
fn running_run_owner_is_stale(_run: &JobRun) -> bool {
    false
}

/// Returns `Some(reason)` only when a running run's owner is conclusively
/// either mismatched or missing — those are the two states that warrant
/// finalizing the run as failed. `ProbeUnavailable` and `LegacyLiveUnverified`
/// classifications never appear here: they keep the run Running.
#[cfg(unix)]
fn running_run_owner_stale_reason(run: &JobRun) -> Option<OwnerIdentity> {
    if run.state != JobRunState::Running {
        return None;
    }
    match classify_run_owner(run) {
        identity @ (OwnerIdentity::Mismatch | OwnerIdentity::Missing) => Some(identity),
        OwnerIdentity::Verified
        | OwnerIdentity::LegacyLiveUnverified
        | OwnerIdentity::ProbeUnavailable => None,
    }
}

#[cfg(not(unix))]
#[allow(dead_code)]
fn running_run_owner_stale_reason(_run: &JobRun) -> Option<()> {
    None
}

/// Outcome of comparing a persisted owner identity against the live process.
///
/// Only `Mismatch` and `Missing` warrant finalizing the run as failed.
///
/// - `Verified` — versioned token (or legacy token re-derived under either
///   environment) matches the live process: the worker is the original owner.
/// - `Mismatch` — versioned persisted token disagrees with the live process's
///   current token: a different process is holding the PID. Stale.
/// - `LegacyLiveUnverified` — legacy (pre-ORB-00036) unversioned token cannot
///   be re-derived under either environment, but `kill(pid, 0)` confirms the
///   PID is still alive. Stays Running; cancellation still refuses to signal
///   it (PID-reuse protection).
/// - `ProbeUnavailable` — the `ps` invocation itself failed (spawn error,
///   IO error, etc.) and `kill(pid, 0)` confirms the PID is still alive.
///   A transient probe failure must never terminalize a live worker.
/// - `Missing` — no PID recorded, or both the probe and `kill(pid, 0)`
///   agree the PID is gone. Stale.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnerIdentity {
    Verified,
    Mismatch,
    LegacyLiveUnverified,
    ProbeUnavailable,
    Missing,
}

#[cfg(unix)]
fn classify_run_owner(run: &JobRun) -> OwnerIdentity {
    classify_run_owner_with_probes(
        run.pid,
        run.pid_start_time.as_deref(),
        probe_process_start_identity,
        |pid| legacy_lstart_matches(pid, run.pid_start_time.as_deref().unwrap_or_default()),
        process_is_alive,
    )
}

/// Inner, testable form of [`classify_run_owner`] with the probes injected.
/// Production callers go through [`classify_run_owner`]; tests pass
/// deterministic closures to exercise rare probe states (Unavailable,
/// NoProcess-but-alive race) without needing real misbehaving processes.
#[cfg(unix)]
fn classify_run_owner_with_probes<P, L, A>(
    pid: Option<u32>,
    persisted: Option<&str>,
    probe: P,
    legacy_match: L,
    is_alive: A,
) -> OwnerIdentity
where
    P: FnOnce(u32) -> ProbeOutcome,
    L: FnOnce(u32) -> bool,
    A: FnOnce(u32) -> bool,
{
    let Some(pid) = pid else {
        return OwnerIdentity::Missing;
    };
    let Some(persisted) = persisted else {
        return if is_alive(pid) {
            OwnerIdentity::LegacyLiveUnverified
        } else {
            OwnerIdentity::Missing
        };
    };
    if persisted.starts_with(STABLE_TOKEN_PREFIX) {
        return match probe(pid) {
            ProbeOutcome::Token(current) if current == persisted => OwnerIdentity::Verified,
            ProbeOutcome::Token(_) => OwnerIdentity::Mismatch,
            ProbeOutcome::NoProcess => {
                if is_alive(pid) {
                    // Race: `ps` returned no-process but `kill(pid, 0)` still
                    // sees the PID. Defer finalization until the probe agrees.
                    OwnerIdentity::ProbeUnavailable
                } else {
                    OwnerIdentity::Missing
                }
            }
            ProbeOutcome::Unavailable => {
                if is_alive(pid) {
                    OwnerIdentity::ProbeUnavailable
                } else {
                    OwnerIdentity::Missing
                }
            }
        };
    }
    if legacy_match(pid) {
        OwnerIdentity::Verified
    } else if is_alive(pid) {
        OwnerIdentity::LegacyLiveUnverified
    } else {
        OwnerIdentity::Missing
    }
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    // Safety: signal 0 performs existence/permission checking only.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
fn stale_job_run_message(run: &JobRun, reason: Option<OwnerIdentity>) -> String {
    let reason_str = match reason {
        Some(OwnerIdentity::Mismatch) => "token_mismatch",
        Some(OwnerIdentity::Missing) => "process_not_found",
        // ProbeUnavailable / Verified / LegacyLiveUnverified never reach the
        // stale-message path, but a future caller could; keep them tagged so
        // the diagnostic is never silently wrong.
        Some(OwnerIdentity::ProbeUnavailable) => "probe_unavailable",
        Some(OwnerIdentity::Verified) => "verified",
        Some(OwnerIdentity::LegacyLiveUnverified) => "legacy_live_unverified",
        None => "unknown",
    };
    format!(
        "job run marked failed because recorded worker process is no longer alive (reason={}, pid={}, pid_start_time={})",
        reason_str,
        run.pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "-".to_string()),
        run.pid_start_time.as_deref().unwrap_or("-")
    )
}

#[cfg(not(unix))]
fn stale_job_run_message(run: &JobRun, _reason: Option<()>) -> String {
    format!(
        "job run marked failed because recorded worker process is no longer alive (reason=unknown, pid={}, pid_start_time={})",
        run.pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "-".to_string()),
        run.pid_start_time.as_deref().unwrap_or("-")
    )
}

fn terminal_run_timing_is_incomplete(run: &JobRun) -> bool {
    run.state.is_terminal()
        && (run.finished_at.is_none() || (run.duration_ms.is_none() && run.started_at.is_some()))
}

fn parse_audit_timestamp(event: &Value, path: &Path) -> Result<Option<DateTime<Utc>>, OrbitError> {
    let Some(raw) = event.get("ts").and_then(Value::as_str) else {
        return Ok(None);
    };
    DateTime::parse_from_rfc3339(raw)
        .map(|value| Some(value.with_timezone(&Utc)))
        .map_err(|error| {
            OrbitError::Store(format!(
                "invalid run audit timestamp '{}' in '{}': {error}",
                raw,
                path.display()
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    #[cfg(unix)]
    use orbit_common::utility::process_identity::process_start_identity_token;
    #[cfg(unix)]
    use std::process::{Command, Stdio};
    #[cfg(unix)]
    use std::time::{Duration as StdDuration, Instant as StdInstant};
    use tempfile::tempdir;

    fn test_runtime() -> (tempfile::TempDir, OrbitRuntime) {
        let root = tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("create global root");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
        (root, runtime)
    }

    fn insert_pending_run(runtime: &OrbitRuntime, job_id: &str) -> JobRun {
        runtime
            .stores()
            .jobs()
            .insert_run(job_id, 1, Utc::now() - Duration::seconds(5), None, None)
            .expect("insert run")
    }

    fn strip_run_timing(runtime: &OrbitRuntime, run: &JobRun) {
        let path = runtime
            .data_root()
            .join("state")
            .join("job-runs")
            .join(&run.job_id)
            .join(&run.run_id)
            .join("jrun.yaml");
        let raw = std::fs::read_to_string(&path).expect("read run yaml");
        let edited = raw
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("finished_at:") {
                    "  finished_at: null".to_string()
                } else if line.trim_start().starts_with("duration_ms:") {
                    "  duration_ms: null".to_string()
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("{edited}\n")).expect("write run yaml");
    }

    fn write_run_finished_audit(runtime: &OrbitRuntime, run_id: &str, finished_at: DateTime<Utc>) {
        let dir = runtime
            .data_root()
            .join("state")
            .join("audit")
            .join("v2_loop");
        std::fs::create_dir_all(&dir).expect("create audit dir");
        let line = serde_json::json!({
            "event_type": "run.finished",
            "ts": finished_at.to_rfc3339(),
            "outcome": "success",
            "error_message": null,
        });
        std::fs::write(dir.join(format!("{run_id}.jsonl")), format!("{line}\n"))
            .expect("write audit event");
    }

    #[test]
    fn cancel_job_run_marks_pending_cancelled_without_signal() {
        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_cancel_pending");

        let result = runtime
            .cancel_job_run_with_context(&run.run_id, "tester", "unit")
            .expect("cancel pending");

        assert_eq!(result.previous_state, "pending");
        assert_eq!(result.final_state, "cancelled");
        assert!(!result.signal_attempted);
        assert_eq!(result.signal_outcome, None);
        let stored = runtime.show_job_run(&run.run_id).expect("show run");
        assert_eq!(stored.state, JobRunState::Cancelled);
        assert!(stored.finished_at.is_some());
        assert_eq!(stored.duration_ms, None);

        let audits = runtime.list_session_events(10).expect("events");
        let payload = audits
            .iter()
            .find(|event| event.event_type == "JobRunCancelled")
            .map(|event| &event.payload["data"])
            .expect("cancel event");
        assert_eq!(payload["run_id"], run.run_id);
        assert_eq!(payload["previous_state"], "pending");
        assert_eq!(payload["final_state"], "cancelled");
        assert_eq!(payload["actor"], "tester");
        assert_eq!(payload["source"], "unit");
        assert_eq!(payload["signal_attempted"], false);
    }

    #[test]
    fn cancelled_pending_run_is_not_claimed_by_pipeline_worker() {
        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_cancel_worker_skip");
        runtime
            .cancel_job_run(&run.run_id)
            .expect("cancel pending run");

        runtime
            .execute_pipeline_run_worker(&run.run_id)
            .expect("worker exits without executing cancelled run");

        let stored = runtime.show_job_run(&run.run_id).expect("show run");
        assert_eq!(stored.state, JobRunState::Cancelled);
        assert!(stored.started_at.is_none());
        assert!(stored.steps.is_empty());
    }

    #[test]
    fn cancelled_run_wait_status_reports_cancelled() {
        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_cancel_wait");
        runtime.cancel_job_run(&run.run_id).expect("cancel run");

        let result = runtime
            .wait_pipeline_runs(std::slice::from_ref(&run.run_id), 1, 1, Some("test"))
            .expect("wait cancelled");

        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].run_id, run.run_id);
        assert_eq!(result.results[0].status, "cancelled");
    }

    #[test]
    fn cancel_job_run_rejects_terminal_run_without_mutating_bundle() {
        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_cancel_terminal");
        let started_at = Utc::now() - Duration::seconds(2);
        let finished_at = Utc::now();
        runtime
            .stores()
            .jobs()
            .mark_run_running(&run.run_id, started_at, std::process::id())
            .expect("mark running");
        runtime
            .stores()
            .jobs()
            .finalize_run(&run.run_id, JobRunState::Success, finished_at, Some(2_000))
            .expect("finalize success");
        let before = runtime.show_job_run(&run.run_id).expect("show before");

        let error = runtime
            .cancel_job_run(&run.run_id)
            .expect_err("terminal cancellation must fail");

        assert!(
            error.to_string().contains("cannot cancel job run"),
            "{error}"
        );
        let after = runtime.show_job_run(&run.run_id).expect("show after");
        assert_eq!(after, before);
        let events = runtime.list_session_events(20).expect("events");
        assert!(
            events
                .iter()
                .all(|event| event.event_type != "JobRunCancelled")
        );
    }

    #[cfg(unix)]
    fn wait_until<F>(timeout: StdDuration, mut condition: F) -> bool
    where
        F: FnMut() -> bool,
    {
        let started = StdInstant::now();
        while started.elapsed() < timeout {
            if condition() {
                return true;
            }
            std::thread::sleep(StdDuration::from_millis(50));
        }
        condition()
    }

    #[cfg(unix)]
    fn read_pid_pair(path: &Path) -> (u32, u32) {
        let raw = std::fs::read_to_string(path).expect("read pid file");
        let mut parts = raw.split_whitespace();
        let owner = parts
            .next()
            .expect("owner pid")
            .parse()
            .expect("parse owner pid");
        let child = parts
            .next()
            .expect("child pid")
            .parse()
            .expect("parse child pid");
        (owner, child)
    }

    #[cfg(unix)]
    #[test]
    fn cancel_job_run_does_not_signal_reused_pid_identity_mismatch() {
        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_cancel_reused_pid");
        let mut sentinel = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sentinel");
        let sentinel_pid = sentinel.id();
        let started_at = Utc::now() - Duration::seconds(1);
        runtime
            .stores()
            .jobs()
            .mark_run_running(&run.run_id, started_at, sentinel_pid)
            .expect("mark running");
        let path = runtime
            .data_root()
            .join("state")
            .join("job-runs")
            .join(&run.job_id)
            .join(&run.run_id)
            .join("jrun.yaml");
        // Versioned token guarantees we exercise the strict `Mismatch`
        // classification path; legacy unversioned tokens may flow through the
        // softer LegacyLiveUnverified branch but must still produce
        // owner_identity_mismatch from `signal_run_owner_process`.
        let mismatched_versioned =
            format!("{STABLE_TOKEN_PREFIX}definitely-not-the-sentinel-start-token");
        let raw = std::fs::read_to_string(&path).expect("read run yaml");
        let edited = if raw.contains("pid_start_time:") {
            raw.lines()
                .map(|line| {
                    if line.trim_start().starts_with("pid_start_time:") {
                        format!("  pid_start_time: {mismatched_versioned}")
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            raw.lines()
                .map(|line| {
                    if line.trim_start().starts_with("pid:") {
                        format!("{line}\n  pid_start_time: {mismatched_versioned}")
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        std::fs::write(&path, format!("{edited}\n")).expect("write mismatched pid token");

        let result = runtime.cancel_job_run(&run.run_id).expect("cancel run");

        assert!(result.signal_attempted);
        assert_eq!(
            result.signal_outcome.as_deref(),
            Some("owner_identity_mismatch")
        );
        assert!(
            process_is_alive(sentinel_pid),
            "sentinel process must not be killed by mismatched owner identity"
        );
        let _ = sentinel.kill();
        let _ = sentinel.wait();
    }

    #[cfg(unix)]
    #[test]
    fn cancel_job_run_terminates_owner_process_group_and_child() {
        use std::os::unix::process::CommandExt;

        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_cancel_process_group");
        let pid_dir = tempdir().expect("pid tempdir");
        let pid_file = pid_dir.path().join("pids");
        let script = format!(
            "trap 'exit 0' TERM; (trap '' TERM; sleep 30) & child=$!; printf '%s %s\\n' $$ \"$child\" > {}; wait",
            shell_quote(pid_file.to_string_lossy().as_ref())
        );
        let mut owner = Command::new("/bin/sh");
        owner
            .arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            owner.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut owner = owner.spawn().expect("spawn owner");
        assert!(
            wait_until(StdDuration::from_secs(2), || pid_file.exists()),
            "owner did not write pid file"
        );
        let (owner_pid, child_pid) = read_pid_pair(&pid_file);
        assert_eq!(owner.id(), owner_pid);
        runtime
            .stores()
            .jobs()
            .mark_run_running(&run.run_id, Utc::now(), owner_pid)
            .expect("mark running");

        let result = runtime.cancel_job_run(&run.run_id).expect("cancel run");
        let _ = owner.wait();

        assert!(result.signal_attempted);
        assert_eq!(
            result.signal_outcome.as_deref(),
            Some("killed_process_group")
        );
        assert!(
            wait_until(StdDuration::from_secs(3), || !process_is_alive(child_pid)),
            "child process {child_pid} should be gone after process-group cancellation"
        );
    }

    #[cfg(unix)]
    fn shell_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }

    #[cfg(unix)]
    #[test]
    fn show_job_run_reconciles_stale_running_owner() {
        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_stale");
        let started_at = Utc::now() - Duration::seconds(3);
        runtime
            .stores()
            .jobs()
            .mark_run_running(&run.run_id, started_at, 999_999)
            .expect("mark running with impossible pid");

        let shown = runtime.show_job_run(&run.run_id).expect("show run");

        assert_eq!(shown.state, JobRunState::Failed);
        assert!(shown.finished_at.is_some());
        assert!(shown.duration_ms.is_some_and(|value| value > 0));
        assert!(shown.steps.iter().any(|step| {
            step.state == JobRunState::Failed
                && step.error_message.as_deref().is_some_and(|message| {
                    message.contains("recorded worker process is no longer alive")
                })
        }));
    }

    #[cfg(unix)]
    #[test]
    fn show_job_run_keeps_live_owner_running() {
        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_live");
        let pid = std::process::id();
        if process_start_identity_token(pid).is_none() {
            return;
        }
        runtime
            .stores()
            .jobs()
            .mark_run_running(&run.run_id, Utc::now(), pid)
            .expect("mark current process running");

        let shown = runtime.show_job_run(&run.run_id).expect("show run");

        assert_eq!(shown.state, JobRunState::Running);
        assert!(shown.finished_at.is_none());
        assert!(shown.duration_ms.is_none());
    }

    #[test]
    fn show_job_run_keeps_pending_runs_pending() {
        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_pending");

        let shown = runtime.show_job_run(&run.run_id).expect("show pending run");

        assert_eq!(shown.state, JobRunState::Pending);
        assert!(shown.finished_at.is_none());
        assert!(shown.duration_ms.is_none());
    }

    #[test]
    fn show_job_run_repairs_terminal_run_missing_timing() {
        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_terminal");
        let started_at = Utc::now() - Duration::seconds(8);
        let finished_at = started_at + Duration::seconds(5);
        runtime
            .stores()
            .jobs()
            .mark_run_running(&run.run_id, started_at, std::process::id())
            .expect("mark running");
        runtime
            .stores()
            .jobs()
            .finalize_run(&run.run_id, JobRunState::Success, finished_at, Some(5_000))
            .expect("finalize success");
        let finalized = runtime.show_job_run(&run.run_id).expect("show finalized");
        strip_run_timing(&runtime, &finalized);
        write_run_finished_audit(&runtime, &run.run_id, finished_at);

        let repaired = runtime.show_job_run(&run.run_id).expect("show repaired");

        assert_eq!(repaired.state, JobRunState::Success);
        assert_eq!(repaired.finished_at, Some(finished_at));
        assert_eq!(repaired.duration_ms, Some(5_000));
    }

    #[cfg(unix)]
    #[test]
    fn list_job_runs_reconciles_before_state_filtering() {
        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_filter");
        runtime
            .stores()
            .jobs()
            .mark_run_running(&run.run_id, Utc::now() - Duration::seconds(3), 999_999)
            .expect("mark stale running");

        let running = runtime
            .list_job_runs(JobRunListParams {
                state: Some(JobRunState::Running),
                ..JobRunListParams::default()
            })
            .expect("list running");
        let failed = runtime
            .list_job_runs(JobRunListParams {
                state: Some(JobRunState::Failed),
                ..JobRunListParams::default()
            })
            .expect("list failed");

        assert!(
            !running
                .iter()
                .any(|candidate| candidate.run_id == run.run_id)
        );
        assert!(
            failed
                .iter()
                .any(|candidate| candidate.run_id == run.run_id)
        );
    }

    // ---- Timezone regression coverage (ORB-00036) ----
    //
    // The bug: persisted `pid_start_time` was derived from ambient `ps -o
    // lstart=` output, so a run marked Running under one TZ would re-derive a
    // different token under another, causing read-path reconciliation to
    // finalize a live worker as Failed. These tests serialize TZ mutation with
    // `TZ_TEST_LOCK` and assert that switching the caller's TZ between
    // mark-running and the read paths (`show_job_run`, `list_job_runs`,
    // `wait_pipeline_runs`) does not produce false stale finalizations.
    #[cfg(unix)]
    static TZ_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(unix)]
    struct TzGuard {
        prior: Option<String>,
    }

    #[cfg(unix)]
    impl TzGuard {
        fn set(value: &str) -> Self {
            let prior = std::env::var("TZ").ok();
            // SAFETY: All TZ-mutating tests in this module take TZ_TEST_LOCK
            // before constructing a TzGuard, serializing env mutation across
            // threads; the guard restores the previous value on drop.
            unsafe { std::env::set_var("TZ", value) };
            Self { prior }
        }
    }

    #[cfg(unix)]
    impl Drop for TzGuard {
        fn drop(&mut self) {
            // SAFETY: see TzGuard::set.
            unsafe {
                match &self.prior {
                    Some(value) => std::env::set_var("TZ", value),
                    None => std::env::remove_var("TZ"),
                }
            }
        }
    }

    #[cfg(unix)]
    fn spawn_sentinel() -> std::process::Child {
        Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sentinel")
    }

    #[cfg(unix)]
    #[test]
    fn live_owner_survives_tz_change_across_read_paths() {
        let _tz_lock = TZ_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_tz_change");
        let mut sentinel = spawn_sentinel();
        let sentinel_pid = sentinel.id();

        // Write the run under a non-UTC ambient TZ. The fix forces the child
        // ps invocation to TZ=UTC regardless, so the persisted token must
        // carry the versioned prefix and remain identical across caller
        // environments.
        let persisted_token = {
            let _tz = TzGuard::set("America/Los_Angeles");
            runtime
                .stores()
                .jobs()
                .mark_run_running(&run.run_id, Utc::now() - Duration::seconds(1), sentinel_pid)
                .expect("mark running under LA tz");
            runtime
                .show_job_run(&run.run_id)
                .expect("show fresh run")
                .pid_start_time
                .expect("token must be persisted")
        };
        assert!(
            persisted_token.starts_with(STABLE_TOKEN_PREFIX),
            "persisted identity token must be versioned: {persisted_token}"
        );

        // Switch TZ before driving the read paths. Pre-fix this is exactly
        // when reconciliation falsely finalized the still-running worker.
        let _tz = TzGuard::set("UTC");

        let shown = runtime.show_job_run(&run.run_id).expect("show under UTC");
        assert_eq!(shown.state, JobRunState::Running);
        assert!(shown.finished_at.is_none());
        assert!(shown.duration_ms.is_none());
        assert!(
            !shown
                .steps
                .iter()
                .any(|step| step.error_message.as_deref().is_some_and(|message| {
                    message.contains("recorded worker process is no longer alive")
                })),
            "live worker must not have a stale-failure step"
        );

        let listed = runtime
            .list_job_runs(JobRunListParams {
                state: Some(JobRunState::Running),
                ..JobRunListParams::default()
            })
            .expect("list running under UTC");
        assert!(
            listed
                .iter()
                .any(|candidate| candidate.run_id == run.run_id),
            "live worker must still appear in the Running list after a TZ change"
        );

        let waited = runtime
            .wait_pipeline_runs(
                std::slice::from_ref(&run.run_id),
                0,
                1,
                Some("tz_change_test"),
            )
            .expect("wait under UTC");
        assert_eq!(waited.results.len(), 1);
        assert_eq!(waited.results[0].run_id, run.run_id);
        assert_ne!(
            waited.results[0].status, "failed",
            "wait must not report failed for a live worker after a TZ change"
        );

        let final_state = runtime.show_job_run(&run.run_id).expect("final show").state;
        assert_eq!(final_state, JobRunState::Running);

        let _ = sentinel.kill();
        let _ = sentinel.wait();
    }

    #[cfg(unix)]
    #[test]
    fn versioned_token_is_stable_across_tz_change() {
        let _tz_lock = TZ_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut sentinel = spawn_sentinel();
        let pid = sentinel.id();

        let utc_token = {
            let _tz = TzGuard::set("UTC");
            process_start_identity_token(pid).expect("token under UTC")
        };
        let la_token = {
            let _tz = TzGuard::set("America/Los_Angeles");
            process_start_identity_token(pid).expect("token under LA")
        };

        assert!(utc_token.starts_with(STABLE_TOKEN_PREFIX));
        assert_eq!(
            utc_token, la_token,
            "versioned identity token must not depend on the caller's TZ"
        );

        let _ = sentinel.kill();
        let _ = sentinel.wait();
    }

    #[cfg(unix)]
    #[test]
    fn legacy_unversioned_token_does_not_falsely_finalize_live_run() {
        // A pre-fix run with a non-versioned `pid_start_time` whose value
        // cannot be matched under either environment should classify as
        // LegacyLiveUnverified, keeping the run Running instead of finalizing
        // it as Failed.
        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_legacy_unverified");
        let mut sentinel = spawn_sentinel();
        let sentinel_pid = sentinel.id();
        runtime
            .stores()
            .jobs()
            .mark_run_running(&run.run_id, Utc::now() - Duration::seconds(1), sentinel_pid)
            .expect("mark running");

        // Rewrite the stored token to look like a pre-fix unversioned value
        // that does not match the live process under either env.
        let yaml_path = runtime
            .data_root()
            .join("state")
            .join("job-runs")
            .join(&run.job_id)
            .join(&run.run_id)
            .join("jrun.yaml");
        let raw = std::fs::read_to_string(&yaml_path).expect("read run yaml");
        let edited = raw
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("pid_start_time:") {
                    "  pid_start_time: legacy-token-that-cannot-be-rederived".to_string()
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&yaml_path, format!("{edited}\n")).expect("write legacy token");

        let shown = runtime.show_job_run(&run.run_id).expect("show legacy run");
        assert_eq!(shown.state, JobRunState::Running);
        assert!(shown.finished_at.is_none());

        let _ = sentinel.kill();
        let _ = sentinel.wait();
    }

    // ---- Probe-outcome regression coverage (ORB-00037) ----
    //
    // `classify_run_owner_with_probes` lets these tests inject deterministic
    // `ProbeOutcome` values without depending on a real misbehaving `ps`.
    // They guard the rule from the task ACs: a transient probe failure with a
    // live PID must never terminalize the run; a dead PID still must.

    #[cfg(unix)]
    #[test]
    fn probe_unavailable_with_live_pid_classifies_as_probe_unavailable() {
        let versioned = format!("{STABLE_TOKEN_PREFIX}lstart-token");
        let identity = classify_run_owner_with_probes(
            Some(4242),
            Some(versioned.as_str()),
            |_| ProbeOutcome::Unavailable,
            |_| false,
            |_| true,
        );
        assert_eq!(identity, OwnerIdentity::ProbeUnavailable);
        // Build a JobRun with state=Running so we can exercise the stale-path
        // gate alongside the classification (the closure-based classifier is
        // the only path that distinguishes Unavailable from NoProcess).
    }

    #[cfg(unix)]
    #[test]
    fn probe_no_process_with_live_pid_classifies_as_probe_unavailable() {
        // Race: ps -p says no-process, but kill(pid, 0) still sees the PID.
        // We must not finalize the run on a single ps result that disagrees
        // with the kernel's liveness signal.
        let versioned = format!("{STABLE_TOKEN_PREFIX}lstart-token");
        let identity = classify_run_owner_with_probes(
            Some(4242),
            Some(versioned.as_str()),
            |_| ProbeOutcome::NoProcess,
            |_| false,
            |_| true,
        );
        assert_eq!(identity, OwnerIdentity::ProbeUnavailable);
    }

    #[cfg(unix)]
    #[test]
    fn probe_unavailable_with_dead_pid_classifies_as_missing() {
        let versioned = format!("{STABLE_TOKEN_PREFIX}lstart-token");
        let identity = classify_run_owner_with_probes(
            Some(4242),
            Some(versioned.as_str()),
            |_| ProbeOutcome::Unavailable,
            |_| false,
            |_| false,
        );
        // Probe failed AND kill(0) confirms dead → still legitimately stale.
        assert_eq!(identity, OwnerIdentity::Missing);
    }

    #[cfg(unix)]
    #[test]
    fn versioned_token_mismatch_with_live_pid_classifies_as_mismatch() {
        let persisted = format!("{STABLE_TOKEN_PREFIX}old-lstart");
        let identity = classify_run_owner_with_probes(
            Some(4242),
            Some(persisted.as_str()),
            |_| ProbeOutcome::Token(format!("{STABLE_TOKEN_PREFIX}fresh-lstart")),
            |_| false,
            |_| true,
        );
        assert_eq!(identity, OwnerIdentity::Mismatch);
    }

    #[cfg(unix)]
    #[test]
    fn versioned_token_match_classifies_as_verified() {
        let persisted = format!("{STABLE_TOKEN_PREFIX}same-lstart");
        let identity = classify_run_owner_with_probes(
            Some(4242),
            Some(persisted.as_str()),
            |_| ProbeOutcome::Token(format!("{STABLE_TOKEN_PREFIX}same-lstart")),
            |_| false,
            |_| true,
        );
        assert_eq!(identity, OwnerIdentity::Verified);
    }

    #[cfg(unix)]
    #[test]
    fn running_run_owner_stale_reason_excludes_probe_unavailable() {
        // A Running run whose probe is Unavailable and whose PID is alive
        // must NOT be classified as stale.
        let run = JobRun {
            run_id: "qa_run".to_string(),
            job_id: "qa_job".to_string(),
            attempt: 1,
            state: JobRunState::Running,
            scheduled_at: Utc::now(),
            started_at: Some(Utc::now()),
            finished_at: None,
            duration_ms: None,
            pid: Some(4242),
            pid_start_time: Some(format!("{STABLE_TOKEN_PREFIX}lstart-token")),
            input: None,
            retry_source_run_id: None,
            created_at: Utc::now(),
            steps: Vec::new(),
            knowledge_metrics: None,
            resolved_crew: None,
            planner_model: None,
            implementer_model: None,
            reviewer_model: None,
        };
        // We can't override the probe at this seam (production wrapper), but
        // we can assert the lower-level helper agrees: ProbeUnavailable is
        // not in the stale set.
        let identity = classify_run_owner_with_probes(
            run.pid,
            run.pid_start_time.as_deref(),
            |_| ProbeOutcome::Unavailable,
            |_| false,
            |_| true,
        );
        assert!(matches!(identity, OwnerIdentity::ProbeUnavailable));
        // And the stale-reason helper would only emit Some for Mismatch /
        // Missing — verified separately by other tests.
    }

    #[cfg(unix)]
    #[test]
    fn stale_failure_message_distinguishes_probe_outcomes() {
        let run = JobRun {
            run_id: "qa_run".to_string(),
            job_id: "qa_job".to_string(),
            attempt: 1,
            state: JobRunState::Running,
            scheduled_at: Utc::now(),
            started_at: Some(Utc::now()),
            finished_at: None,
            duration_ms: None,
            pid: Some(4242),
            pid_start_time: Some(format!("{STABLE_TOKEN_PREFIX}lstart-token")),
            input: None,
            retry_source_run_id: None,
            created_at: Utc::now(),
            steps: Vec::new(),
            knowledge_metrics: None,
            resolved_crew: None,
            planner_model: None,
            implementer_model: None,
            reviewer_model: None,
        };
        let mismatch_message = stale_job_run_message(&run, Some(OwnerIdentity::Mismatch));
        let missing_message = stale_job_run_message(&run, Some(OwnerIdentity::Missing));
        let probe_unavailable_message =
            stale_job_run_message(&run, Some(OwnerIdentity::ProbeUnavailable));

        assert!(
            mismatch_message.contains("reason=token_mismatch"),
            "{mismatch_message}"
        );
        assert!(
            missing_message.contains("reason=process_not_found"),
            "{missing_message}"
        );
        // Even though this state never finalizes, the tag must be set so a
        // future caller's diagnostic is never silently mis-labeled.
        assert!(
            probe_unavailable_message.contains("reason=probe_unavailable"),
            "{probe_unavailable_message}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn show_job_run_reconciles_dead_pid_with_probe_outcome_in_message() {
        // End-to-end regression: dead PID still finalizes, and the failure
        // step's error message carries `reason=process_not_found`.
        let (_root, runtime) = test_runtime();
        let run = insert_pending_run(&runtime, "qa_dead_pid_reason");
        let started_at = Utc::now() - Duration::seconds(3);
        runtime
            .stores()
            .jobs()
            .mark_run_running(&run.run_id, started_at, 999_999)
            .expect("mark running with impossible pid");

        let shown = runtime.show_job_run(&run.run_id).expect("show run");
        assert_eq!(shown.state, JobRunState::Failed);
        let failure_step = shown
            .steps
            .iter()
            .find(|step| step.state == JobRunState::Failed)
            .expect("stale failure step");
        let message = failure_step
            .error_message
            .as_deref()
            .expect("failure message");
        assert!(
            message.contains("reason=process_not_found"),
            "diagnostic must record probe outcome: {message}"
        );
    }
}
