use std::sync::Arc;

use chrono::{DateTime, Utc};
use orbit_types::{Job, JobRun, JobRunState, JobScheduleState, OrbitError};

use super::contracts::{
    JobCreateParams, JobRunQuery, JobRunStepParams, JobStoreBackend, JobUpdateParams,
};

/// A layered job store that merges a workspace store with a global store.
///
/// Read semantics: workspace entries shadow global entries by job ID.
/// Write semantics: writes go to workspace store if present, otherwise global.
/// Mutations target whichever store owns the entry.
pub struct LayeredJobStore {
    workspace: Arc<dyn JobStoreBackend>,
    global: Arc<dyn JobStoreBackend>,
}

impl LayeredJobStore {
    pub fn new(
        workspace: Arc<dyn JobStoreBackend>,
        global: Arc<dyn JobStoreBackend>,
    ) -> Self {
        Self { workspace, global }
    }

    /// Returns (store_that_owns_it, is_workspace) for mutation routing.
    fn owning_store(&self, job_id: &str) -> Result<&dyn JobStoreBackend, OrbitError> {
        if self.workspace.get_job(job_id)?.is_some() {
            Ok(self.workspace.as_ref())
        } else {
            Ok(self.global.as_ref())
        }
    }

    /// Returns the store that owns a given run_id.
    fn owning_store_for_run(&self, run_id: &str) -> Result<&dyn JobStoreBackend, OrbitError> {
        if self.workspace.get_job_run(run_id)?.is_some() {
            Ok(self.workspace.as_ref())
        } else {
            Ok(self.global.as_ref())
        }
    }
}

impl JobStoreBackend for LayeredJobStore {
    fn add_job(&self, params: JobCreateParams) -> Result<Job, OrbitError> {
        self.workspace.add_job(params)
    }

    fn update_job(&self, job_id: &str, params: JobUpdateParams) -> Result<Job, OrbitError> {
        self.owning_store(job_id)?.update_job(job_id, params)
    }

    fn list_jobs(&self, include_disabled: bool) -> Result<Vec<Job>, OrbitError> {
        let workspace_jobs = self.workspace.list_jobs(include_disabled)?;
        let global_jobs = self.global.list_jobs(include_disabled)?;

        let workspace_ids: std::collections::HashSet<String> =
            workspace_jobs.iter().map(|j| j.job_id.clone()).collect();

        let mut merged = workspace_jobs;
        for job in global_jobs {
            if !workspace_ids.contains(&job.job_id) {
                merged.push(job);
            }
        }
        merged.sort_by(|a, b| a.job_id.cmp(&b.job_id));
        Ok(merged)
    }

    fn get_job(&self, job_id: &str) -> Result<Option<Job>, OrbitError> {
        if let Some(job) = self.workspace.get_job(job_id)? {
            return Ok(Some(job));
        }
        self.global.get_job(job_id)
    }

    fn list_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.owning_store(job_id)?.list_job_runs(job_id)
    }

    fn list_job_runs_filtered(&self, query: &JobRunQuery) -> Result<Vec<JobRun>, OrbitError> {
        let ws_runs = self.workspace.list_job_runs_filtered(query)?;
        let global_runs = self.global.list_job_runs_filtered(query)?;

        let ws_ids: std::collections::HashSet<String> =
            ws_runs.iter().map(|r| r.run_id.clone()).collect();

        let mut merged = ws_runs;
        for run in global_runs {
            if !ws_ids.contains(&run.run_id) {
                merged.push(run);
            }
        }
        Ok(merged)
    }

    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        if let Some(run) = self.workspace.get_job_run(run_id)? {
            return Ok(Some(run));
        }
        self.global.get_job_run(run_id)
    }

    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.owning_store(job_id)?
            .list_pending_or_running_job_runs(job_id)
    }

    fn set_job_state(&self, job_id: &str, state: JobScheduleState) -> Result<bool, OrbitError> {
        self.owning_store(job_id)?.set_job_state(job_id, state)
    }

    fn mark_job_disabled(&self, job_id: &str) -> Result<bool, OrbitError> {
        self.owning_store(job_id)?.mark_job_disabled(job_id)
    }

    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
    ) -> Result<JobRun, OrbitError> {
        self.owning_store(job_id)?
            .insert_job_run(job_id, attempt, scheduled_at)
    }

    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.owning_store_for_run(run_id)?
            .mark_job_run_running(run_id, started_at, pid)
    }

    fn abandon_job_run(
        &self,
        run_id: &str,
        finished_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.owning_store_for_run(run_id)?
            .abandon_job_run(run_id, finished_at)
    }

    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.owning_store_for_run(run_id)?
            .complete_job_run_step(run_id, params)
    }

    fn finalize_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.owning_store_for_run(run_id)?
            .finalize_job_run(run_id, state, finished_at, duration_ms)
    }

    fn archive_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.owning_store_for_run(run_id)?.archive_job_run(run_id)
    }

    fn delete_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.owning_store_for_run(run_id)?.delete_job_run(run_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::memory_job::MemoryJobStoreBackend;
    use orbit_types::JobStep;

    fn make_params(id: &str) -> JobCreateParams {
        JobCreateParams {
            job_id: Some(id.to_string()),
            default_input: None,
            max_active_runs: 1,
            steps: vec![JobStep {
                target_type: orbit_types::JobTargetType::Activity,
                target_id: "test-activity".to_string(),
                ..Default::default()
            }],
            initial_state: JobScheduleState::Enabled,
        }
    }

    fn make_layered() -> (
        Arc<MemoryJobStoreBackend>,
        Arc<MemoryJobStoreBackend>,
        LayeredJobStore,
    ) {
        let ws = Arc::new(MemoryJobStoreBackend::default());
        let global = Arc::new(MemoryJobStoreBackend::default());
        let layered = LayeredJobStore::new(ws.clone(), global.clone());
        (ws, global, layered)
    }

    #[test]
    fn workspace_shadows_global_by_job_id() {
        let (ws, global, layered) = make_layered();
        global.add_job(make_params("shared")).unwrap();
        ws.add_job(make_params("shared")).unwrap();

        let jobs = layered.list_jobs(true).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, "shared");
    }

    #[test]
    fn global_only_jobs_visible() {
        let (_ws, global, layered) = make_layered();
        global.add_job(make_params("global-only")).unwrap();

        assert!(layered.get_job("global-only").unwrap().is_some());
        let jobs = layered.list_jobs(true).unwrap();
        assert_eq!(jobs.len(), 1);
    }

    #[test]
    fn merge_returns_union() {
        let (ws, global, layered) = make_layered();
        global.add_job(make_params("g1")).unwrap();
        global.add_job(make_params("g2")).unwrap();
        ws.add_job(make_params("w1")).unwrap();
        ws.add_job(make_params("g2")).unwrap(); // shadows global g2

        let jobs = layered.list_jobs(true).unwrap();
        assert_eq!(jobs.len(), 3);
    }

    #[test]
    fn writes_go_to_workspace() {
        let (ws, global, layered) = make_layered();
        layered.add_job(make_params("new")).unwrap();

        assert!(ws.get_job("new").unwrap().is_some());
        assert!(global.get_job("new").unwrap().is_none());
    }

    #[test]
    fn update_targets_owning_store() {
        let (_ws, global, layered) = make_layered();
        global.add_job(make_params("gj")).unwrap();

        let update = JobUpdateParams {
            max_active_runs: Some(5),
            ..Default::default()
        };
        let updated = layered.update_job("gj", update).unwrap();
        assert_eq!(updated.max_active_runs, 5);
    }
}
