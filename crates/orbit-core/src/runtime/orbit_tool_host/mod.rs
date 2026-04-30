mod input;
mod json;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use orbit_common::types::{
    AuditEventStatus, OrbitError, ReviewThreadStatus, Task, TaskPriority, TaskStatus,
    build_task_status_index, normalize_optional_attribution_label,
    optional_csv_or_string_list_alias, optional_raw_string, optional_string, optional_string_alias,
    optional_string_list_alias, optional_u32_alias, prune_missing_context_files, required_string,
    task_dependencies_ready,
};
use orbit_common::utility::path::workspace_relative_paths_overlap;
use orbit_store::{
    ExpiredTaskReservation, TaskLockConflict, TaskLockHolder, TaskReservationCheckParams,
    TaskReservationReleaseParams, TaskReservationReserveParams, state_io,
};
use orbit_tools::{OrbitBuiltinAction, OrbitTaskScope, OrbitToolHost};
use serde_json::{Value, json};

use self::input::{
    empty_string_to_none, optional_bool_alias, parse_artifacts,
    parse_optional_poll_interval_seconds, parse_optional_timeout_seconds, parse_string_array_field,
    parse_task_complexity, parse_task_priority, parse_task_status, parse_task_type,
    require_object_field, resolve_state_dir, resolve_state_payload, resolve_step_index,
};
use self::json::{
    serialize_error, serialize_task, serialize_task_lint_report, task_fields_to_json,
    task_lock_status_rank, task_lock_to_json, task_to_json,
};
use crate::OrbitRuntime;
use crate::command::task::{TaskAddParams, TaskUpdateParams, canonicalize_context_files_for_read};

pub(crate) fn build_orbit_tool_host(
    runtime: &OrbitRuntime,
    task_id: Option<String>,
) -> Arc<dyn OrbitToolHost> {
    Arc::new(RuntimeOrbitToolHost {
        runtime: runtime.clone(),
        task_scope: OrbitTaskScope {
            orbit_root: Some(runtime.data_root_path().to_path_buf()),
            task_id,
        },
    })
}

#[derive(Clone)]
struct RuntimeOrbitToolHost {
    runtime: OrbitRuntime,
    task_scope: OrbitTaskScope,
}

impl OrbitToolHost for RuntimeOrbitToolHost {
    fn execute(
        &self,
        action: OrbitBuiltinAction,
        input: Value,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Value, OrbitError> {
        let (agent, model) = self
            .runtime
            .try_canonical_agent_model_identity(agent.as_deref(), model.as_deref())?;
        match action {
            OrbitBuiltinAction::PipelineInvoke => {
                let job_name = required_string(&input, &["job_name"], "job_name")?;
                let payload = require_object_field(&input, "input")?.clone();
                let priority = optional_string(&input, "priority")?
                    .map(|value| parse_task_priority("priority", &value))
                    .transpose()?
                    .map(|value| value.to_string());
                let actor = Some(
                    normalize_optional_attribution_label(
                        model.as_deref().or(agent.as_deref()),
                        model.as_deref(),
                    )
                    .unwrap_or_else(|| self.runtime.actor_label().to_string()),
                )
                .filter(|value| !value.trim().is_empty());
                serde_json::to_value(self.runtime.submit_pipeline_run(
                    &job_name,
                    payload,
                    priority.as_deref(),
                    actor.as_deref(),
                )?)
                .map_err(serialize_error("serialize pipeline invoke"))
            }
            OrbitBuiltinAction::PipelineWait => {
                let run_ids = parse_string_array_field(&input, "run_ids")?;
                let timeout_seconds = OrbitRuntime::normalize_pipeline_wait_timeout(
                    parse_optional_timeout_seconds(&input)?,
                )?;
                let poll_interval_seconds = OrbitRuntime::normalize_pipeline_wait_poll_interval(
                    parse_optional_poll_interval_seconds(&input)?,
                );
                let actor = Some(
                    normalize_optional_attribution_label(
                        model.as_deref().or(agent.as_deref()),
                        model.as_deref(),
                    )
                    .unwrap_or_else(|| self.runtime.actor_label().to_string()),
                )
                .filter(|value| !value.trim().is_empty());
                serde_json::to_value(self.runtime.wait_pipeline_runs(
                    &run_ids,
                    timeout_seconds,
                    poll_interval_seconds,
                    actor.as_deref(),
                )?)
                .map_err(serialize_error("serialize pipeline wait"))
            }
            OrbitBuiltinAction::ReviewThreadAdd => {
                require_review_model(model.as_deref(), "orbit.task.review_thread.add")?;
                let id = required_string(&input, &["id"], "id")?;
                let body = required_string(&input, &["body"], "body")?;
                let path = optional_string(&input, "path")?;
                let line = optional_string(&input, "line")?
                    .map(|value| {
                        value.parse::<u64>().map_err(|error| {
                            OrbitError::InvalidInput(format!(
                                "`line` must be an unsigned integer: {error}"
                            ))
                        })
                    })
                    .transpose()?;
                self.runtime
                    .add_review_thread(&id, body, path, line, agent, model)?;
                serialize_task(&self.runtime, &self.runtime.get_task(&id)?)
            }
            OrbitBuiltinAction::ReviewThreadList => {
                let id = required_string(&input, &["id"], "id")?;
                let status = optional_string(&input, "status")?
                    .map(|value| ReviewThreadStatus::from_str(&value))
                    .transpose()
                    .map_err(OrbitError::InvalidInput)?;
                serde_json::to_value(self.runtime.list_review_threads(&id, status)?)
                    .map_err(serialize_error("serialize review threads"))
            }
            OrbitBuiltinAction::ReviewThreadReply => {
                require_review_model(model.as_deref(), "orbit.task.review_thread.reply")?;
                let id = required_string(&input, &["id"], "id")?;
                let thread_id = required_string(&input, &["thread_id"], "thread_id")?;
                let body = required_string(&input, &["body"], "body")?;
                self.runtime
                    .reply_review_thread(&id, &thread_id, body, agent, model)?;
                serialize_task(&self.runtime, &self.runtime.get_task(&id)?)
            }
            OrbitBuiltinAction::ReviewThreadResolve => {
                let id = required_string(&input, &["id"], "id")?;
                let thread_id = required_string(&input, &["thread_id"], "thread_id")?;
                self.runtime
                    .resolve_review_thread(&id, &thread_id, agent, model)?;
                serialize_task(&self.runtime, &self.runtime.get_task(&id)?)
            }
            OrbitBuiltinAction::StateGet => {
                let state_dir = resolve_state_dir(&self.task_scope, &input)?;
                let pipeline = state_io::read_pipeline(&state_dir)?;
                match optional_string(&input, "key")? {
                    Some(key) => Ok(pipeline
                        .as_object()
                        .and_then(|map| map.get(&key))
                        .cloned()
                        .unwrap_or(Value::Null)),
                    None => Ok(pipeline),
                }
            }
            OrbitBuiltinAction::StateSet => {
                let state_dir = resolve_state_dir(&self.task_scope, &input)?;
                let step_index = resolve_step_index(&input)?;
                let payload = resolve_state_payload(&input)?;
                state_io::write_step_output(&state_dir, step_index, &payload)?;
                Ok(json!({
                    "state_dir": state_dir.display().to_string(),
                    "step_index": step_index,
                    "written": payload,
                }))
            }
            OrbitBuiltinAction::TaskAdd => {
                let title = required_string(&input, &["title"], "title")?;
                let description = required_string(&input, &["description"], "description")?;
                let workspace = required_string(&input, &["workspace"], "workspace")?;
                let plan = match input.get("plan") {
                    Some(Value::String(value)) => value.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(_) => {
                        return Err(OrbitError::InvalidInput(
                            "`plan` must be a string".to_string(),
                        ));
                    }
                };
                let task = self.runtime.add_task_with_identity(
                    TaskAddParams {
                        parent_id: optional_string_alias(
                            &input,
                            &["parent_id", "parent", "parentId"],
                        )?,
                        title,
                        description,
                        acceptance_criteria: optional_string_list_alias(
                            &input,
                            &[
                                "acceptance_criteria",
                                "acceptanceCriteria",
                                "acceptance-criteria",
                            ],
                        )?
                        .unwrap_or_default(),
                        dependencies: optional_csv_or_string_list_alias(&input, &["dependencies"])?
                            .unwrap_or_default(),
                        plan,
                        comment: optional_string(&input, "comment")?,
                        context_files: optional_csv_or_string_list_alias(
                            &input,
                            &["context_files", "context"],
                        )?
                        .unwrap_or_default(),
                        workspace_path: Some(workspace),
                        priority: optional_string(&input, "priority")?
                            .map(|value| parse_task_priority("priority", &value))
                            .transpose()?
                            .unwrap_or(TaskPriority::Medium),
                        complexity: optional_string(&input, "complexity")?
                            .map(|value| parse_task_complexity("complexity", &value))
                            .transpose()?,
                        task_type: optional_string_alias(
                            &input,
                            &["type", "task_type", "taskType"],
                        )?
                        .map(|value| parse_task_type("type", &value))
                        .transpose()?,
                        status: optional_string(&input, "status")?
                            .map(|value| parse_task_status("status", &value))
                            .transpose()?,
                        system_created: false,
                        source_task_id: optional_string_alias(
                            &input,
                            &["source_task_id", "source_task", "sourceTaskId"],
                        )?,
                    },
                    agent,
                    model,
                )?;
                serialize_task(&self.runtime, &task)
            }
            OrbitBuiltinAction::TaskApprove => {
                let id = required_string(&input, &["id"], "id")?;
                let task = self.runtime.approve_task_with_identity(
                    &id,
                    optional_string(&input, "note")?,
                    optional_string(&input, "comment")?,
                    agent,
                    model,
                )?;
                serialize_task(&self.runtime, &task)
            }
            OrbitBuiltinAction::TaskDelete => {
                let id = required_string(&input, &["id"], "id")?;
                self.runtime.delete_task(&id)?;
                Ok(json!({ "id": id, "deleted": true }))
            }
            OrbitBuiltinAction::TaskLint => {
                let id = required_string(&input, &["id"], "id")?;
                serialize_task_lint_report(&self.runtime.lint_task(&id)?)
            }
            OrbitBuiltinAction::TaskList => {
                let status = optional_string(&input, "status")?
                    .map(|value| parse_task_status("status", &value))
                    .transpose()?;
                let task_type = optional_string_alias(&input, &["type", "task_type", "taskType"])?
                    .map(|value| parse_task_type("type", &value))
                    .transpose()?;
                let parent_id =
                    optional_string_alias(&input, &["parent_id", "parent", "parentId"])?;
                let batch_id = optional_string(&input, "batch_id")?;
                let ready = optional_bool_alias(&input, &["ready"])?;
                let all_tasks = self.runtime.list_tasks()?;
                let status_by_id = build_task_status_index(&all_tasks);
                let tasks = all_tasks
                    .into_iter()
                    .filter(|task| status.is_none_or(|value| task.status == value))
                    .filter(|task| {
                        parent_id
                            .as_deref()
                            .is_none_or(|value| task.parent_id.as_deref() == Some(value))
                    })
                    .filter(|task| {
                        batch_id
                            .as_deref()
                            .is_none_or(|value| task.batch_id.as_deref() == Some(value))
                    })
                    .filter(|task| {
                        ready != Some(true) || task_dependencies_ready(task, &status_by_id)
                    })
                    .collect::<Vec<_>>();
                Ok(Value::Array(
                    tasks
                        .into_iter()
                        .filter(|task| task_type.is_none_or(|kind| task.task_type == kind))
                        .map(|task| task_to_json(&task, &status_by_id))
                        .collect::<Vec<_>>(),
                ))
            }
            OrbitBuiltinAction::TaskSearch => {
                let query = required_string(&input, &["query"], "query")?;
                let status_by_id = build_task_status_index(&self.runtime.list_tasks()?);
                let tasks = self.runtime.search_tasks(&query)?;
                Ok(Value::Array(
                    tasks
                        .into_iter()
                        .map(|task| task_to_json(&task, &status_by_id))
                        .collect::<Vec<_>>(),
                ))
            }
            OrbitBuiltinAction::TaskLocks => {
                let mut tasks: Vec<_> = self
                    .runtime
                    .list_tasks()?
                    .into_iter()
                    .filter(|task| {
                        matches!(task.status, TaskStatus::InProgress | TaskStatus::Review)
                    })
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
                    .flat_map(|task| existing_context_files(&self.runtime, task))
                    .collect();

                Ok(json!({
                    "locked_files": locked_files.iter().cloned().collect::<Vec<_>>(),
                    "by_task": tasks.iter().map(task_lock_to_json).collect::<Vec<_>>(),
                    "total_locked": locked_files.len(),
                    "total_tasks": tasks.len(),
                }))
            }
            OrbitBuiltinAction::TaskLocksRelease => {
                let reservation_id = required_string(
                    &input,
                    &["reservation_id", "reservationId", "reservation-id"],
                    "reservation_id",
                )?;
                let result = self.runtime.stores().task_reservations().release(
                    TaskReservationReleaseParams {
                        workspace_orbit_dir: workspace_orbit_dir(&self.runtime),
                        reservation_id: reservation_id.clone(),
                    },
                )?;
                emit_expired_reservation_events(&self.runtime, &result.expired_reservations)?;
                if result.released {
                    record_task_lock_audit_event(
                        &self.runtime,
                        "task.locks.reserve.released",
                        "orbit.task.locks.release",
                        Some(reservation_id.as_str()),
                        AuditEventStatus::Success,
                        json!({
                            "reservation_id": reservation_id,
                            "released_at": result.released_at,
                            "released_by": reservation_actor_label(
                                &self.runtime,
                                agent.as_deref(),
                                model.as_deref(),
                            ),
                        }),
                    )?;
                }
                Ok(json!({ "released": result.released }))
            }
            OrbitBuiltinAction::TaskLocksReserve => {
                let task_ids = parse_task_ids(&input)?;
                let ttl_seconds =
                    optional_u32_alias(&input, &["ttl_seconds", "ttlSeconds", "ttl-seconds"])?
                        .unwrap_or(1800);
                if !(1..=7200).contains(&ttl_seconds) {
                    return Err(OrbitError::InvalidInput(
                        "`ttl_seconds` must be between 1 and 7200 seconds".to_string(),
                    ));
                }

                let actor =
                    reservation_actor_label(&self.runtime, agent.as_deref(), model.as_deref());
                let requested_files = requested_task_files(&self.runtime, &task_ids)?;
                let mut conflicts =
                    task_lock_conflicts(&self.runtime, &task_ids, &requested_files)?;

                record_task_lock_audit_event(
                    &self.runtime,
                    "task.locks.reserve.requested",
                    "orbit.task.locks.reserve",
                    None,
                    AuditEventStatus::Success,
                    json!({
                        "actor": actor.clone(),
                        "task_ids": task_ids.clone(),
                        "ttl_seconds": ttl_seconds,
                    }),
                )?;

                let reservation_result = if conflicts.is_empty() {
                    self.runtime.stores().task_reservations().reserve(
                        TaskReservationReserveParams {
                            workspace_orbit_dir: workspace_orbit_dir(&self.runtime),
                            task_ids: task_ids.clone(),
                            requested_files: requested_files.clone(),
                            actor: actor.clone(),
                            ttl_seconds,
                        },
                    )?
                } else {
                    let check = self.runtime.stores().task_reservations().check(
                        TaskReservationCheckParams {
                            workspace_orbit_dir: workspace_orbit_dir(&self.runtime),
                            requested_files: requested_files.clone(),
                        },
                    )?;
                    conflicts = merge_task_lock_conflicts(conflicts, check.conflicts);
                    emit_expired_reservation_events(&self.runtime, &check.expired_reservations)?;
                    orbit_store::TaskReservationReserveResult {
                        reserved: false,
                        reservation_id: None,
                        expires_at: None,
                        reserved_files: Vec::new(),
                        conflicts: conflicts.clone(),
                        expired_reservations: Vec::new(),
                    }
                };

                emit_expired_reservation_events(
                    &self.runtime,
                    &reservation_result.expired_reservations,
                )?;

                if reservation_result.reserved {
                    let reservation_id =
                        reservation_result.reservation_id.clone().ok_or_else(|| {
                            OrbitError::Execution(
                                "reservation grant is missing reservation_id".to_string(),
                            )
                        })?;
                    record_task_lock_audit_event(
                        &self.runtime,
                        "task.locks.reserve.granted",
                        "orbit.task.locks.reserve",
                        Some(reservation_id.as_str()),
                        AuditEventStatus::Success,
                        json!({
                            "reservation_id": reservation_id,
                            "files": reservation_result.reserved_files.clone(),
                            "expires_at": reservation_result.expires_at.clone(),
                            "actor": actor,
                        }),
                    )?;
                    Ok(json!({
                        "reserved": true,
                        "reservation_id": reservation_result.reservation_id,
                        "expires_at": reservation_result.expires_at,
                        "reserved_files": reservation_result.reserved_files,
                    }))
                } else {
                    let conflicts =
                        merge_task_lock_conflicts(conflicts, reservation_result.conflicts);
                    record_task_lock_audit_event(
                        &self.runtime,
                        "task.locks.reserve.denied",
                        "orbit.task.locks.reserve",
                        None,
                        AuditEventStatus::Denied,
                        json!({
                            "actor": actor,
                            "task_ids": task_ids.clone(),
                            "conflicts": conflicts.clone(),
                        }),
                    )?;
                    Ok(json!({
                        "reserved": false,
                        "conflicts": conflicts,
                    }))
                }
            }
            OrbitBuiltinAction::TaskReject => {
                let id = required_string(&input, &["id"], "id")?;
                let note = required_string(&input, &["note"], "note")?;
                let task = self.runtime.reject_task_with_identity(
                    &id,
                    note,
                    optional_string(&input, "comment")?,
                    agent,
                    model,
                )?;
                serialize_task(&self.runtime, &task)
            }
            OrbitBuiltinAction::TaskShow => {
                let id = required_string(&input, &["id"], "id")?;
                let task = self.runtime.get_task(&id)?;
                let fields = optional_csv_or_string_list_alias(&input, &["fields", "field"])?;
                if let Some(fields) = fields {
                    task_fields_to_json(&self.runtime, &task, &fields)
                } else {
                    serialize_task(&self.runtime, &task)
                }
            }
            OrbitBuiltinAction::TaskStart => {
                let id = required_string(&input, &["id"], "id")?;
                let task = self.runtime.start_task_with_identity(
                    &id,
                    optional_string(&input, "note")?,
                    optional_string(&input, "comment")?,
                    agent,
                    model,
                )?;
                serialize_task(&self.runtime, &task)
            }
            OrbitBuiltinAction::TaskUpdate => {
                let id = required_string(&input, &["id"], "id")?;
                let task = self.runtime.update_task_with_identity(
                    &id,
                    TaskUpdateParams {
                        title: optional_string(&input, "title")?,
                        description: input
                            .get("description")
                            .map(|value| {
                                value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                                    OrbitError::InvalidInput(
                                        "`description` must be a string".to_string(),
                                    )
                                })
                            })
                            .transpose()?,
                        acceptance_criteria: optional_string_list_alias(
                            &input,
                            &[
                                "acceptance_criteria",
                                "acceptanceCriteria",
                                "acceptance-criteria",
                            ],
                        )?,
                        dependencies: optional_csv_or_string_list_alias(&input, &["dependencies"])?,
                        plan: input
                            .get("plan")
                            .map(|value| {
                                value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                                    OrbitError::InvalidInput("`plan` must be a string".to_string())
                                })
                            })
                            .transpose()?,
                        execution_summary: optional_raw_string(&input, "execution_summary")?,
                        comment: optional_string(&input, "comment")?,
                        status: optional_string(&input, "status")?
                            .map(|value| parse_task_status("status", &value))
                            .transpose()?,
                        planned_by: optional_raw_string(&input, "planned_by")?
                            .map(empty_string_to_none),
                        implemented_by: optional_raw_string(&input, "implemented_by")?
                            .map(empty_string_to_none),
                        pr_number: optional_raw_string(&input, "pr_number")?
                            .map(empty_string_to_none),
                        pr_status: optional_raw_string(&input, "pr_status")?
                            .map(empty_string_to_none),
                        batch_id: optional_raw_string(&input, "batch_id")?
                            .map(empty_string_to_none),
                        context_files: optional_csv_or_string_list_alias(
                            &input,
                            &["context_files", "context"],
                        )?,
                        upsert_artifacts: parse_artifacts(&input)?,
                        ..Default::default()
                    },
                    agent,
                    model,
                )?;
                serialize_task(&self.runtime, &task)
            }
        }
    }

    fn task_scope(&self) -> OrbitTaskScope {
        self.task_scope.clone()
    }

    fn task_id_pattern(&self) -> Option<String> {
        self.runtime.task_id_pattern().map(ToString::to_string)
    }
}

fn require_review_model(model: Option<&str>, tool_name: &str) -> Result<(), OrbitError> {
    let resolved = model.map(str::trim).filter(|value| !value.is_empty());
    if resolved.is_none() {
        return Err(OrbitError::InvalidInput(format!(
            "{tool_name} requires `model`. Pass the calling agent's model identifier (e.g. `gpt-5.5`, `claude-opus-4-7`); pass `human` for human-authored review feedback to opt out of scoring."
        )));
    }
    Ok(())
}

pub(crate) fn parse_task_ids(input: &Value) -> Result<Vec<String>, OrbitError> {
    let task_ids = optional_string_list_alias(input, &["task_ids", "taskIds", "task-ids"])?
        .ok_or_else(|| OrbitError::InvalidInput("missing `task_ids`".to_string()))?;
    let deduped = task_ids.into_iter().collect::<BTreeSet<_>>();
    if deduped.is_empty() {
        return Err(OrbitError::InvalidInput(
            "`task_ids` must contain at least one task ID".to_string(),
        ));
    }
    Ok(deduped.into_iter().collect())
}

pub(crate) fn workspace_orbit_dir(runtime: &OrbitRuntime) -> String {
    runtime.paths().orbit_dir.to_string_lossy().into_owned()
}

fn task_workspace_root(runtime: &OrbitRuntime, task: &Task) -> PathBuf {
    task.workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                runtime.paths().repo_root.join(path)
            }
        })
        .unwrap_or_else(|| runtime.paths().repo_root.clone())
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
            .ok_or_else(|| OrbitError::TaskNotFound(task_id.clone()))?;
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
            AuditEventStatus::Success,
            json!({
                "reservation_id": expired.reservation_id,
                "expired_at": expired.expired_at,
            }),
        )?;
    }
    Ok(())
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
    status: AuditEventStatus,
    payload: Value,
) -> Result<(), OrbitError> {
    runtime.record_audit_event(&crate::AuditEventInsertParams {
        execution_id: format!(
            "audit-{}-{}",
            command.replace('.', "-"),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ),
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
        task_id: target_id.map(ToOwned::to_owned),
        job_run_id: std::env::var("ORBIT_RUN_ID").ok().filter(|s| !s.is_empty()),
        activity_id: std::env::var("ORBIT_ACTIVITY_ID")
            .ok()
            .filter(|s| !s.is_empty()),
        step_index: std::env::var("ORBIT_STEP_INDEX")
            .ok()
            .and_then(|s| s.parse().ok()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use orbit_common::types::TaskType;
    use orbit_store::TaskCreateParams;
    use serde_json::json;
    use tempfile::tempdir;

    fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf) {
        let root = tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("create global root");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
        (root, runtime, repo_root)
    }

    fn create_task(
        runtime: &OrbitRuntime,
        workspace_path: &std::path::Path,
        title: &str,
        description: &str,
        status: TaskStatus,
        context_files: &[&str],
    ) -> Task {
        runtime
            .stores()
            .tasks()
            .create(TaskCreateParams {
                actor: "test".to_string(),
                parent_id: None,
                title: title.to_string(),
                description: description.to_string(),
                acceptance_criteria: Vec::new(),
                dependencies: Vec::new(),
                plan: String::new(),
                execution_summary: String::new(),
                context_files: context_files
                    .iter()
                    .map(|path| (*path).to_string())
                    .collect(),
                workspace_path: Some(workspace_path.to_string_lossy().into_owned()),
                repo_root: None,
                created_by: Some("test".to_string()),
                planned_by: None,
                implemented_by: None,
                agent: None,
                model: None,
                status,
                priority: TaskPriority::Medium,
                complexity: None,
                task_type: TaskType::Task,
                pr_number: None,
                source_task_id: None,
                comments: Vec::new(),
            })
            .expect("create task")
    }

    fn create_context_task(
        runtime: &OrbitRuntime,
        workspace_path: &std::path::Path,
        status: TaskStatus,
        context_files: &[&str],
    ) -> Task {
        create_task(
            runtime,
            workspace_path,
            "test task",
            "test",
            status,
            context_files,
        )
    }

    #[test]
    fn requested_task_files_prune_missing_context_entries() {
        let (_root, runtime, repo_root) = test_runtime();
        std::fs::create_dir_all(repo_root.join("docs/design")).expect("create docs dir");
        std::fs::write(repo_root.join("docs/design/groundhog.md"), "alias")
            .expect("write alias doc");

        let task = create_context_task(
            &runtime,
            &repo_root,
            TaskStatus::Backlog,
            &["docs/design/groundhog.md", "docs/design/missing.md"],
        );

        let requested =
            requested_task_files(&runtime, &[task.id]).expect("collect requested task files");
        assert_eq!(requested, vec!["file:docs/design/groundhog.md".to_string()]);
    }

    #[test]
    fn task_lock_conflicts_ignore_missing_held_context_entries() {
        let (_root, runtime, repo_root) = test_runtime();
        std::fs::create_dir_all(repo_root.join("src")).expect("create src dir");
        std::fs::write(repo_root.join("src/lib.rs"), "pub fn ok() {}\n")
            .expect("write source file");

        let holder = create_context_task(
            &runtime,
            &repo_root,
            TaskStatus::InProgress,
            &["docs/design/groundhog.md", "src/lib.rs"],
        );

        let conflicts = task_lock_conflicts(
            &runtime,
            &[],
            &[
                "docs/design/groundhog.md".to_string(),
                "src/lib.rs".to_string(),
            ],
        )
        .expect("compute task lock conflicts");

        assert_eq!(
            conflicts,
            vec![TaskLockConflict {
                file: "src/lib.rs".to_string(),
                held_by: TaskLockHolder::Task,
                held_by_id: holder.id,
            }]
        );
    }

    #[test]
    fn task_lock_conflicts_use_selector_anchor_overlap() {
        let (_root, runtime, repo_root) = test_runtime();
        std::fs::create_dir_all(repo_root.join("src")).expect("create src dir");
        std::fs::write(repo_root.join("src/lib.rs"), "pub fn ok() {}\n")
            .expect("write source file");

        let holder = create_context_task(
            &runtime,
            &repo_root,
            TaskStatus::InProgress,
            &["symbol:src/lib.rs#ok:function"],
        );

        let conflicts = task_lock_conflicts(
            &runtime,
            &[],
            &["file:src/lib.rs".to_string(), "dir:src".to_string()],
        )
        .expect("compute selector-aware task lock conflicts");

        assert_eq!(
            conflicts,
            vec![
                TaskLockConflict {
                    file: "dir:src".to_string(),
                    held_by: TaskLockHolder::Task,
                    held_by_id: holder.id.clone(),
                },
                TaskLockConflict {
                    file: "file:src/lib.rs".to_string(),
                    held_by: TaskLockHolder::Task,
                    held_by_id: holder.id,
                },
            ]
        );
    }

    #[test]
    fn execute_tool_command_searches_tasks_for_agents() {
        let (_root, runtime, repo_root) = test_runtime();
        let title_match = create_task(
            &runtime,
            &repo_root,
            "Fix search surface",
            "Wire the tool through Orbit.",
            TaskStatus::Backlog,
            &[],
        );
        let description_match = create_task(
            &runtime,
            &repo_root,
            "Refactor task queries",
            "Preserve SEARCH parity for agents.",
            TaskStatus::Review,
            &[],
        );
        create_task(
            &runtime,
            &repo_root,
            "Unrelated maintenance",
            "Nothing to see here.",
            TaskStatus::Backlog,
            &[],
        );

        let output = runtime
            .execute_tool_command(
                "orbit.task.search",
                json!({ "query": "sEaRcH" }),
                Some("codex".to_string()),
                Some("gpt-5.4".to_string()),
            )
            .expect("search tool succeeds");

        let matches = output.as_array().expect("search returns an array");
        let ids = matches
            .iter()
            .filter_map(|task| task.get("id").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&title_match.id.as_str()));
        assert!(ids.contains(&description_match.id.as_str()));
    }

    #[test]
    fn task_add_tool_creates_proposed_tasks_for_agents() {
        let (_root, runtime, _repo_root) = test_runtime();

        let output = runtime
            .execute_tool_command(
                "orbit.task.add",
                json!({
                    "title": "Propose task from tool",
                    "description": "Exercise the agent-facing task creation path.",
                    "workspace": ".",
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("task add tool succeeds");

        assert_eq!(
            output.get("status").and_then(Value::as_str),
            Some("proposed")
        );
    }

    #[test]
    fn task_add_tool_persists_dependencies() {
        let (_root, runtime, repo_root) = test_runtime();
        let dependency = create_task(
            &runtime,
            &repo_root,
            "Dependency task",
            "Existing task that must finish first.",
            TaskStatus::Backlog,
            &[],
        );

        let output = runtime
            .execute_tool_command(
                "orbit.task.add",
                json!({
                    "title": "Dependent task from tool",
                    "description": "Exercise dependency input on the agent-facing task creation path.",
                    "workspace": ".",
                    "dependencies": [dependency.id.clone()],
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("task add tool succeeds");

        assert_eq!(
            output.get("dependencies"),
            Some(&json!([dependency.id.as_str()]))
        );
    }

    #[test]
    fn task_add_tool_infers_agent_from_model_only_input() {
        let (_root, runtime, _repo_root) = test_runtime();

        let output = runtime
            .execute_tool_command(
                "orbit.task.add",
                json!({
                    "title": "Propose model-only task",
                    "description": "Exercise model-first provenance.",
                    "workspace": ".",
                    "model": "gpt-5.5",
                }),
                None,
                None,
            )
            .expect("task add tool succeeds");

        assert_eq!(output.get("agent").and_then(Value::as_str), Some("codex"));
        assert_eq!(output.get("model").and_then(Value::as_str), Some("gpt-5.5"));
        assert_eq!(
            output.get("created_by").and_then(Value::as_str),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn task_update_tool_infers_agent_from_model_only_input() {
        let (_root, runtime, repo_root) = test_runtime();
        let task = create_task(
            &runtime,
            &repo_root,
            "Update model-only task",
            "Exercise model-first update provenance.",
            TaskStatus::Backlog,
            &[],
        );

        let output = runtime
            .execute_tool_command(
                "orbit.task.update",
                json!({
                    "id": task.id,
                    "comment": "record model-only update",
                    "model": "gemini-3.1-pro-preview",
                }),
                None,
                None,
            )
            .expect("task update tool succeeds");

        assert_eq!(output.get("agent").and_then(Value::as_str), Some("gemini"));
        assert_eq!(
            output.get("model").and_then(Value::as_str),
            Some("gemini-3.1-pro-preview")
        );
    }

    #[test]
    fn task_update_tool_replaces_dependencies() {
        let (_root, runtime, repo_root) = test_runtime();
        let first_dependency = create_task(
            &runtime,
            &repo_root,
            "First dependency",
            "Existing task that must finish first.",
            TaskStatus::Backlog,
            &[],
        );
        let second_dependency = create_task(
            &runtime,
            &repo_root,
            "Second dependency",
            "Replacement prerequisite.",
            TaskStatus::Backlog,
            &[],
        );
        let task = create_task(
            &runtime,
            &repo_root,
            "Update dependency task",
            "Exercise dependency replacement through tool input.",
            TaskStatus::Backlog,
            &[],
        );

        let output = runtime
            .execute_tool_command(
                "orbit.task.update",
                json!({
                    "id": task.id.clone(),
                    "dependencies": [first_dependency.id.clone()],
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("task update tool sets dependency");

        assert_eq!(
            output.get("dependencies"),
            Some(&json!([first_dependency.id.as_str()]))
        );

        let output = runtime
            .execute_tool_command(
                "orbit.task.update",
                json!({
                    "id": task.id,
                    "dependencies": [second_dependency.id.clone()],
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("task update tool replaces dependency");

        assert_eq!(
            output.get("dependencies"),
            Some(&json!([second_dependency.id.as_str()]))
        );
    }

    #[test]
    fn task_update_tool_allows_explicit_attribution_updates() {
        let (_root, runtime, repo_root) = test_runtime();
        let task = create_task(
            &runtime,
            &repo_root,
            "Update explicit attribution",
            "Exercise explicit provenance correction.",
            TaskStatus::Backlog,
            &[],
        );

        let output = runtime
            .execute_tool_command(
                "orbit.task.update",
                json!({
                    "id": task.id.clone(),
                    "planned_by": "manual-planner",
                    "implemented_by": "manual-implementer",
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("task update tool succeeds");

        assert_eq!(
            output.get("planned_by").and_then(Value::as_str),
            Some("manual-planner")
        );
        assert_eq!(
            output.get("implemented_by").and_then(Value::as_str),
            Some("manual-implementer")
        );

        let output = runtime
            .execute_tool_command(
                "orbit.task.update",
                json!({
                    "id": task.id,
                    "planned_by": "",
                    "implemented_by": "",
                }),
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("task update tool clears attribution");

        assert_eq!(output.get("planned_by"), Some(&Value::Null));
        assert_eq!(output.get("implemented_by"), Some(&Value::Null));
    }

    #[test]
    fn task_update_tool_explicit_implemented_by_overrides_review_stamp() {
        let (_root, runtime, repo_root) = test_runtime();
        let task = create_task(
            &runtime,
            &repo_root,
            "Review explicit attribution",
            "Exercise explicit provenance correction on review transition.",
            TaskStatus::InProgress,
            &[],
        );

        let output = runtime
            .execute_tool_command(
                "orbit.task.update",
                json!({
                    "id": task.id,
                    "status": "review",
                    "execution_summary": "Implemented and validated.",
                    "implemented_by": "manual-implementer",
                    "model": "gemini-3.1-pro-preview",
                }),
                None,
                None,
            )
            .expect("task update tool succeeds");

        assert_eq!(output.get("status").and_then(Value::as_str), Some("review"));
        assert_eq!(
            output.get("implemented_by").and_then(Value::as_str),
            Some("manual-implementer")
        );
    }

    #[test]
    fn task_tool_rejects_mismatched_agent_and_model() {
        let (_root, runtime, _repo_root) = test_runtime();

        let error = runtime
            .execute_tool_command(
                "orbit.task.add",
                json!({
                    "title": "Reject mismatched identity",
                    "description": "Exercise explicit mismatch validation.",
                    "workspace": ".",
                    "agent": "claude",
                    "model": "gpt-5.5",
                }),
                None,
                None,
            )
            .expect_err("mismatched identity should fail");

        assert!(error.to_string().contains("does not match `model`"));
    }

    #[test]
    fn review_thread_add_rejects_missing_model() {
        let (_root, runtime, repo_root) = test_runtime();
        let task = create_task(
            &runtime,
            &repo_root,
            "Review thread missing model",
            "Exercise required-model enforcement.",
            TaskStatus::Review,
            &[],
        );

        let error = runtime
            .execute_tool_command(
                "orbit.task.review_thread.add",
                json!({
                    "id": task.id,
                    "body": "Review feedback.",
                }),
                None,
                None,
            )
            .expect_err("missing model should fail");

        assert!(matches!(error, OrbitError::InvalidInput(_)));
        assert!(
            error
                .to_string()
                .contains("orbit.task.review_thread.add requires `model`")
        );
    }

    #[test]
    fn review_thread_add_rejects_empty_model() {
        let (_root, runtime, repo_root) = test_runtime();
        let task = create_task(
            &runtime,
            &repo_root,
            "Review thread empty model",
            "Exercise required-model enforcement on empty input.",
            TaskStatus::Review,
            &[],
        );

        let error = runtime
            .execute_tool_command(
                "orbit.task.review_thread.add",
                json!({
                    "id": task.id,
                    "body": "Review feedback.",
                    "model": "   ",
                }),
                None,
                None,
            )
            .expect_err("empty model should fail");

        assert!(
            matches!(error, OrbitError::InvalidInput(_)),
            "expected InvalidInput, got: {error}"
        );
        assert!(
            error.to_string().contains("model"),
            "error should mention model: {error}"
        );
    }

    #[test]
    fn review_thread_reply_rejects_missing_model() {
        let (_root, runtime, repo_root) = test_runtime();
        let task = create_task(
            &runtime,
            &repo_root,
            "Reply missing model",
            "Exercise required-model enforcement on reply.",
            TaskStatus::Review,
            &[],
        );

        let thread = runtime
            .add_review_thread(
                &task.id,
                "Initial review.".to_string(),
                None,
                None,
                Some("codex".to_string()),
                Some("gpt-5.5".to_string()),
            )
            .expect("add review thread");

        let error = runtime
            .execute_tool_command(
                "orbit.task.review_thread.reply",
                json!({
                    "id": task.id,
                    "thread_id": thread.thread_id,
                    "body": "Reply.",
                }),
                None,
                None,
            )
            .expect_err("missing model should fail");

        assert!(matches!(error, OrbitError::InvalidInput(_)));
        assert!(
            error
                .to_string()
                .contains("orbit.task.review_thread.reply requires `model`")
        );
    }

    #[test]
    fn review_thread_add_accepts_human_model() {
        let (_root, runtime, repo_root) = test_runtime();
        let task = create_task(
            &runtime,
            &repo_root,
            "Human review accepted",
            "Exercise that explicit `model: human` opts out of scoring without erroring.",
            TaskStatus::Review,
            &[],
        );

        let output = runtime
            .execute_tool_command(
                "orbit.task.review_thread.add",
                json!({
                    "id": task.id,
                    "body": "Human review feedback.",
                    "model": "human",
                }),
                None,
                None,
            )
            .expect("human-attributed review should succeed");

        assert_eq!(
            output.get("id").and_then(Value::as_str),
            Some(task.id.as_str())
        );
    }
}
