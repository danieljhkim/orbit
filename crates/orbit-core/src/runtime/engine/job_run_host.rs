use chrono::{DateTime, Utc};
use orbit_common::types::{JobRun, JobRunState, KnowledgeRunMetrics, OrbitError};
use orbit_engine::JobRunHost;
use orbit_store::{JobRunStepParams, TaskReservationReleaseReason};

use crate::OrbitRuntime;

impl JobRunHost for OrbitRuntime {
    fn list_all_pending_or_running_runs(&self) -> Result<Vec<JobRun>, OrbitError> {
        self.reconcile_stale_job_runs(None)?;
        self.stores().jobs().list_all_pending_or_running()
    }

    fn list_pending_or_running_job_runs(&self, job_id: &str) -> Result<Vec<JobRun>, OrbitError> {
        self.reconcile_stale_job_runs(Some(job_id))?;
        self.stores().jobs().list_pending_or_running(job_id)
    }

    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
        input: Option<serde_json::Value>,
        retry_source_run_id: Option<String>,
    ) -> Result<JobRun, OrbitError> {
        self.stores()
            .jobs()
            .insert_run(job_id, attempt, scheduled_at, input, retry_source_run_id)
    }

    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.stores()
            .jobs()
            .mark_run_running(run_id, started_at, pid)
    }

    fn take_over_running_job_run(
        &self,
        run_id: &str,
        expected_pid: Option<u32>,
        expected_pid_start_time: Option<String>,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.stores().jobs().take_over_running_run(
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
        self.stores().jobs().abandon_run(run_id, finished_at)
    }

    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.stores().jobs().complete_run_step(run_id, params)
    }

    fn record_job_run_knowledge_metrics(
        &self,
        run_id: &str,
        metrics: KnowledgeRunMetrics,
    ) -> Result<bool, OrbitError> {
        self.stores()
            .jobs()
            .record_run_knowledge_metrics(run_id, metrics)
    }

    fn finalize_job_run(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
    ) -> Result<bool, OrbitError> {
        self.finalize_job_run_with_reservation_cleanup(
            run_id,
            state,
            finished_at,
            duration_ms,
            TaskReservationReleaseReason::RunTerminal,
        )
    }

    fn get_job_run(&self, run_id: &str) -> Result<Option<JobRun>, OrbitError> {
        match self.show_job_run(run_id) {
            Ok(run) => Ok(Some(run)),
            Err(OrbitError::JobRunNotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn read_run_state(
        &self,
        run_id: &str,
    ) -> Result<Option<orbit_common::types::PipelineState>, OrbitError> {
        self.stores().jobs().read_run_state(run_id)
    }

    fn write_run_state(
        &self,
        run_id: &str,
        state: &orbit_common::types::PipelineState,
    ) -> Result<(), OrbitError> {
        self.stores().jobs().write_run_state(run_id, state)
    }
}
