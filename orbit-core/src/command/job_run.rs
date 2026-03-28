use chrono::{DateTime, Utc};
use orbit_store::JobRunQuery;
use orbit_types::{JobRun, JobRunState, OrbitError, OrbitEvent};

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
        if !matches!(run.state, JobRunState::Pending | JobRunState::Running) {
            return Err(OrbitError::JobValidation(format!(
                "job run '{}' is not active (state: {}); only pending or running runs can be cancelled",
                run_id, run.state
            )));
        }
        let now = chrono::Utc::now();
        let duration_ms = run
            .started_at
            .map(|s| now.signed_duration_since(s).num_milliseconds().max(0) as u64);
        self.finalize_job_run_record(run_id, JobRunState::Cancelled, now, duration_ms)?;
        self.record_event(OrbitEvent::JobRunCancelled {
            job_id: run.job_id,
            run_id: run_id.to_string(),
        })?;
        if let Some(pid) = run.pid {
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
        let job_id = self.archive_job_run_record(run_id)?;
        self.record_event(OrbitEvent::JobRunArchived {
            job_id,
            run_id: run_id.to_string(),
        })
    }

    pub fn delete_job_run(&self, run_id: &str) -> Result<(), OrbitError> {
        if let Some(run) = self.get_job_run_backend(run_id)?
            && matches!(run.state, JobRunState::Pending | JobRunState::Running)
        {
            return Err(OrbitError::JobValidation(format!(
                "job run '{}' is active and cannot be deleted",
                run_id
            )));
        }
        let job_id = self.delete_job_run_record(run_id)?;
        self.record_event(OrbitEvent::JobRunDeleted {
            job_id,
            run_id: run_id.to_string(),
        })
    }

    pub fn job_history(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        let job = self.show_job(job_id)?;
        let _ = self.recover_stale_active_run_for_job(&job, Utc::now())?;
        self.list_job_history_backend(job_id)
    }

    pub fn list_job_runs(&self, params: JobRunListParams) -> Result<Vec<JobRun>, OrbitError> {
        let now = Utc::now();
        if let Some(job_id) = params.job_id.as_deref() {
            let job = self.show_job(job_id)?;
            let _ = self.recover_stale_active_run_for_job(&job, now)?;
        } else {
            for job in self.list_jobs(true)? {
                let _ = self.recover_stale_active_run_for_job(&job, now)?;
            }
        }

        self.list_job_runs_filtered_backend(&JobRunQuery {
            job_id: params.job_id,
            state: params.state,
            created_since: params.since,
            limit: params.limit,
        })
    }

    pub fn show_job_run(&self, run_id: &str) -> Result<JobRun, OrbitError> {
        let run = self
            .get_job_run_backend(run_id)?
            .ok_or_else(|| OrbitError::JobRunNotFound(run_id.to_string()))?;

        if matches!(run.state, JobRunState::Pending | JobRunState::Running)
            && let Ok(job) = self.show_job(&run.job_id)
        {
            let _ = self.recover_stale_active_run_for_job(&job, Utc::now())?;
            return self
                .get_job_run_backend(run_id)?
                .ok_or_else(|| OrbitError::JobRunNotFound(run_id.to_string()));
        }

        Ok(run)
    }

    fn list_job_history_backend(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.list_job_run_records(job_id)
    }

    fn list_job_runs_filtered_backend(
        &self,
        query: &JobRunQuery,
    ) -> Result<Vec<JobRun>, OrbitError> {
        self.list_job_runs_filtered_record(query)
    }

    fn get_job_run_backend(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.get_job_run_record(run_id)
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
