use std::collections::BTreeSet;

use chrono::{Duration, Utc};
use orbit_common::types::OrbitError;
use orbit_common::utility::path::workspace_relative_paths_overlap;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::{
    ActiveTaskReservation, ExpiredTaskReservation, ReleasedTaskReservation, Store,
    TaskLockConflict, TaskLockHolder, TaskReservationCheckParams, TaskReservationCheckResult,
    TaskReservationListResult, TaskReservationOwnedConflictsParams,
    TaskReservationOwnedConflictsResult, TaskReservationReleaseByOwnerParams,
    TaskReservationReleaseByOwnerResult, TaskReservationReleaseParams,
    TaskReservationReleaseReason, TaskReservationReleaseResult, TaskReservationReserveParams,
    TaskReservationReserveResult,
};

impl Store {
    pub fn list_active_task_reservations(
        &self,
        workspace_orbit_dir: &str,
        workspace_id: Option<&str>,
    ) -> Result<TaskReservationListResult, OrbitError> {
        self.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            let now = crate::now_string();
            let expired_reservations =
                expire_reservations(tx, workspace_orbit_dir, workspace_id, &now)?;
            let sql = format!(
                "SELECT {}
                 FROM task_reservations
                 WHERE {}
                   AND released_at IS NULL
                   AND expires_at > ?3
                 ORDER BY created_at ASC, reservation_id ASC",
                select_reservation_columns(),
                reservation_scope_clause(),
            );
            let mut stmt = tx
                .tx
                .prepare(&sql)
                .map_err(|error| OrbitError::Store(error.to_string()))?;
            let rows = stmt
                .query_map(
                    params![workspace_id, workspace_orbit_dir, now],
                    reservation_row,
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;

            let mut reservations = Vec::new();
            for row in rows {
                reservations.push(
                    row.map_err(|error| OrbitError::Store(error.to_string()))?
                        .into_active()?,
                );
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
            let expired_reservations = expire_reservations(
                tx,
                &params.workspace_orbit_dir,
                params.workspace_id.as_deref(),
                &now,
            )?;
            let conflicts = find_reservation_conflicts(
                tx,
                &params.workspace_orbit_dir,
                params.workspace_id.as_deref(),
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
            let expired_reservations = expire_reservations(
                tx,
                &params.workspace_orbit_dir,
                params.workspace_id.as_deref(),
                &now,
            )?;
            let conflicts = find_reservation_conflicts(
                tx,
                &params.workspace_orbit_dir,
                params.workspace_id.as_deref(),
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
                        workspace_id,
                        task_ids_json,
                        files_json,
                        actor,
                        created_at,
                        expires_at,
                        released_at,
                        owner_run_id,
                        owner_metadata_json,
                        release_reason,
                        release_metadata_json
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10, NULL, NULL)",
                    params![
                        reservation_id,
                        params.workspace_orbit_dir,
                        params.workspace_id.as_deref(),
                        task_ids_json,
                        files_json,
                        params.actor,
                        created_at,
                        expires_at,
                        params.owner_run_id.as_deref(),
                        params.owner_metadata_json.as_deref(),
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
            let mut expired_reservations = expire_reservations(
                tx,
                &params.workspace_orbit_dir,
                params.workspace_id.as_deref(),
                &now,
            )?;
            let existing = load_reservation_row(
                tx,
                &params.workspace_orbit_dir,
                params.workspace_id.as_deref(),
                &params.reservation_id,
            )?;

            let Some(existing) = existing else {
                return Ok(TaskReservationReleaseResult {
                    released: false,
                    released_at: None,
                    reservation: None,
                    expired_reservations,
                });
            };

            if existing.released_at.is_some() {
                return Ok(TaskReservationReleaseResult {
                    released: false,
                    released_at: None,
                    reservation: None,
                    expired_reservations,
                });
            }

            let released_at = crate::now_string();
            let sql = format!(
                "UPDATE task_reservations
                 SET released_at = ?4,
                     release_reason = ?5,
                     release_metadata_json = ?6
                 WHERE {}
                   AND reservation_id = ?3
                   AND released_at IS NULL",
                reservation_scope_clause(),
            );
            let affected = tx
                .tx
                .execute(
                    &sql,
                    params![
                        params.workspace_id.as_deref(),
                        params.workspace_orbit_dir,
                        params.reservation_id,
                        released_at,
                        params.release_reason.as_str(),
                        params.release_metadata_json.as_deref(),
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
                    reservation: None,
                    expired_reservations,
                });
            }

            Ok(TaskReservationReleaseResult {
                released: true,
                released_at: Some(released_at.clone()),
                reservation: Some(existing.into_released(
                    released_at,
                    params.release_reason,
                    params.release_metadata_json.clone(),
                )?),
                expired_reservations,
            })
        })
    }

    pub fn release_task_reservations_by_owner_run_id(
        &self,
        params: &TaskReservationReleaseByOwnerParams,
    ) -> Result<TaskReservationReleaseByOwnerResult, OrbitError> {
        self.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            let now = crate::now_string();
            let expired_reservations = expire_reservations(
                tx,
                &params.workspace_orbit_dir,
                params.workspace_id.as_deref(),
                &now,
            )?;
            let existing = load_active_reservations_by_owner(
                tx,
                &params.workspace_orbit_dir,
                params.workspace_id.as_deref(),
                &params.owner_run_id,
            )?;
            if existing.is_empty() {
                return Ok(TaskReservationReleaseByOwnerResult {
                    released_reservations: Vec::new(),
                    expired_reservations,
                });
            }

            let released_at = crate::now_string();
            let sql = format!(
                "UPDATE task_reservations
                 SET released_at = ?4,
                     release_reason = ?5,
                     release_metadata_json = ?6
                 WHERE {}
                   AND owner_run_id = ?3
                   AND released_at IS NULL",
                reservation_scope_clause(),
            );
            tx.tx
                .execute(
                    &sql,
                    params![
                        params.workspace_id.as_deref(),
                        params.workspace_orbit_dir,
                        params.owner_run_id,
                        released_at,
                        params.release_reason.as_str(),
                        params.release_metadata_json.as_deref(),
                    ],
                )
                .map_err(|error| OrbitError::Store(error.to_string()))?;

            let released_reservations = existing
                .into_iter()
                .map(|row| {
                    row.into_released(
                        released_at.clone(),
                        params.release_reason,
                        params.release_metadata_json.clone(),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TaskReservationReleaseByOwnerResult {
                released_reservations,
                expired_reservations,
            })
        })
    }

    pub fn list_owned_task_reservation_conflicts(
        &self,
        params: &TaskReservationOwnedConflictsParams,
    ) -> Result<TaskReservationOwnedConflictsResult, OrbitError> {
        self.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            let now = crate::now_string();
            let expired_reservations = expire_reservations(
                tx,
                &params.workspace_orbit_dir,
                params.workspace_id.as_deref(),
                &now,
            )?;
            let reservations = find_owned_reservation_conflicts(
                tx,
                &params.workspace_orbit_dir,
                params.workspace_id.as_deref(),
                &now,
                &params.requested_files,
                params.limit,
            )?;
            Ok(TaskReservationOwnedConflictsResult {
                reservations,
                expired_reservations,
            })
        })
    }
}

fn expire_reservations(
    tx: &mut crate::StoreTx<'_>,
    workspace_orbit_dir: &str,
    workspace_id: Option<&str>,
    now: &str,
) -> Result<Vec<ExpiredTaskReservation>, OrbitError> {
    let sql = format!(
        "SELECT reservation_id, expires_at
         FROM task_reservations
         WHERE {}
           AND released_at IS NULL
           AND expires_at <= ?3
         ORDER BY expires_at ASC, reservation_id ASC",
        reservation_scope_clause(),
    );
    let mut stmt = tx
        .tx
        .prepare(&sql)
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = stmt
        .query_map(params![workspace_id, workspace_orbit_dir, now], |row| {
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
        let sql = format!(
            "UPDATE task_reservations
             SET released_at = ?3,
                 release_reason = ?4
             WHERE {}
               AND released_at IS NULL
               AND expires_at <= ?3",
            reservation_scope_clause(),
        );
        tx.tx
            .execute(
                &sql,
                params![
                    workspace_id,
                    workspace_orbit_dir,
                    now,
                    TaskReservationReleaseReason::TtlExpired.as_str(),
                ],
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
    }

    Ok(expired_reservations)
}

fn find_reservation_conflicts(
    tx: &mut crate::StoreTx<'_>,
    workspace_orbit_dir: &str,
    workspace_id: Option<&str>,
    now: &str,
    requested_files: &[String],
) -> Result<Vec<TaskLockConflict>, OrbitError> {
    let requested_files: BTreeSet<String> = requested_files.iter().cloned().collect();
    if requested_files.is_empty() {
        return Ok(Vec::new());
    }

    let sql = format!(
        "SELECT reservation_id, files_json
         FROM task_reservations
         WHERE {}
           AND released_at IS NULL
           AND expires_at > ?3
         ORDER BY created_at ASC, reservation_id ASC",
        reservation_scope_clause(),
    );
    let mut stmt = tx
        .tx
        .prepare(&sql)
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = stmt
        .query_map(params![workspace_id, workspace_orbit_dir, now], |row| {
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

fn reservation_scope_clause() -> &'static str {
    // Parameter contract for every caller: ?1 is workspace_id, ?2 is
    // workspace_orbit_dir. V2 callers see workspace-bound rows plus legacy
    // NULL-workspace rows for the same orbit dir; legacy callers see all rows
    // scoped to that orbit dir.
    "(
        (?1 IS NOT NULL AND (workspace_id = ?1 OR (workspace_id IS NULL AND workspace_orbit_dir = ?2)))
        OR (?1 IS NULL AND workspace_orbit_dir = ?2)
    )"
}

#[derive(Debug, Clone)]
struct ReservationRow {
    reservation_id: String,
    workspace_id: Option<String>,
    task_ids_json: String,
    files_json: String,
    actor: String,
    created_at: String,
    expires_at: String,
    released_at: Option<String>,
    owner_run_id: Option<String>,
    owner_metadata_json: Option<String>,
}

impl ReservationRow {
    fn into_active(self) -> Result<ActiveTaskReservation, OrbitError> {
        Ok(ActiveTaskReservation {
            reservation_id: self.reservation_id,
            workspace_id: self.workspace_id,
            task_ids: parse_string_list(&self.task_ids_json)?,
            files: parse_string_list(&self.files_json)?,
            actor: self.actor,
            created_at: self.created_at,
            expires_at: self.expires_at,
            owner_run_id: self.owner_run_id,
            owner_metadata_json: self.owner_metadata_json,
        })
    }

    fn into_released(
        self,
        released_at: String,
        release_reason: TaskReservationReleaseReason,
        release_metadata_json: Option<String>,
    ) -> Result<ReleasedTaskReservation, OrbitError> {
        Ok(ReleasedTaskReservation {
            reservation_id: self.reservation_id,
            workspace_id: self.workspace_id,
            task_ids: parse_string_list(&self.task_ids_json)?,
            files: parse_string_list(&self.files_json)?,
            actor: self.actor,
            created_at: self.created_at,
            expires_at: self.expires_at,
            released_at,
            owner_run_id: self.owner_run_id,
            owner_metadata_json: self.owner_metadata_json,
            release_reason,
            release_metadata_json,
        })
    }
}

fn select_reservation_columns() -> &'static str {
    "reservation_id, workspace_id, task_ids_json, files_json, actor, created_at, expires_at,
     released_at, owner_run_id, owner_metadata_json"
}

fn reservation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReservationRow> {
    Ok(ReservationRow {
        reservation_id: row.get(0)?,
        workspace_id: row.get(1)?,
        task_ids_json: row.get(2)?,
        files_json: row.get(3)?,
        actor: row.get(4)?,
        created_at: row.get(5)?,
        expires_at: row.get(6)?,
        released_at: row.get(7)?,
        owner_run_id: row.get(8)?,
        owner_metadata_json: row.get(9)?,
    })
}

fn load_reservation_row(
    tx: &mut crate::StoreTx<'_>,
    workspace_orbit_dir: &str,
    workspace_id: Option<&str>,
    reservation_id: &str,
) -> Result<Option<ReservationRow>, OrbitError> {
    let sql = format!(
        "SELECT {}
         FROM task_reservations
         WHERE {}
           AND reservation_id = ?3",
        select_reservation_columns(),
        reservation_scope_clause(),
    );
    tx.tx
        .query_row(
            &sql,
            params![workspace_id, workspace_orbit_dir, reservation_id],
            reservation_row,
        )
        .optional()
        .map_err(|error| OrbitError::Store(error.to_string()))
}

fn load_active_reservations_by_owner(
    tx: &mut crate::StoreTx<'_>,
    workspace_orbit_dir: &str,
    workspace_id: Option<&str>,
    owner_run_id: &str,
) -> Result<Vec<ReservationRow>, OrbitError> {
    let sql = format!(
        "SELECT {}
         FROM task_reservations
         WHERE {}
           AND owner_run_id = ?3
           AND released_at IS NULL
         ORDER BY created_at ASC, reservation_id ASC",
        select_reservation_columns(),
        reservation_scope_clause(),
    );
    let mut stmt = tx
        .tx
        .prepare(&sql)
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = stmt
        .query_map(
            params![workspace_id, workspace_orbit_dir, owner_run_id],
            reservation_row,
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| OrbitError::Store(error.to_string()))
}

fn find_owned_reservation_conflicts(
    tx: &mut crate::StoreTx<'_>,
    workspace_orbit_dir: &str,
    workspace_id: Option<&str>,
    now: &str,
    requested_files: &[String],
    limit: usize,
) -> Result<Vec<ActiveTaskReservation>, OrbitError> {
    let requested_files: BTreeSet<String> = requested_files.iter().cloned().collect();
    if requested_files.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }

    let sql = format!(
        "SELECT {}
         FROM task_reservations
         WHERE {}
           AND released_at IS NULL
           AND expires_at > ?3
           AND owner_run_id IS NOT NULL
         ORDER BY created_at ASC, reservation_id ASC",
        select_reservation_columns(),
        reservation_scope_clause(),
    );
    let mut stmt = tx
        .tx
        .prepare(&sql)
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let rows = stmt
        .query_map(
            params![workspace_id, workspace_orbit_dir, now],
            reservation_row,
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;

    let mut reservations = Vec::new();
    for row in rows {
        let row = row.map_err(|error| OrbitError::Store(error.to_string()))?;
        let reserved_files = parse_string_list(&row.files_json)?;
        let overlaps = requested_files.iter().any(|requested_file| {
            reserved_files
                .iter()
                .any(|held_file| workspace_relative_paths_overlap(requested_file, held_file))
        });
        if overlaps {
            reservations.push(row.into_active()?);
            if reservations.len() >= limit {
                break;
            }
        }
    }
    Ok(reservations)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn reserve_params(file: &str) -> TaskReservationReserveParams {
        TaskReservationReserveParams {
            workspace_orbit_dir: "/workspace/.orbit".to_string(),
            workspace_id: None,
            task_ids: vec!["T1".to_string()],
            requested_files: vec![file.to_string()],
            actor: "test".to_string(),
            ttl_seconds: 3600,
            owner_run_id: None,
            owner_metadata_json: None,
        }
    }

    #[test]
    fn task_reservation_workspace_id_scopes_rows_and_still_sees_legacy_path_rows() {
        let store = Store::open_in_memory().expect("open store");

        let mut legacy = reserve_params("file:src/legacy.rs");
        legacy.task_ids = vec!["T-legacy".to_string()];
        let legacy_result = store
            .reserve_task_reservation(&legacy)
            .expect("reserve legacy path row");
        assert!(legacy_result.reserved);

        let mut scoped = reserve_params("file:src/scoped.rs");
        scoped.workspace_id = Some("repo-abcdef".to_string());
        scoped.task_ids = vec!["ORB-00001".to_string()];
        let scoped_result = store
            .reserve_task_reservation(&scoped)
            .expect("reserve scoped row");
        assert!(scoped_result.reserved);

        let mut other = reserve_params("file:src/other.rs");
        other.workspace_id = Some("other-abcdef".to_string());
        let other_result = store
            .reserve_task_reservation(&other)
            .expect("reserve other workspace");
        assert!(other_result.reserved);

        let active = store
            .list_active_task_reservations("/workspace/.orbit", Some("repo-abcdef"))
            .expect("list scoped active reservations");
        let ids = active
            .reservations
            .iter()
            .map(|reservation| reservation.task_ids.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![vec!["T-legacy".to_string()], vec!["ORB-00001".to_string()]]
        );
        assert_eq!(active.reservations[0].workspace_id, None);
        assert_eq!(
            active.reservations[1].workspace_id.as_deref(),
            Some("repo-abcdef")
        );

        let conflicts = store
            .check_task_reservation_conflicts(&TaskReservationCheckParams {
                workspace_orbit_dir: "/workspace/.orbit".to_string(),
                workspace_id: Some("other-abcdef".to_string()),
                requested_files: vec!["file:src/scoped.rs".to_string()],
            })
            .expect("check other scope");
        assert!(conflicts.conflicts.is_empty());
    }

    #[test]
    fn task_reservation_persists_nullable_owner_context() {
        let store = Store::open_in_memory().expect("open store");

        let mut unowned = reserve_params("file:src/lib.rs");
        unowned.task_ids = vec!["T-unowned".to_string()];
        let unowned_result = store
            .reserve_task_reservation(&unowned)
            .expect("reserve unowned");
        assert!(unowned_result.reserved);

        let mut owned = reserve_params("file:src/main.rs");
        owned.task_ids = vec!["T-owned".to_string()];
        owned.owner_run_id = Some("jrun-owner".to_string());
        owned.owner_metadata_json = Some(r#"{"source":"test"}"#.to_string());
        let owned_result = store
            .reserve_task_reservation(&owned)
            .expect("reserve owned");
        assert!(owned_result.reserved);

        let active = store
            .list_active_task_reservations("/workspace/.orbit", None)
            .expect("list active");
        assert_eq!(active.reservations.len(), 2);
        let unowned_active = active
            .reservations
            .iter()
            .find(|reservation| {
                Some(reservation.reservation_id.as_str())
                    == unowned_result.reservation_id.as_deref()
            })
            .expect("unowned reservation");
        assert_eq!(unowned_active.owner_run_id, None);
        assert_eq!(unowned_active.owner_metadata_json, None);
        let owned_active = active
            .reservations
            .iter()
            .find(|reservation| {
                Some(reservation.reservation_id.as_str()) == owned_result.reservation_id.as_deref()
            })
            .expect("owned reservation");
        assert_eq!(owned_active.owner_run_id.as_deref(), Some("jrun-owner"));
        assert_eq!(
            owned_active.owner_metadata_json.as_deref(),
            Some(r#"{"source":"test"}"#)
        );
    }

    #[test]
    fn task_reservation_owner_batch_release_preserves_unowned_rows() {
        let store = Store::open_in_memory().expect("open store");
        let unowned = store
            .reserve_task_reservation(&reserve_params("file:src/lib.rs"))
            .expect("reserve unowned");

        let mut owned = reserve_params("file:src/main.rs");
        owned.owner_run_id = Some("jrun-owner".to_string());
        let owned_result = store
            .reserve_task_reservation(&owned)
            .expect("reserve owned");

        let released = store
            .release_task_reservations_by_owner_run_id(&TaskReservationReleaseByOwnerParams {
                workspace_orbit_dir: "/workspace/.orbit".to_string(),
                workspace_id: None,
                owner_run_id: "jrun-owner".to_string(),
                release_reason: TaskReservationReleaseReason::RunTerminal,
                release_metadata_json: Some(r#"{"why":"terminal"}"#.to_string()),
            })
            .expect("release owner");

        assert_eq!(released.released_reservations.len(), 1);
        assert_eq!(
            released.released_reservations[0].reservation_id,
            owned_result.reservation_id.expect("owned reservation id")
        );
        assert_eq!(
            released.released_reservations[0].release_reason,
            TaskReservationReleaseReason::RunTerminal
        );

        let active = store
            .list_active_task_reservations("/workspace/.orbit", None)
            .expect("list active");
        assert_eq!(active.reservations.len(), 1);
        assert_eq!(
            Some(active.reservations[0].reservation_id.as_str()),
            unowned.reservation_id.as_deref()
        );
        assert_eq!(active.reservations[0].owner_run_id, None);
    }

    #[test]
    fn task_reservation_explicit_release_is_idempotent_without_metadata_churn() {
        let store = Store::open_in_memory().expect("open store");
        let mut params = reserve_params("file:src/lib.rs");
        params.owner_run_id = Some("jrun-owner".to_string());
        let reservation = store
            .reserve_task_reservation(&params)
            .expect("reserve")
            .reservation_id
            .expect("reservation id");

        let first = store
            .release_task_reservation(&TaskReservationReleaseParams {
                workspace_orbit_dir: "/workspace/.orbit".to_string(),
                workspace_id: None,
                reservation_id: reservation.clone(),
                release_reason: TaskReservationReleaseReason::Explicit,
                release_metadata_json: Some(r#"{"first":true}"#.to_string()),
            })
            .expect("release first");
        assert!(first.released);
        assert_eq!(
            first
                .reservation
                .as_ref()
                .and_then(|reservation| reservation.owner_run_id.as_deref()),
            Some("jrun-owner")
        );

        let second = store
            .release_task_reservation(&TaskReservationReleaseParams {
                workspace_orbit_dir: "/workspace/.orbit".to_string(),
                workspace_id: None,
                reservation_id: reservation.clone(),
                release_reason: TaskReservationReleaseReason::RunTerminal,
                release_metadata_json: Some(r#"{"second":true}"#.to_string()),
            })
            .expect("release second");
        assert!(!second.released);

        let conn = store.connection();
        let guard = conn.lock().expect("conn lock");
        let (reason, metadata): (String, Option<String>) = guard
            .query_row(
                "SELECT release_reason, release_metadata_json
                 FROM task_reservations
                 WHERE reservation_id = ?1",
                params![reservation],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("query release metadata");
        assert_eq!(reason, "explicit");
        assert_eq!(metadata.as_deref(), Some(r#"{"first":true}"#));
    }

    #[test]
    fn task_reservation_owned_conflicts_return_owner_fields() {
        let store = Store::open_in_memory().expect("open store");
        let mut params = reserve_params("file:src/lib.rs");
        params.owner_run_id = Some("jrun-owner".to_string());
        params.owner_metadata_json = Some(r#"{"source":"test"}"#.to_string());
        store
            .reserve_task_reservation(&params)
            .expect("reserve owned");

        let conflicts = store
            .list_owned_task_reservation_conflicts(&TaskReservationOwnedConflictsParams {
                workspace_orbit_dir: "/workspace/.orbit".to_string(),
                workspace_id: None,
                requested_files: vec!["file:src/lib.rs".to_string()],
                limit: 10,
            })
            .expect("owned conflicts");

        assert_eq!(conflicts.reservations.len(), 1);
        assert_eq!(
            conflicts.reservations[0].owner_run_id.as_deref(),
            Some("jrun-owner")
        );
        assert_eq!(
            conflicts.reservations[0].owner_metadata_json.as_deref(),
            Some(r#"{"source":"test"}"#)
        );
    }

    #[test]
    fn task_reservation_owned_conflict_limit_applies_after_overlap_filter() {
        let store = Store::open_in_memory().expect("open store");
        for index in 0..3 {
            let mut params = reserve_params(&format!("file:src/non_overlap_{index}.rs"));
            params.owner_run_id = Some(format!("jrun-non-overlap-{index}"));
            store.reserve_task_reservation(&params).expect("reserve");
        }

        let mut overlapping = reserve_params("file:src/target.rs");
        overlapping.owner_run_id = Some("jrun-target".to_string());
        store
            .reserve_task_reservation(&overlapping)
            .expect("reserve overlapping");

        let conflicts = store
            .list_owned_task_reservation_conflicts(&TaskReservationOwnedConflictsParams {
                workspace_orbit_dir: "/workspace/.orbit".to_string(),
                workspace_id: None,
                requested_files: vec!["file:src/target.rs".to_string()],
                limit: 1,
            })
            .expect("owned conflicts");

        assert_eq!(conflicts.reservations.len(), 1);
        assert_eq!(
            conflicts.reservations[0].owner_run_id.as_deref(),
            Some("jrun-target")
        );
    }
}
