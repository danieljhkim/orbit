use std::path::{Path, PathBuf};

use orbit_common::types::{JobRun, JobRunState, OrbitError};

use crate::file::layout::validate_path_stem;
use crate::file::sort::sort_by_created_desc_id_asc;

pub(crate) struct JobFileStore {
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
            runs_root: orbit_root.join("state").join("job-runs"),
        }
    }

    pub(crate) fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        validate_path_stem(job_id, "job")?;
        let mut runs = self.read_runs_for_activity(job_id)?;
        sort_by_created_desc_id_asc(&mut runs, |run| &run.created_at, |run| &run.run_id);
        Ok(runs)
    }

    pub(crate) fn list_job_runs_filtered(
        &self,
        query: &crate::backend::JobRunQuery,
    ) -> Result<Vec<JobRun>, OrbitError> {
        let mut runs = if let Some(job_id) = query.job_id.as_deref() {
            validate_path_stem(job_id, "job")?;
            self.read_runs_for_activity(job_id)?
        } else {
            self.read_all_runs()?
        };

        if let Some(state) = query.state {
            runs.retain(|run| run.state == state);
        }
        if query.terminal_only {
            runs.retain(|run| run.state.is_terminal());
        }
        if let Some(created_since) = query.created_since {
            runs.retain(|run| run.created_at >= created_since);
        }

        sort_by_created_desc_id_asc(&mut runs, |run| &run.created_at, |run| &run.run_id);

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
        validate_path_stem(job_id, "job")?;
        let mut runs = self
            .read_runs_for_activity(job_id)?
            .into_iter()
            .filter(|run| run.state == JobRunState::Pending || run.state == JobRunState::Running)
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| std::cmp::Reverse(run.created_at));
        Ok(runs)
    }

    pub(crate) fn list_all_pending_or_running_runs(&self) -> Result<Vec<JobRun>, OrbitError> {
        let mut runs = self
            .read_all_runs()?
            .into_iter()
            .filter(|run| run.state == JobRunState::Pending || run.state == JobRunState::Running)
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| std::cmp::Reverse(run.created_at));
        Ok(runs)
    }
}
