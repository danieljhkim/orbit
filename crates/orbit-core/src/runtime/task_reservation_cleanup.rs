use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use orbit_common::types::{JobRunState, OrbitError};
use orbit_store::{
    ReleasedTaskReservation, TaskReservationOwnedConflictsParams,
    TaskReservationReleaseByOwnerParams, TaskReservationReleaseReason,
};
use serde_json::json;

use crate::OrbitRuntime;

use super::orbit_tool_host::{
    emit_expired_reservation_events, emit_task_lock_release_event, workspace_orbit_dir,
    workspace_task_reservation_id,
};

impl OrbitRuntime {
    pub(crate) fn finalize_job_run_with_reservation_cleanup(
        &self,
        run_id: &str,
        state: JobRunState,
        finished_at: DateTime<Utc>,
        duration_ms: Option<u64>,
        release_reason: TaskReservationReleaseReason,
    ) -> Result<bool, OrbitError> {
        let changed = self
            .stores()
            .jobs()
            .finalize_run(run_id, state, finished_at, duration_ms)?;
        if state.is_terminal() {
            self.best_effort_release_task_reservations_for_owner_run_id(run_id, release_reason);
        }
        Ok(changed)
    }

    pub(crate) fn release_task_reservations_for_owner_run_id(
        &self,
        owner_run_id: &str,
        release_reason: TaskReservationReleaseReason,
    ) -> Result<Vec<ReleasedTaskReservation>, OrbitError> {
        let result = self.stores().task_reservations().release_by_owner_run_id(
            TaskReservationReleaseByOwnerParams {
                workspace_orbit_dir: workspace_orbit_dir(self),
                workspace_id: workspace_task_reservation_id(self)?,
                owner_run_id: owner_run_id.to_string(),
                release_reason,
                release_metadata_json: Some(
                    json!({
                        "owner_run_id": owner_run_id,
                        "release_reason": release_reason.as_str(),
                    })
                    .to_string(),
                ),
            },
        )?;
        emit_expired_reservation_events(self, &result.expired_reservations)?;
        for reservation in &result.released_reservations {
            emit_task_lock_release_event(self, reservation, release_reason)?;
        }
        Ok(result.released_reservations)
    }

    pub(crate) fn best_effort_release_task_reservations_for_owner_run_id(
        &self,
        owner_run_id: &str,
        release_reason: TaskReservationReleaseReason,
    ) {
        if let Err(error) =
            self.release_task_reservations_for_owner_run_id(owner_run_id, release_reason)
        {
            tracing::warn!(
                owner_run_id = owner_run_id,
                release_reason = release_reason.as_str(),
                "failed to release task reservations for terminal job run: {error}"
            );
        }
    }

    pub(crate) fn reconcile_stale_owned_reservations_for_files(
        &self,
        requested_files: &[String],
        limit: usize,
    ) -> Result<Vec<ReleasedTaskReservation>, OrbitError> {
        let candidates = self.stores().task_reservations().list_owned_conflicts(
            TaskReservationOwnedConflictsParams {
                workspace_orbit_dir: workspace_orbit_dir(self),
                workspace_id: workspace_task_reservation_id(self)?,
                requested_files: requested_files.to_vec(),
                limit,
            },
        )?;
        emit_expired_reservation_events(self, &candidates.expired_reservations)?;

        let mut released = Vec::new();
        let mut inspected_owner_run_ids = BTreeSet::new();
        for reservation in candidates.reservations {
            let Some(owner_run_id) = reservation.owner_run_id.as_deref() else {
                continue;
            };
            if !inspected_owner_run_ids.insert(owner_run_id.to_string()) {
                continue;
            }

            match self.get_job_run_backend(owner_run_id)? {
                Some(run) if run.state.is_terminal() => {
                    released.extend(self.release_task_reservations_for_owner_run_id(
                        owner_run_id,
                        TaskReservationReleaseReason::StaleRunReconciled,
                    )?);
                }
                Some(run) => {
                    if self.reconcile_stale_job_run(&run)? {
                        released.extend(self.release_task_reservations_for_owner_run_id(
                            owner_run_id,
                            TaskReservationReleaseReason::StaleRunReconciled,
                        )?);
                    }
                }
                None => {
                    released.extend(self.release_task_reservations_for_owner_run_id(
                        owner_run_id,
                        TaskReservationReleaseReason::StaleRunReconciled,
                    )?);
                }
            }
        }
        Ok(released)
    }
}

