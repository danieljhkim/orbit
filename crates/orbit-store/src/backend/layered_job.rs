use std::sync::Arc;

use chrono::{DateTime, Utc};
use orbit_types::{
    Job, JobRun, JobRunState, JobScheduleState, KnowledgeRunMetrics, OrbitError, PipelineState,
};

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
    pub fn new(workspace: Arc<dyn JobStoreBackend>, global: Arc<dyn JobStoreBackend>) -> Self {
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
        self.workspace.list_job_runs(job_id)
    }

    fn list_job_runs_filtered(&self, query: &JobRunQuery) -> Result<Vec<JobRun>, OrbitError> {
        self.workspace.list_job_runs_filtered(query)
    }

    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        self.workspace.get_job_run(run_id)
    }

    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.workspace.list_pending_or_running_job_runs(job_id)
    }

    fn list_all_pending_or_running_runs(&self) -> Result<Vec<JobRun>, OrbitError> {
        self.workspace.list_all_pending_or_running_runs()
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
        input: Option<serde_json::Value>,
        retry_source_run_id: Option<String>,
    ) -> Result<JobRun, OrbitError> {
        // Runs are always created in the workspace store, regardless of
        // where the job definition lives.
        self.workspace
            .insert_job_run(job_id, attempt, scheduled_at, input, retry_source_run_id)
    }

    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.workspace.mark_job_run_running(run_id, started_at, pid)
    }

    fn take_over_running_job_run(
        &self,
        run_id: &str,
        expected_pid: Option<u32>,
        expected_pid_start_time: Option<String>,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.workspace.take_over_running_job_run(
            run_id,
            expected_pid,
            expected_pid_start_time,
            started_at,
            pid,
        )
    }

    fn abandon_job_run(
        &self,
        run_id: &str,
        finished_at: DateTime<Utc>,
    ) -> Result<bool, OrbitError> {
        self.workspace.abandon_job_run(run_id, finished_at)
    }

    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.workspace.complete_job_run_step(run_id, params)
    }

    fn record_job_run_knowledge_metrics(
        &self,
        run_id: &str,
        metrics: KnowledgeRunMetrics,
    ) -> Result<bool, OrbitError> {
        self.workspace
            .record_job_run_knowledge_metrics(run_id, metrics)
    }

    fn finalize_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.workspace
            .finalize_job_run(run_id, state, finished_at, duration_ms)
    }

    fn archive_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.workspace.archive_job_run(run_id)
    }

    fn delete_job_run(&self, run_id: &str) -> Result<String, OrbitError> {
        self.workspace.delete_job_run(run_id)
    }

    fn read_run_state(&self, run_id: &str) -> Result<Option<PipelineState>, OrbitError> {
        self.workspace.read_run_state(run_id)
    }

    fn write_run_state(&self, run_id: &str, state: &PipelineState) -> Result<(), OrbitError> {
        self.workspace.write_run_state(run_id, state)
    }
}
