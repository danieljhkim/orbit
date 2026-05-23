//! Stale-run reconciliation, terminal timing repair, and audit helpers.

use chrono::{DateTime, Utc};
use orbit_common::types::{JobRun, JobRunState, OrbitError, OrbitEvent};
use orbit_store::TaskReservationReleaseReason;

use crate::OrbitRuntime;

use super::owner::{
    running_run_owner_is_stale, running_run_owner_stale_reason, stale_job_run_message,
};

impl OrbitRuntime {
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
        let stale_reason = running_run_owner_stale_reason(run);
        let _ = self.record_pipeline_failure_step(
            run,
            step_started_at,
            finished_at,
            &stale_job_run_message(run, stale_reason),
        );
        self.record_event(OrbitEvent::JobRunCompleted {
            job_id: run.job_id.clone(),
            run_id: run.run_id.clone(),
            state: JobRunState::Failed.to_string(),
        })?;
        Ok(true)
    }

    pub(super) fn reconcile_job_run_records(&self, runs: &[JobRun]) -> Result<usize, OrbitError> {
        let mut reconciled = 0usize;
        for run in runs {
            if self.reconcile_stale_job_run(run)? {
                reconciled += 1;
            }
        }
        Ok(reconciled)
    }

    pub(super) fn list_reconciled_job_history_backend(
        &self,
        job_id: &str,
    ) -> Result<Vec<JobRun>, OrbitError> {
        let runs = self.list_job_history_backend(job_id)?;
        if self.reconcile_job_run_records(&runs)? > 0 {
            self.list_job_history_backend(job_id)
        } else {
            Ok(runs)
        }
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
        for event in self.collect_run_audit_events(run_id)? {
            if matches!(event.event_type.as_deref(), Some("run.finished"))
                || matches!(event.body_kind.as_deref(), Some("run_finished"))
            {
                return Ok(event.timestamp);
            }
        }
        Ok(None)
    }
}

fn terminal_run_timing_is_incomplete(run: &JobRun) -> bool {
    run.state.is_terminal()
        && (run.finished_at.is_none() || (run.duration_ms.is_none() && run.started_at.is_some()))
}
