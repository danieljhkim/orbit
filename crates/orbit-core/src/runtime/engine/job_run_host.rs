use chrono::{DateTime, Utc};
use orbit_common::types::{JobRun, JobRunState, NotFoundKind, OrbitError};
use orbit_engine::JobRunHost;
use orbit_store::{JobRunStepParams, TaskReservationReleaseReason};

use crate::OrbitRuntime;

impl JobRunHost for OrbitRuntime {
    fn insert_job_run(
        &self,
        job_id: &str,
        attempt: u32,
        scheduled_at: DateTime<Utc>,
        input: Option<serde_json::Value>,
        retry_source_run_id: Option<String>,
    ) -> Result<JobRun, OrbitError> {
        self.stores().jobs().insert_job_run(
            job_id,
            attempt,
            scheduled_at,
            input,
            retry_source_run_id,
        )
    }

    fn mark_job_run_running(
        &self,
        run_id: &str,
        started_at: DateTime<Utc>,
        pid: u32,
    ) -> Result<bool, OrbitError> {
        self.stores()
            .jobs()
            .mark_job_run_running(run_id, started_at, pid)
    }

    fn complete_job_run_step(
        &self,
        run_id: &str,
        params: &JobRunStepParams,
    ) -> Result<bool, OrbitError> {
        self.stores().jobs().complete_job_run_step(run_id, params)
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
            Err(OrbitError::NotFound {
                kind: NotFoundKind::JobRun,
                ..
            }) => Ok(None),
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
