//! Cancellation, archive, delete, and run-state helpers for job runs.

use chrono::Utc;
use orbit_common::{NotFoundKind, OrbitError};
use orbit_store::contracts::TaskReservationReleaseReason;
use orbit_types::record::OrbitEvent;
use orbit_types::workflow::{
    ChildCancellation, ChildCancellationPolicy, JobRun, JobRunState, PipelineState,
};
use serde_json::Value;

use crate::OrbitRuntime;

use super::owner::signal_run_owner_process;
use super::types::JobRunCancelResult;

/// `source` recorded on a cancellation a parent propagated to its child.
const CHILD_CASCADE_SOURCE: &str = "parent_cascade";
/// Backstop against a cycle in persisted dispatch records.
const MAX_CHILD_CANCEL_CASCADE_DEPTH: usize = 8;

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
        self.cancel_job_run_with_signaller(run_id, actor, source, signal_run_owner_process)
    }

    /// Internal cancellation seam so tests can model a failed post-signal
    /// liveness check without signalling a process they do not own.
    pub(super) fn cancel_job_run_with_signaller<F>(
        &self,
        run_id: &str,
        actor: &str,
        source: &str,
        signal: F,
    ) -> Result<JobRunCancelResult, OrbitError>
    where
        F: FnOnce(&JobRun) -> Result<String, OrbitError>,
    {
        self.cancel_job_run_cascading(run_id, actor, source, signal, 0)
    }

    fn cancel_job_run_cascading<F>(
        &self,
        run_id: &str,
        actor: &str,
        source: &str,
        signal: F,
        depth: usize,
    ) -> Result<JobRunCancelResult, OrbitError>
    where
        F: FnOnce(&JobRun) -> Result<String, OrbitError>,
    {
        let run = self
            .get_job_run_backend(run_id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::JobRun, run_id.to_string()))?;
        run.state
            .try_transition(orbit_types::workflow::RunEvent::Cancel)
            .map_err(|msg| {
                OrbitError::JobValidation(format!("cannot cancel job run '{}': {}", run_id, msg))
            })?;
        let signal_attempted = run.state == JobRunState::Running && run.pid.is_some();
        let signal_outcome = if signal_attempted {
            Some(signal(&run)?)
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
                .try_transition(orbit_types::workflow::RunEvent::Cancel)
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
        self.settle_child_dispatches_on_cancel(&cancelled_run, actor, depth)?;
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
        let job_id = self.stores().jobs().archive_job_run(run_id)?;
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
        let job_id = self.stores().jobs().delete_job_run(run_id)?;
        self.record_event(OrbitEvent::JobRunDeleted {
            job_id,
            run_id: run_id.to_string(),
        })
    }

    pub fn read_run_state(
        &self,
        run_id: &str,
    ) -> Result<Option<orbit_types::workflow::PipelineState>, OrbitError> {
        self.stores().jobs().read_run_state(run_id)
    }

    /// Persist a run's pipeline state. The write side of [`Self::read_run_state`],
    /// for callers outside the store layer that own a read-modify-write of the
    /// run's own state — dispatch checkpoints, waiting reasons, cancellation.
    pub fn write_run_state(&self, run_id: &str, state: &PipelineState) -> Result<(), OrbitError> {
        self.stores().jobs().write_run_state(run_id, state)
    }

    /// Apply this run's child-cancellation policy and close every dispatch it
    /// still holds open [ORB-10971].
    ///
    /// **The policy.** A child the parent was *blocking* on cascades: the
    /// parent's wait was that child's only consumer, so leaving it running
    /// would produce work nobody joins and no operator expects. A child
    /// dispatched *detached* does not: it was submitted precisely to outlive
    /// the parent's step (`workspace_auto_pipeline` starts an epic that way),
    /// and its own drain re-observes it. The dispatch record carries which
    /// shape it was, so the rule is decided by how the child was dispatched
    /// rather than by whoever happens to be cancelling.
    ///
    /// Either way the linkage survives and the wait step stops rendering as
    /// `running`: a cancelled parent must still name the child it left behind,
    /// which is the only handle on that work.
    ///
    /// Cascading is best effort. A child that already terminalized, or that
    /// refuses cancellation, is recorded as such and never blocks the parent's
    /// own cancellation from completing.
    fn settle_child_dispatches_on_cancel(
        &self,
        run: &JobRun,
        actor: &str,
        depth: usize,
    ) -> Result<(), OrbitError> {
        let Some(state) = self.read_run_state(&run.run_id)? else {
            return Ok(());
        };
        let open: Vec<(String, ChildCancellationPolicy)> = state
            .open_child_dispatches()
            .map(|dispatch| {
                (
                    dispatch.child_run_id.clone(),
                    dispatch.cancellation_policy(),
                )
            })
            .collect();
        if open.is_empty() {
            return Ok(());
        }

        let settled: Vec<(String, ChildCancellation)> = open
            .into_iter()
            .map(|(child_run_id, policy)| {
                let (outcome, error) = match policy {
                    ChildCancellationPolicy::Detach => ("detached".to_string(), None),
                    ChildCancellationPolicy::Cascade => {
                        self.cascade_cancel_child(&child_run_id, actor, depth)
                    }
                };
                (
                    child_run_id,
                    ChildCancellation {
                        policy,
                        outcome,
                        error,
                        at: Utc::now(),
                    },
                )
            })
            .collect();

        // Re-read: cascading a child re-enters cancellation, and the parent's
        // own state must be stamped from whatever that left behind.
        let Some(mut state) = self.read_run_state(&run.run_id)? else {
            return Ok(());
        };
        for (child_run_id, cancellation) in settled {
            state.terminalize_child_dispatch(&child_run_id, cancellation);
        }
        self.write_run_state(&run.run_id, &state)
    }

    /// Cancel one blocking child, reporting what happened rather than failing
    /// the parent's cancellation.
    fn cascade_cancel_child(
        &self,
        child_run_id: &str,
        actor: &str,
        depth: usize,
    ) -> (String, Option<String>) {
        // A dispatch graph is a DAG in practice, so this bound is a backstop
        // against a corrupted state file, not an expected limit.
        if depth >= MAX_CHILD_CANCEL_CASCADE_DEPTH {
            return (
                "skipped".to_string(),
                Some(format!(
                    "child cancellation cascade stopped at depth {depth}"
                )),
            );
        }
        match self.cancel_job_run_cascading(
            child_run_id,
            actor,
            CHILD_CASCADE_SOURCE,
            signal_run_owner_process,
            depth + 1,
        ) {
            Ok(_) => ("cancelled".to_string(), None),
            // A child that reached a terminal state on its own is the ordinary
            // race, not a fault: the parent simply lost it to the finish line.
            Err(OrbitError::JobValidation(message))
            | Err(OrbitError::JobRunStateTransition(message)) => {
                ("already_terminal".to_string(), Some(message))
            }
            Err(error) => ("failed".to_string(), Some(error.to_string())),
        }
    }

    /// Cancelling a run preserves its child lineage. `clear_waiting_reasons`
    /// deliberately does not touch `child_dispatches`: dependency and lock
    /// waits are momentary and meaningless once terminal, but the child a
    /// parent dispatched outlives the parent's own record of waiting for it.
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
            self.write_run_state(&run.run_id, &state)?;
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
            self.write_run_state(&run.run_id, &state)?;
        }
        Ok(())
    }
}
