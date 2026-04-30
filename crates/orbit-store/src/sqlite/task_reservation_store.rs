use std::collections::BTreeSet;

use chrono::{Duration, Utc};
use orbit_common::types::OrbitError;
use orbit_common::utility::path::workspace_relative_paths_overlap;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::{
    ActiveTaskReservation, ExpiredTaskReservation, Store, TaskLockConflict, TaskLockHolder,
    TaskReservationCheckParams, TaskReservationCheckResult, TaskReservationListResult,
    TaskReservationReleaseParams, TaskReservationReleaseResult, TaskReservationReserveParams,
    TaskReservationReserveResult,
};

impl Store {
    pub fn list_active_task_reservations(
        &self,
        workspace_orbit_dir: &str,
    ) -> Result<TaskReservationListResult, OrbitError> {
        self.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            let now = crate::now_string();
            let expired_reservations = expire_reservations(tx, workspace_orbit_dir, &now)?;
            let mut stmt = tx
                .tx
                .prepare(
                    "SELECT reservation_id, task_ids_json, files_json, actor, created_at, expires_at
                     FROM task_reservations
                     WHERE workspace_orbit_dir = ?1
                       AND released_at IS NULL
                       AND expires_at > ?2
                     ORDER BY created_at ASC, reservation_id ASC",
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            let rows = stmt
                .query_map(params![workspace_orbit_dir, now], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })
                .map_err(|error| OrbitError::Store(error.to_string()))?;

            let mut reservations = Vec::new();
            for row in rows {
                let (reservation_id, task_ids_json, files_json, actor, created_at, expires_at) =
                    row.map_err(|error| OrbitError::Store(error.to_string()))?;
                reservations.push(ActiveTaskReservation {
                    reservation_id,
                    task_ids: parse_string_list(&task_ids_json)?,
                    files: parse_string_list(&files_json)?,
                    actor,
                    created_at,
                    expires_at,
                });
            }

            Ok(TaskReservationListResult {
                reservations,
                expired_reservations,
            })
        })
    }

    pub fn check_task_reservation_conflicts(
        &self,
        params: &TaskReservationCheckParams,
    ) -> Result<TaskReservationCheckResult, OrbitError> {
        self.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            let now = crate::now_string();
            let expired_reservations = expire_reservations(tx, &params.workspace_orbit_dir, &now)?;
            let conflicts = find_reservation_conflicts(
                tx,
                &params.workspace_orbit_dir,
                &now,
                &params.requested_files,
            )?;
            Ok(TaskReservationCheckResult {
                conflicts,
                expired_reservations,
            })
        })
    }

    pub fn reserve_task_reservation(
        &self,
        params: &TaskReservationReserveParams,
    ) -> Result<TaskReservationReserveResult, OrbitError> {
        self.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            let now = crate::now_string();
            let expired_reservations = expire_reservations(tx, &params.workspace_orbit_dir, &now)?;
            let conflicts = find_reservation_conflicts(
                tx,
                &params.workspace_orbit_dir,
                &now,
                &params.requested_files,
            )?;
            if !conflicts.is_empty() {
                return Ok(TaskReservationReserveResult {
                    reserved: false,
                    reservation_id: None,
                    expires_at: None,
                    reserved_files: Vec::new(),
                    conflicts,
                    expired_reservations,
                });
            }

            let reservation_id = format!(
                "reservation-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0)
            );
            let created_at = now;
            let expires_at =
                (Utc::now() + Duration::seconds(params.ttl_seconds as i64)).to_rfc3339();
            let task_ids_json = serialize_string_list(&params.task_ids)?;
            let files_json = serialize_string_list(&params.requested_files)?;

            tx.tx
                .execute(
                    "INSERT INTO task_reservations(
                        reservation_id,
                        workspace_orbit_dir,
                        task_ids_json,
                        files_json,
                        actor,
                        created_at,
                        expires_at,
                        released_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
                    params![
                        reservation_id,
                        params.workspace_orbit_dir,
                        task_ids_json,
                        files_json,
                        params.actor,
                        created_at,
                        expires_at,
                    ],
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;

            Ok(TaskReservationReserveResult {
                reserved: true,
                reservation_id: Some(reservation_id),
                expires_at: Some(expires_at),
                reserved_files: params.requested_files.clone(),
                conflicts: Vec::new(),
                expired_reservations,
            })
        })
    }

    pub fn release_task_reservation(
        &self,
        params: &TaskReservationReleaseParams,
    ) -> Result<TaskReservationReleaseResult, OrbitError> {
        self.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            let now = crate::now_string();
            let mut expired_reservations =
                expire_reservations(tx, &params.workspace_orbit_dir, &now)?;
            let existing_release: Option<Option<String>> = tx
                .tx
                .query_row(
                    "SELECT released_at FROM task_reservations
                     WHERE workspace_orbit_dir = ?1 AND reservation_id = ?2",
                    params![params.workspace_orbit_dir, params.reservation_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| OrbitError::Store(error.to_string()))?;

            let Some(released_at) = existing_release else {
                return Ok(TaskReservationReleaseResult {
                    released: false,
                    released_at: None,
                    expired_reservations,
                });
            };

            if released_at.is_some() {
                return Ok(TaskReservationReleaseResult {
                    released: false,
                    released_at: None,
                    expired_reservations,
                });
            }

            let released_at = crate::now_string();
            let affected = tx
                .tx
                .execute(
                    "UPDATE task_reservations
                     SET released_at = ?1
                     WHERE workspace_orbit_dir = ?2
                       AND reservation_id = ?3
                       AND released_at IS NULL",
                    params![
                        released_at,
                        params.workspace_orbit_dir,
                        params.reservation_id
                    ],
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;

            if affected == 0 {
                expired_reservations.push(ExpiredTaskReservation {
                    reservation_id: params.reservation_id.clone(),
                    expired_at: now,
                });
                return Ok(TaskReservationReleaseResult {
                    released: false,
                    released_at: None,
                    expired_reservations,
                });
            }

            Ok(TaskReservationReleaseResult {
                released: true,
                released_at: Some(released_at),
                expired_reservations,
            })
        })
    }
}

fn expire_reservations(
    tx: &mut crate::StoreTx<'_>,
    workspace_orbit_dir: &str,
    now: &str,
) -> Result<Vec<ExpiredTaskReservation>, OrbitError> {
    let mut stmt = tx
        .tx
        .prepare(
            "SELECT reservation_id, expires_at
             FROM task_reservations
             WHERE workspace_orbit_dir = ?1
               AND released_at IS NULL
               AND expires_at <= ?2
             ORDER BY expires_at ASC, reservation_id ASC",
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = stmt
        .query_map(params![workspace_orbit_dir, now], |row| {
            Ok(ExpiredTaskReservation {
                reservation_id: row.get(0)?,
                expired_at: row.get(1)?,
            })
        })
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let expired_reservations = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| OrbitError::Store(error.to_string()))?;

    if !expired_reservations.is_empty() {
        tx.tx
            .execute(
                "UPDATE task_reservations
                 SET released_at = ?1
                 WHERE workspace_orbit_dir = ?2
                   AND released_at IS NULL
                   AND expires_at <= ?1",
                params![now, workspace_orbit_dir],
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
    }

    Ok(expired_reservations)
}

fn find_reservation_conflicts(
    tx: &mut crate::StoreTx<'_>,
    workspace_orbit_dir: &str,
    now: &str,
    requested_files: &[String],
) -> Result<Vec<TaskLockConflict>, OrbitError> {
    let requested_files: BTreeSet<String> = requested_files.iter().cloned().collect();
    if requested_files.is_empty() {
        return Ok(Vec::new());
    }

    let mut stmt = tx
        .tx
        .prepare(
            "SELECT reservation_id, files_json
             FROM task_reservations
             WHERE workspace_orbit_dir = ?1
               AND released_at IS NULL
               AND expires_at > ?2
             ORDER BY created_at ASC, reservation_id ASC",
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = stmt
        .query_map(params![workspace_orbit_dir, now], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| OrbitError::Store(error.to_string()))?;

    let mut conflicts = Vec::new();
    for row in rows {
        let (reservation_id, files_json) =
            row.map_err(|error| OrbitError::Store(error.to_string()))?;
        let reserved_files = parse_string_list(&files_json)?;
        for requested_file in &requested_files {
            if reserved_files
                .iter()
                .any(|held_file| workspace_relative_paths_overlap(requested_file, held_file))
            {
                conflicts.push(TaskLockConflict {
                    file: requested_file.clone(),
                    held_by: TaskLockHolder::Reservation,
                    held_by_id: reservation_id.clone(),
                });
            }
        }
    }

    conflicts.sort_by(|left, right| {
        left.file
            .cmp(&right.file)
            .then(left.held_by_id.cmp(&right.held_by_id))
    });
    Ok(conflicts)
}

fn parse_string_list(raw: &str) -> Result<Vec<String>, OrbitError> {
    serde_json::from_str(raw)
        .map_err(|error| OrbitError::Store(format!("deserialize reservation string list: {error}")))
}

fn serialize_string_list(values: &[String]) -> Result<String, OrbitError> {
    let unique = values.iter().cloned().collect::<BTreeSet<_>>();
    serde_json::to_string(&unique.into_iter().collect::<Vec<_>>())
        .map_err(|error| OrbitError::Store(format!("serialize reservation string list: {error}")))
}
