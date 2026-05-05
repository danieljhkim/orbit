use chrono::{DateTime, Utc};
use orbit_common::types::{JobRun, JobRunState, OrbitError, OrbitEvent, PipelineState};
use orbit_store::{JobRunQuery, TaskReservationReleaseReason};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::process::Command;
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
            .ok_or_else(|| OrbitError::JobRunNotFound(run_id.to_string()))?;
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
            .ok_or_else(|| OrbitError::JobRunNotFound(run_id.to_string()))?;
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
            .ok_or_else(|| OrbitError::JobRunNotFound(run_id.to_string()))?;
        self.reconcile_stale_job_run(&run)?;
        self.get_job_run_backend(run_id)?
            .ok_or_else(|| OrbitError::JobRunNotFound(run_id.to_string()))
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
        let _ = self.record_pipeline_failure_step(
            run,
            step_started_at,
            finished_at,
            &stale_job_run_message(run),
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
    if !run_owner_identity_matches(run) {
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
    if run.state != JobRunState::Running {
        return false;
    }
    !run_owner_identity_matches(run)
}

#[cfg(not(unix))]
fn running_run_owner_is_stale(_run: &JobRun) -> bool {
    false
}

#[cfg(unix)]
fn run_owner_identity_matches(run: &JobRun) -> bool {
    let Some(pid) = run.pid else {
        return false;
    };
    let Some(expected) = run.pid_start_time.as_deref() else {
        return process_is_alive(pid);
    };
    match process_start_time_token(pid) {
        Some(actual) => actual == expected,
        None => false,
    }
}

#[cfg(not(unix))]
fn run_owner_identity_matches(_run: &JobRun) -> bool {
    false
}

#[cfg(unix)]
fn process_start_time_token(pid: u32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!token.is_empty()).then_some(token)
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

fn stale_job_run_message(run: &JobRun) -> String {
    format!(
        "job run marked failed because recorded worker process is no longer alive (pid={}, pid_start_time={})",
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
    use std::process::Stdio;
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
        let raw = std::fs::read_to_string(&path).expect("read run yaml");
        let edited = if raw.contains("pid_start_time:") {
            raw.lines()
                .map(|line| {
                    if line.trim_start().starts_with("pid_start_time:") {
                        "  pid_start_time: definitely-not-the-sentinel-start-token".to_string()
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
                        format!("{line}\n  pid_start_time: definitely-not-the-sentinel-start-token")
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
        assert!(
            process_start_time_token(pid).is_some(),
            "test requires process start token for current process"
        );
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
}
