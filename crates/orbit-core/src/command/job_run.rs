use chrono::{DateTime, Utc};
use orbit_common::types::{JobRun, JobRunState, OrbitError, OrbitEvent};
use orbit_store::JobRunQuery;
use serde_json::Value;
use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::process::Command;

use crate::OrbitRuntime;

#[derive(Debug, Clone, Default)]
pub struct JobRunListParams {
    pub job_id: Option<String>,
    pub state: Option<JobRunState>,
    pub since: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}

impl OrbitRuntime {
    pub fn cancel_job_run(&self, run_id: &str) -> Result<(), OrbitError> {
        let run = self.show_job_run(run_id)?;
        run.state
            .try_transition(orbit_common::types::RunEvent::Cancel)
            .map_err(|msg| {
                OrbitError::JobValidation(format!("cannot cancel job run '{}': {}", run_id, msg))
            })?;
        let now = chrono::Utc::now();
        let duration_ms = run
            .started_at
            .map(|s| now.signed_duration_since(s).num_milliseconds().max(0) as u64);
        let should_signal_owner = run_owner_identity_matches(&run);
        let owner_pid = run.pid;
        self.stores()
            .jobs()
            .finalize_run(run_id, JobRunState::Cancelled, now, duration_ms)?;
        self.record_event(OrbitEvent::JobRunCancelled {
            job_id: run.job_id,
            run_id: run_id.to_string(),
        })?;
        if let Some(pid) = owner_pid
            && should_signal_owner
        {
            signal_run_owner_process(pid)?;
        }
        Ok(())
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

    fn reconcile_stale_job_run(&self, run: &JobRun) -> Result<bool, OrbitError> {
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
        let changed = self.stores().jobs().finalize_run(
            &run.run_id,
            JobRunState::Failed,
            finished_at,
            duration_ms,
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

    fn get_job_run_backend(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.stores().jobs().get_run(run_id)
    }
}

#[cfg(unix)]
fn signal_run_owner_process(pid: u32) -> Result<(), OrbitError> {
    if pid == std::process::id() {
        return Ok(());
    }

    // Safety: `kill` only sends a signal to the run owner process so it can
    // tear down its active child process tree.
    let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    if rc == 0 {
        return Ok(());
    }

    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }

    Err(OrbitError::Execution(format!(
        "failed to signal job run owner pid {pid}: {err}"
    )))
}

#[cfg(not(unix))]
fn signal_run_owner_process(_pid: u32) -> Result<(), OrbitError> {
    Ok(())
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
        None => process_is_alive(pid),
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
