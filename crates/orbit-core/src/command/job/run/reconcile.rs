//! Stale-run reconciliation, terminal timing repair, and audit helpers.

use chrono::{DateTime, Utc};
use orbit_common::types::{JobRun, JobRunState, OrbitError, OrbitEvent};
use orbit_store::TaskReservationReleaseReason;
use serde_json::Value;
use std::path::Path;

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
            let raw = std::fs::read_to_string(&path).map_err(|error| {
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

fn terminal_run_timing_is_incomplete(run: &JobRun) -> bool {
    run.state.is_terminal()
        && (run.finished_at.is_none() || (run.duration_ms.is_none() && run.started_at.is_some()))
}
