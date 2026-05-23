//! Cancellation, archive, delete, and run-state helpers for job runs.

use chrono::Utc;
use orbit_common::types::{
    JobRun, JobRunState, NotFoundKind, OrbitError, OrbitEvent, PipelineState,
};
use orbit_store::TaskReservationReleaseReason;
use serde_json::Value;

use crate::OrbitRuntime;

use super::owner::signal_run_owner_process;
use super::types::JobRunCancelResult;

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
}
