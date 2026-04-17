use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_types::{Job, JobRun, JobRunState, JobScheduleState, OrbitError};

use super::resource::{job_to_resource, validate_max_active_runs};
use crate::backend::{JobCreateParams, JobUpdateParams};
use crate::file::fs_utils::write_atomic;

pub(crate) struct JobFileStore {
    pub(super) jobs_root: PathBuf,
    pub(super) runs_root: PathBuf,
}

impl JobFileStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        let orbit_root = root
            .parent()
            .and_then(Path::parent)
            .unwrap_or(root.as_path())
            .to_path_buf();
        Self {
            jobs_root: root,
            runs_root: orbit_root.join("state").join("job-runs"),
        }
    }

    pub(crate) fn add_job(&self, params: JobCreateParams) -> Result<Job, OrbitError> {
        self.ensure_layout()?;
        validate_max_active_runs(params.max_active_runs)?;
        let resolved_id = match params.job_id {
            Some(id) => {
                if self.job_path(&id).exists() {
                    return Err(OrbitError::JobValidation(format!(
                        "job id already exists: {id}"
                    )));
                }
                id
            }
            None => self.next_job_id(),
        };
        let now = Utc::now();
        let job = Job {
            job_id: resolved_id,
            state: params.initial_state,
            default_input: params.default_input,
            max_active_runs: params.max_active_runs,
            max_iterations: params.max_iterations,
            steps: params.steps,
            policy: params.policy,
            created_at: now,
            updated_at: now,
        };
        self.write_activity(&job)?;
        Ok(job)
    }

    pub(crate) fn list_jobs(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        let mut jobs = self.read_all_activities()?;
        if !include_disabled {
            jobs.retain(|job| job.state != JobScheduleState::Disabled);
        }
        jobs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.job_id.cmp(&b.job_id))
        });
        Ok(jobs)
    }

    pub(crate) fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        let path = self.job_path(job_id);
        if path.exists() {
            return Ok(Some(self.read_activity_at(&path)?));
        }
        let disabled_path = self.disabled_job_path(job_id);
        if disabled_path.exists() {
            return Ok(Some(self.read_activity_at(&disabled_path)?));
        }
        Ok(None)
    }

    pub(crate) fn update_job(
        &self,
        job_id: &str,
        params: &JobUpdateParams,
    ) -> Result<Job, OrbitError> {
        self.ensure_layout()?;
        let Some(mut job) = self.get_job(job_id)? else {
            return Err(OrbitError::JobNotFound(job_id.to_string()));
        };

        if let Some(default_input) = params.default_input.clone() {
            job.default_input = default_input;
        }
        if let Some(max_active_runs) = params.max_active_runs {
            validate_max_active_runs(max_active_runs)?;
            job.max_active_runs = max_active_runs;
        }
        if let Some(max_iterations) = params.max_iterations {
            job.max_iterations = max_iterations;
        }
        if let Some(steps) = params.steps.clone() {
            job.steps = steps;
        }
        if let Some(policy) = params.policy.clone() {
            job.policy = policy;
        }
        if let Some(state) = params.state {
            job.state = state;
        }
        job.updated_at = Utc::now();

        self.write_activity(&job)?;
        let disabled_path = self.disabled_job_path(job_id);
        let active_path = self.job_path(job_id);
        match job.state {
            JobScheduleState::Enabled => {
                if disabled_path.exists() {
                    fs::remove_file(&disabled_path).map_err(|e| OrbitError::Io(e.to_string()))?;
                }
            }
            JobScheduleState::Disabled => {
                if active_path.exists() {
                    fs::remove_file(&active_path).map_err(|e| OrbitError::Io(e.to_string()))?;
                }
            }
        }

        Ok(job)
    }

    pub(crate) fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        let mut runs = self.read_runs_for_activity(job_id)?;
        runs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.run_id.cmp(&b.run_id))
        });
        Ok(runs)
    }

    pub(crate) fn list_job_runs_filtered(
        &self,
        query: &crate::backend::JobRunQuery,
    ) -> Result<Vec<JobRun>, OrbitError> {
        let mut runs = if let Some(job_id) = query.job_id.as_deref() {
            self.read_runs_for_activity(job_id)?
        } else {
            self.read_all_runs()?
        };

        if let Some(state) = query.state {
            runs.retain(|run| run.state == state);
        }
        if let Some(created_since) = query.created_since {
            runs.retain(|run| run.created_at >= created_since);
        }

        runs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.run_id.cmp(&b.run_id))
        });

        if let Some(limit) = query.limit {
            runs.truncate(limit);
        }

        Ok(runs)
    }

    pub(crate) fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        let Some((_job_id, run_dir)) = self.find_run_path(run_id)? else {
            return Ok(None);
        };
        Ok(Some(self.read_run_at(&run_dir)?))
    }

    pub(crate) fn list_pending_or_running_job_runs(
        &self,
        job_id: &str,
    ) -> Result<Vec<JobRun>, OrbitError> {
        let mut runs = self
            .read_runs_for_activity(job_id)?
            .into_iter()
            .filter(|run| run.state == JobRunState::Pending || run.state == JobRunState::Running)
            .collect::<Vec<_>>();
        runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(runs)
    }

    pub(crate) fn list_all_pending_or_running_runs(&self) -> Result<Vec<JobRun>, OrbitError> {
        let mut runs = self
            .read_all_runs()?
            .into_iter()
            .filter(|run| run.state == JobRunState::Pending || run.state == JobRunState::Running)
            .collect::<Vec<_>>();
        runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(runs)
    }

    pub(crate) fn set_job_state(
        &self,
        job_id: &str,
        state: JobScheduleState,
    ) -> Result<bool, OrbitError> {
        let Some(mut job) = self.get_job(job_id)? else {
            return Ok(false);
        };
        if state == JobScheduleState::Disabled {
            return self.mark_job_disabled(job_id);
        }
        job.state = state;
        job.updated_at = Utc::now();
        self.write_activity(&job)?;
        // If the job was previously in disabled/, remove that stale copy.
        let disabled_path = self.disabled_job_path(job_id);
        if disabled_path.exists() {
            fs::remove_file(&disabled_path).map_err(|e| OrbitError::Io(e.to_string()))?;
        }
        Ok(true)
    }

    pub(crate) fn mark_job_disabled(&self, job_id: &str) -> Result<bool, OrbitError> {
        let Some(mut job) = self.get_job(job_id)? else {
            return Ok(false);
        };
        // If already in disabled/, nothing to move.
        let disabled_path = self.disabled_job_path(job_id);
        if disabled_path.exists() {
            return Ok(true);
        }
        job.state = JobScheduleState::Disabled;
        job.updated_at = Utc::now();
        // Write updated state to disabled/ then remove the active file.
        let content = serde_yaml::to_string(&job_to_resource(&job))
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        write_atomic(&disabled_path, &content)?;
        let active_path = self.job_path(job_id);
        if active_path.exists() {
            fs::remove_file(&active_path).map_err(|e| OrbitError::Io(e.to_string()))?;
        }
        Ok(true)
    }
}
