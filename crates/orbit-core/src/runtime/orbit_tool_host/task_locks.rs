use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use orbit_common::types::{
    AuditEventStatus, NotFoundKind, OrbitError, Task, TaskStatus, audit_execution_id,
    normalize_optional_attribution_label, optional_string_list_alias, optional_u32_alias,
    prune_missing_context_files, required_string,
};
use orbit_common::utility::path::workspace_relative_paths_overlap;
use orbit_common::utility::selector::Selector;
use orbit_store::sqlite::task_registry::read_workspace_config_optional;
use orbit_store::{
    ExpiredTaskReservation, ReleasedTaskReservation, TaskLockConflict, TaskLockHolder,
    TaskReservationCheckParams, TaskReservationReleaseParams, TaskReservationReleaseReason,
    TaskReservationReserveParams,
};
use orbit_tools::ReservationOwnerContext;
use serde_json::{Value, json};

use crate::OrbitRuntime;
use crate::command::task::canonicalize_context_files_for_read;

use super::json::{task_lock_status_rank, task_lock_to_json};

pub(super) fn list(runtime: &OrbitRuntime) -> Result<Value, OrbitError> {
    let workspace_id = workspace_task_reservation_id(runtime)?;
    let reservation_result = runtime
        .stores()
        .task_reservations()
        .list_active(&workspace_orbit_dir(runtime), workspace_id.as_deref())?;
    emit_expired_reservation_events(runtime, &reservation_result.expired_reservations)?;

    let mut tasks: Vec<_> = runtime
        .list_tasks()?
        .into_iter()
        .filter(|task| matches!(task.status, TaskStatus::InProgress | TaskStatus::Review))
        .collect();
    tasks.sort_by_key(|task| {
        (
            task_lock_status_rank(task.status),
            task.created_at,
            task.id.clone(),
        )
    });

    let locked_files: BTreeSet<String> = tasks
        .iter()
        .flat_map(|task| existing_context_files(runtime, task))
        .chain(
            reservation_result
                .reservations
                .iter()
                .flat_map(|reservation| reservation.files.iter().cloned()),
        )
        .collect();
    let by_reservation = reservation_result
        .reservations
        .iter()
        .map(|reservation| {
            json!({
                "reservation_id": reservation.reservation_id.clone(),
                "workspace_id": reservation.workspace_id.clone(),
                "task_ids": reservation.task_ids.clone(),
                "files": reservation.files.clone(),
                "actor": reservation.actor.clone(),
                "created_at": reservation.created_at.clone(),
                "expires_at": reservation.expires_at.clone(),
                "owner_run_id": reservation.owner_run_id.clone(),
                "owner_metadata_json": reservation.owner_metadata_json.clone(),
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "locked_files": locked_files.iter().cloned().collect::<Vec<_>>(),
        "by_task": tasks.iter().map(task_lock_to_json).collect::<Vec<_>>(),
        "by_reservation": by_reservation,
        "total_locked": locked_files.len(),
        "total_tasks": tasks.len(),
        "total_reservations": reservation_result.reservations.len(),
    }))
}

pub(super) fn release(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
) -> Result<Value, OrbitError> {
    let reservation_id = required_string(
        &input,
        &["reservation_id", "reservationId", "reservation-id"],
        "reservation_id",
    )?;
    let result = runtime
        .stores()
        .task_reservations()
        .release(TaskReservationReleaseParams {
            workspace_orbit_dir: workspace_orbit_dir(runtime),
            workspace_id: workspace_task_reservation_id(runtime)?,
            reservation_id: reservation_id.clone(),
            release_reason: TaskReservationReleaseReason::Explicit,
            release_metadata_json: Some(
                json!({
                    "released_by": reservation_actor_label(
                        runtime,
                        agent.as_deref(),
                        model.as_deref(),
                    ),
                })
                .to_string(),
            ),
        })?;
    emit_expired_reservation_events(runtime, &result.expired_reservations)?;
    if result.released {
        let released_task_id = result
            .reservation
            .as_ref()
            .and_then(|reservation| first_task_id(&reservation.task_ids));
        let owner_run_id = result
            .reservation
            .as_ref()
            .and_then(|reservation| reservation.owner_run_id.clone());
        record_task_lock_audit_event(
            runtime,
            "task.locks.reserve.released",
            "orbit.task.locks.release",
            Some(reservation_id.as_str()),
            released_task_id,
            AuditEventStatus::Success,
            json!({
                "reservation_id": reservation_id,
                "owner_run_id": owner_run_id,
                "release_reason": TaskReservationReleaseReason::Explicit.as_str(),
                "released_at": result.released_at,
                "released_by": reservation_actor_label(
                    runtime,
                    agent.as_deref(),
                    model.as_deref(),
                ),
            }),
        )?;
    }
    Ok(json!({ "released": result.released }))
}

pub(super) fn reserve(
    runtime: &OrbitRuntime,
    input: Value,
    agent: Option<String>,
    model: Option<String>,
    reservation_owner: Option<ReservationOwnerContext>,
) -> Result<Value, OrbitError> {
    let reservation_scope = parse_task_lock_reservation_scope(&input)?;
    let ttl_seconds =
        optional_u32_alias(&input, &["ttl_seconds", "ttlSeconds", "ttl-seconds"])?.unwrap_or(1800);
    if !(1..=7200).contains(&ttl_seconds) {
        return Err(OrbitError::InvalidInput(
            "`ttl_seconds` must be between 1 and 7200 seconds".to_string(),
        ));
    }

    let actor = reservation_actor_label(runtime, agent.as_deref(), model.as_deref());
    let workspace_id = workspace_task_reservation_id(runtime)?;
    let (task_ids, requested_files) = match &reservation_scope {
        TaskLockReservationScope::TaskIds(task_ids) => {
            (task_ids.clone(), requested_task_files(runtime, task_ids)?)
        }
        TaskLockReservationScope::Files(files) => (Vec::new(), files.clone()),
    };
    runtime.reconcile_stale_owned_reservations_for_files(&requested_files, 32)?;
    let mut conflicts = task_lock_conflicts(runtime, &task_ids, &requested_files)?;

    record_task_lock_audit_event(
        runtime,
        "task.locks.reserve.requested",
        "orbit.task.locks.reserve",
        None,
        first_task_id(&task_ids),
        AuditEventStatus::Success,
        json!({
            "actor": actor.clone(),
            "task_ids": task_ids.clone(),
            "files": requested_files.clone(),
            "ttl_seconds": ttl_seconds,
            "owner_run_id": reservation_owner
                .as_ref()
                .map(|owner| owner.owner_run_id.clone()),
        }),
    )?;

    let reservation_result = if conflicts.is_empty() {
        runtime
            .stores()
            .task_reservations()
            .reserve(TaskReservationReserveParams {
                workspace_orbit_dir: workspace_orbit_dir(runtime),
                workspace_id: workspace_id.clone(),
                task_ids: task_ids.clone(),
                requested_files: requested_files.clone(),
                actor: actor.clone(),
                ttl_seconds,
                owner_run_id: reservation_owner
                    .as_ref()
                    .map(|owner| owner.owner_run_id.clone()),
                owner_metadata_json: reservation_owner
                    .as_ref()
                    .and_then(|owner| owner.owner_metadata_json.clone()),
            })?
    } else {
        let check = runtime
            .stores()
            .task_reservations()
            .check(TaskReservationCheckParams {
                workspace_orbit_dir: workspace_orbit_dir(runtime),
                workspace_id: workspace_id.clone(),
                requested_files: requested_files.clone(),
            })?;
        conflicts = merge_task_lock_conflicts(conflicts, check.conflicts);
        emit_expired_reservation_events(runtime, &check.expired_reservations)?;
        orbit_store::TaskReservationReserveResult {
            reserved: false,
            reservation_id: None,
            expires_at: None,
            reserved_files: Vec::new(),
            conflicts: conflicts.clone(),
            expired_reservations: Vec::new(),
        }
    };

    emit_expired_reservation_events(runtime, &reservation_result.expired_reservations)?;

    if reservation_result.reserved {
        let reservation_id = reservation_result.reservation_id.clone().ok_or_else(|| {
            OrbitError::Execution("reservation grant is missing reservation_id".to_string())
        })?;
        record_task_lock_audit_event(
            runtime,
            "task.locks.reserve.granted",
            "orbit.task.locks.reserve",
            Some(reservation_id.as_str()),
            first_task_id(&task_ids),
            AuditEventStatus::Success,
            json!({
                "reservation_id": reservation_id,
                "files": reservation_result.reserved_files.clone(),
                "expires_at": reservation_result.expires_at.clone(),
                "actor": actor,
                "task_ids": task_ids.clone(),
                "owner_run_id": reservation_owner
                    .as_ref()
                    .map(|owner| owner.owner_run_id.clone()),
            }),
        )?;
        Ok(json!({
            "reserved": true,
            "reservation_id": reservation_result.reservation_id,
            "expires_at": reservation_result.expires_at,
            "reserved_files": reservation_result.reserved_files,
        }))
    } else {
        let conflicts = merge_task_lock_conflicts(conflicts, reservation_result.conflicts);
        record_task_lock_audit_event(
            runtime,
            "task.locks.reserve.denied",
            "orbit.task.locks.reserve",
            None,
            first_task_id(&task_ids),
            AuditEventStatus::Denied,
            json!({
                "actor": actor,
                "task_ids": task_ids.clone(),
                "files": requested_files.clone(),
                "conflicts": conflicts.clone(),
                "owner_run_id": reservation_owner
                    .as_ref()
                    .map(|owner| owner.owner_run_id.clone()),
            }),
        )?;
        Ok(json!({
            "reserved": false,
            "conflicts": conflicts,
        }))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TaskLockReservationScope {
    TaskIds(Vec<String>),
    Files(Vec<String>),
}

pub(super) fn parse_task_lock_reservation_scope(
    input: &Value,
) -> Result<TaskLockReservationScope, OrbitError> {
    let task_ids = optional_string_list_alias(input, &["task_ids", "taskIds", "task-ids"])?;
    let files = optional_string_list_alias(input, &["files"])?;

    match (task_ids, files) {
        (Some(_), Some(_)) | (None, None) => Err(OrbitError::InvalidInput(
            "exactly one of 'task_ids' or 'files' must be provided".to_string(),
        )),
        (Some(task_ids), None) => {
            parse_task_id_list(task_ids).map(TaskLockReservationScope::TaskIds)
        }
        (None, Some(files)) => {
            parse_file_lock_selectors(files).map(TaskLockReservationScope::Files)
        }
    }
}

fn parse_task_id_list(task_ids: Vec<String>) -> Result<Vec<String>, OrbitError> {
    let deduped = task_ids.into_iter().collect::<BTreeSet<_>>();
    if deduped.is_empty() {
        return Err(OrbitError::InvalidInput(
            "`task_ids` must contain at least one task ID".to_string(),
        ));
    }
    Ok(deduped.into_iter().collect())
}

fn parse_file_lock_selectors(files: Vec<String>) -> Result<Vec<String>, OrbitError> {
    let mut deduped = BTreeSet::new();
    for raw in files {
        let selector: Selector = raw.parse().map_err(|error| {
            OrbitError::InvalidInput(format!(
                "`files` entries must be canonical file or directory selectors using `file:` or `dir:`: {error}"
            ))
        })?;
        match &selector {
            Selector::Dir { .. } | Selector::File { .. } => {
                deduped.insert(selector.to_string());
            }
            Selector::Symbol { .. } => {
                return Err(OrbitError::InvalidInput(
                    "`files` entries must be canonical file or directory selectors using `file:` or `dir:`; `symbol:` selectors are not supported for task locks".to_string(),
                ));
            }
        }
    }
    if deduped.is_empty() {
        return Err(OrbitError::InvalidInput(
            "`files` must contain at least one file or directory selector using `file:` or `dir:`"
                .to_string(),
        ));
    }
    Ok(deduped.into_iter().collect())
}

pub(crate) fn workspace_orbit_dir(runtime: &OrbitRuntime) -> String {
    runtime.paths().orbit_dir.to_string_lossy().into_owned()
}

pub(crate) fn workspace_task_reservation_id(
    runtime: &OrbitRuntime,
) -> Result<Option<String>, OrbitError> {
    match read_workspace_config_optional(&runtime.paths().orbit_dir)? {
        Some(config) => Ok(Some(config.workspace_id)),
        None => Err(OrbitError::Store(format!(
            "task artifact workspace config is missing at '{}'; rebuild the runtime before writing task lock reservations",
            runtime.paths().orbit_dir.join("config.yaml").display()
        ))),
    }
}

fn task_workspace_root(runtime: &OrbitRuntime, task: &Task) -> PathBuf {
    let _ = task;
    runtime.paths().repo_root.clone()
}

fn existing_context_files(runtime: &OrbitRuntime, task: &Task) -> Vec<String> {
    let workspace_root = task_workspace_root(runtime, task);
    let canonical = canonicalize_context_files_for_read(&task.context_files, &workspace_root);
    let (kept, _dropped) = prune_missing_context_files(&workspace_root, canonical);
    kept
}

pub(crate) fn requested_task_files(
    runtime: &OrbitRuntime,
    task_ids: &[String],
) -> Result<Vec<String>, OrbitError> {
    let tasks = runtime.stores().tasks().list()?;
    let task_map = tasks
        .into_iter()
        .map(|task| (task.id.clone(), task))
        .collect::<BTreeMap<_, _>>();

    let mut requested_files = BTreeSet::new();
    for task_id in task_ids {
        let task = task_map
            .get(task_id)
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, task_id.clone()))?;
        requested_files.extend(existing_context_files(runtime, task));
    }

    Ok(requested_files.into_iter().collect())
}

pub(crate) fn task_lock_conflicts(
    runtime: &OrbitRuntime,
    bundle_task_ids: &[String],
    requested_files: &[String],
) -> Result<Vec<TaskLockConflict>, OrbitError> {
    let bundle_ids = bundle_task_ids.iter().cloned().collect::<BTreeSet<_>>();
    let requested_files = requested_files.iter().cloned().collect::<BTreeSet<_>>();
    if requested_files.is_empty() {
        return Ok(Vec::new());
    }

    let mut tasks: Vec<Task> = runtime
        .stores()
        .tasks()
        .list()?
        .into_iter()
        .filter(|task| {
            matches!(task.status, TaskStatus::InProgress | TaskStatus::Review)
                && !bundle_ids.contains(&task.id)
        })
        .collect();
    tasks.sort_by_key(|task| {
        (
            task_lock_status_rank(task.status),
            task.created_at,
            task.id.clone(),
        )
    });

    let mut conflicts = Vec::new();
    for task in tasks {
        let held_files = existing_context_files(runtime, &task);
        for requested_file in &requested_files {
            if held_files
                .iter()
                .any(|held_file| workspace_relative_paths_overlap(requested_file, held_file))
            {
                conflicts.push(TaskLockConflict {
                    file: requested_file.clone(),
                    held_by: TaskLockHolder::Task,
                    held_by_id: task.id.clone(),
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

pub(crate) fn merge_task_lock_conflicts(
    left: Vec<TaskLockConflict>,
    right: Vec<TaskLockConflict>,
) -> Vec<TaskLockConflict> {
    let mut merged = left;
    merged.extend(right);
    merged.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then_with(|| match (a.held_by, b.held_by) {
                (TaskLockHolder::Task, TaskLockHolder::Reservation) => std::cmp::Ordering::Less,
                (TaskLockHolder::Reservation, TaskLockHolder::Task) => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            })
            .then(a.held_by_id.cmp(&b.held_by_id))
    });
    merged.dedup_by(|a, b| {
        a.file == b.file && a.held_by == b.held_by && a.held_by_id == b.held_by_id
    });
    merged
}

pub(crate) fn emit_expired_reservation_events(
    runtime: &OrbitRuntime,
    expired_reservations: &[ExpiredTaskReservation],
) -> Result<(), OrbitError> {
    for expired in expired_reservations {
        record_task_lock_audit_event(
            runtime,
            "task.locks.reserve.expired",
            "orbit.task.locks.reserve",
            Some(expired.reservation_id.as_str()),
            None,
            AuditEventStatus::Success,
            json!({
                "reservation_id": expired.reservation_id,
                "expired_at": expired.expired_at,
            }),
        )?;
    }
    Ok(())
}

pub(crate) fn emit_task_lock_release_event(
    runtime: &OrbitRuntime,
    reservation: &ReleasedTaskReservation,
    release_reason: TaskReservationReleaseReason,
) -> Result<(), OrbitError> {
    record_task_lock_audit_event(
        runtime,
        "task.locks.reserve.released",
        "orbit.task.locks.release",
        Some(reservation.reservation_id.as_str()),
        first_task_id(&reservation.task_ids),
        AuditEventStatus::Success,
        json!({
            "reservation_id": reservation.reservation_id,
            "owner_run_id": reservation.owner_run_id,
            "release_reason": release_reason.as_str(),
            "released_at": reservation.released_at,
        }),
    )
}

fn reservation_actor_label(
    runtime: &OrbitRuntime,
    agent: Option<&str>,
    model: Option<&str>,
) -> String {
    normalize_optional_attribution_label(model.or(agent), model)
        .unwrap_or_else(|| runtime.actor_label().to_string())
}

fn record_task_lock_audit_event(
    runtime: &OrbitRuntime,
    command: &str,
    tool_name: &str,
    target_id: Option<&str>,
    task_id: Option<&str>,
    status: AuditEventStatus,
    payload: Value,
) -> Result<(), OrbitError> {
    let execution_id_prefix = format!("audit-{}", command.replace('.', "-"));
    let job_run_id = owner_run_id_from_payload(&payload)
        .or_else(|| std::env::var("ORBIT_RUN_ID").ok().filter(|s| !s.is_empty()));
    runtime.record_audit_event(&crate::AuditEventInsertParams {
        execution_id: audit_execution_id(&execution_id_prefix),
        command: command.to_string(),
        subcommand: None,
        tool_name: Some(tool_name.to_string()),
        target_type: Some("task_reservation".to_string()),
        target_id: target_id.map(ToOwned::to_owned),
        role: "admin".to_string(),
        status,
        exit_code: if status == AuditEventStatus::Denied {
            1
        } else {
            0
        },
        duration_ms: 0,
        working_directory: runtime.paths().repo_root.to_string_lossy().into_owned(),
        arguments_json: Some(
            serde_json::to_string(&payload).map_err(|error| {
                OrbitError::Execution(format!("serialize audit payload: {error}"))
            })?,
        ),
        stdout_truncated: None,
        stderr_truncated: None,
        error_message: None,
        host: std::env::var("HOSTNAME").ok(),
        pid: std::process::id(),
        session_id: None,
        task_id: task_id.map(ToOwned::to_owned),
        job_run_id,
        activity_id: std::env::var("ORBIT_ACTIVITY_ID")
            .ok()
            .filter(|s| !s.is_empty()),
        step_index: std::env::var("ORBIT_STEP_INDEX")
            .ok()
            .and_then(|s| s.parse().ok()),
        backend: None,
    })
}

fn first_task_id(task_ids: &[String]) -> Option<&str> {
    task_ids.first().map(String::as_str)
}

fn owner_run_id_from_payload(payload: &Value) -> Option<String> {
    payload
        .get("owner_run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_common::types::{AuditEvent, Role, TaskStatus};
    use orbit_tools::{ReservationOwnerContext, ToolContext};
    use serde_json::{Value, json};

    use super::super::test_support::{create_context_task, test_runtime, unmanaged_tool_env_guard};

    fn task_lock_audit_event(runtime: &OrbitRuntime, tool_name: &str, command: &str) -> AuditEvent {
        runtime
            .list_audit_events(None, Some(tool_name.to_string()), None, None, 16)
            .expect("list audit events")
            .into_iter()
            .find(|event| event.command == command)
            .expect("task lock audit event")
    }

    fn reserve_files(runtime: &OrbitRuntime, owner_run_id: Option<&str>) -> String {
        let input = json!({
            "files": ["file:src/lib.rs"],
            "ttl_seconds": 3600,
            "model": "gpt-5.5",
        });
        let output = match owner_run_id {
            Some(owner_run_id) => runtime
                .run_tool_with_context_and_role(
                    "orbit.task.locks.reserve",
                    input,
                    Role::Admin,
                    ToolContext {
                        reservation_owner: Some(ReservationOwnerContext {
                            owner_run_id: owner_run_id.to_string(),
                            owner_metadata_json: None,
                        }),
                        ..ToolContext::default()
                    },
                )
                .expect("reserve direct file selectors with owner"),
            None => runtime
                .run_tool("orbit.task.locks.reserve", input)
                .expect("reserve direct file selectors"),
        };

        output
            .get("reservation_id")
            .and_then(Value::as_str)
            .expect("reservation id")
            .to_string()
    }

    #[test]
    fn release_audit_without_owner_has_no_task_or_job_run_id() {
        let _env = unmanaged_tool_env_guard();
        let (_root, runtime, repo_root) = test_runtime();
        std::fs::create_dir_all(repo_root.join("src")).expect("create src dir");
        std::fs::write(repo_root.join("src/lib.rs"), "pub fn ok() {}\n")
            .expect("write source file");

        let reservation_id = reserve_files(&runtime, None);
        let release = runtime
            .run_tool(
                "orbit.task.locks.release",
                json!({
                    "reservation_id": reservation_id.clone(),
                    "model": "gpt-5.5",
                }),
            )
            .expect("release reservation");
        assert_eq!(release["released"], true);

        let row = task_lock_audit_event(
            &runtime,
            "orbit.task.locks.release",
            "task.locks.reserve.released",
        );
        assert_eq!(row.target_id.as_deref(), Some(reservation_id.as_str()));
        assert!(row.task_id.is_none());
        assert!(row.job_run_id.is_none());
    }

    #[test]
    fn release_audit_uses_reservation_owner_run_id() {
        let _env = unmanaged_tool_env_guard();
        let (_root, runtime, repo_root) = test_runtime();
        std::fs::create_dir_all(repo_root.join("src")).expect("create src dir");
        std::fs::write(repo_root.join("src/lib.rs"), "pub fn ok() {}\n")
            .expect("write source file");

        let reservation_id = reserve_files(&runtime, Some("jrun-owner"));
        let release = runtime
            .run_tool(
                "orbit.task.locks.release",
                json!({
                    "reservation_id": reservation_id.clone(),
                    "model": "gpt-5.5",
                }),
            )
            .expect("release reservation");
        assert_eq!(release["released"], true);

        let row = task_lock_audit_event(
            &runtime,
            "orbit.task.locks.release",
            "task.locks.reserve.released",
        );
        assert_eq!(row.target_id.as_deref(), Some(reservation_id.as_str()));
        assert!(row.task_id.is_none());
        assert_eq!(row.job_run_id.as_deref(), Some("jrun-owner"));
    }

    #[test]
    fn reserve_audit_for_task_scope_records_first_task_id() {
        let _env = unmanaged_tool_env_guard();
        let (_root, runtime, repo_root) = test_runtime();
        std::fs::create_dir_all(repo_root.join("src")).expect("create src dir");
        std::fs::write(repo_root.join("src/lib.rs"), "pub fn ok() {}\n")
            .expect("write source file");
        let task = create_context_task(
            &runtime,
            &repo_root,
            TaskStatus::Backlog,
            &["file:src/lib.rs"],
        );

        let reserve = runtime
            .run_tool(
                "orbit.task.locks.reserve",
                json!({
                    "task_ids": [task.id.clone()],
                    "ttl_seconds": 3600,
                    "model": "gpt-5.5",
                }),
            )
            .expect("reserve task scope");
        assert_eq!(reserve["reserved"], true);
        let reservation_id = reserve["reservation_id"]
            .as_str()
            .expect("reservation id")
            .to_string();

        let row = task_lock_audit_event(
            &runtime,
            "orbit.task.locks.reserve",
            "task.locks.reserve.granted",
        );
        assert_eq!(row.target_id.as_deref(), Some(reservation_id.as_str()));
        assert_eq!(row.task_id.as_deref(), Some(task.id.as_str()));
    }
}
