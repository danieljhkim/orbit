//! `impl V2RuntimeHost for OrbitRuntime` — the orbit-core side of the v2
//! dispatch boundary.
//!
//! The trait surface is deliberately small: orbit-core owns deterministic
//! action dispatch (which needs the live `ToolContext` + tool registry),
//! provider credential sourcing (env / config access), and the CLI-command
//! resolution for `backend: cli` (workspace-scoped env / config overrides).
//! HTTP agent-loop transport and CLI subprocess execution both live in
//! `orbit-engine`, so this module never names orbit-agent types.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use orbit_common::types::activity_job::AgentRole;
use orbit_common::types::{
    AuditEventStatus, ExecutorSandboxKind, ExecutorType, ResolvedFsProfile, Role, TaskStatus,
    TaskType, UNRESTRICTED_FS_PROFILE, prune_missing_context_files,
};
use orbit_common::types::{InvocationTrace, Task};
use orbit_common::utility::path::workspace_relative_paths_overlap;
use orbit_common::utility::selector::canonical_selector_in_workspace;
use orbit_engine::activity_job::{
    DispatchError, ResolvedCliExecutor, ResolvedSandbox, V2RuntimeHost,
};
use orbit_engine::{
    AgentRoleConfig, EnvironmentHost, StateExecutionContext, execute_deterministic_action,
};
use orbit_store::{AuditEventInsertParams, InvocationInsertParams, Store, token_scoreboard};
use orbit_tools::{FsAuditLogger, ToolContext};
use serde::Serialize;
use serde_json::Value;

use super::orbit_tool_host::{
    emit_expired_reservation_events, merge_task_lock_conflicts, parse_task_ids,
    requested_task_files, task_lock_conflicts, workspace_orbit_dir,
};
use crate::OrbitRuntime;
use crate::command::task::{canonicalize_context_files_for_read, context_workspace_root};
use crate::runtime::build_orbit_tool_host;

impl V2RuntimeHost for OrbitRuntime {
    fn run_deterministic(
        &self,
        action: &str,
        config: &Value,
        input: &Value,
        tool_context: ToolContext,
    ) -> Result<Value, DispatchError> {
        match action {
            "orbit_tool_call" => {
                // The `config` block shape (see deterministic_reference.yaml):
                //   config: { tool_name: <name>, args: <object> }
                // Input overrides config when both are present.
                let tool_name = input
                    .get("tool_name")
                    .or_else(|| config.get("tool_name"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: "missing `tool_name` in config or input".to_string(),
                    })?;
                let args = input
                    .get("args")
                    .or_else(|| config.get("args"))
                    .cloned()
                    .unwrap_or(Value::Null);

                self.run_tool_with_context_and_role(tool_name, args, Role::Admin, tool_context)
                    .map_err(|err| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: format!("{err}"),
                    })
            }
            "git_commit" | "git_merge" | "git_push" | "pr_open" | "run_planning_duel"
            | "update_task" | "worktree_setup" => execute_deterministic_action(
                self,
                action,
                input,
                false,
                &HashMap::new(),
                Option::<&StateExecutionContext>::None,
            )
            .map_err(|err| DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!("{err}"),
            }),
            // Phase 4 stub handlers. Real git/API logic lands in a follow-up
            // task once the per-asset migration ports the rest of the
            // pipeline dependencies (worktree_setup, pr_open, pr_merge, …).
            // Returning a structured result keeps the activities dispatchable
            // so the §7 `activity.started` / `activity.finished` envelope is
            // emitted end-to-end — an operator running the pipeline today
            // sees the intent even while the implementation is stubbed.
            "promote_agent_main" => {
                let target = input
                    .get("target_branch")
                    .and_then(Value::as_str)
                    .unwrap_or("main");
                let source = input
                    .get("source_branch")
                    .and_then(Value::as_str)
                    .unwrap_or("agent-main");
                Ok(serde_json::json!({
                    "promoted": false,
                    "target_sha": null,
                    "skipped_reason":
                        format!("stub: real promotion from `{source}` to `{target}` lands in a follow-up"),
                }))
            }
            "revert_on_red" => {
                let sha = input
                    .get("commit_sha")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                Ok(serde_json::json!({
                    "reverted": false,
                    "revert_sha": null,
                    "follow_up_issue": null,
                    "skipped_reason":
                        format!("stub: real revert of `{sha}` lands in a follow-up"),
                }))
            }
            "context_conflict_check" => {
                let task_ids = parse_task_ids(input).map_err(|error| {
                    DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: error.to_string(),
                    }
                })?;
                let requested_files = requested_task_files(self, &task_ids).map_err(|error| {
                    DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: error.to_string(),
                    }
                })?;
                let task_conflicts = task_lock_conflicts(self, &task_ids, &requested_files)
                    .map_err(|error| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: error.to_string(),
                    })?;
                let reservation_check = self
                    .stores()
                    .task_reservations()
                    .check(orbit_store::TaskReservationCheckParams {
                        workspace_orbit_dir: workspace_orbit_dir(self),
                        requested_files,
                    })
                    .map_err(|error| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: error.to_string(),
                    })?;
                emit_expired_reservation_events(self, &reservation_check.expired_reservations)
                    .map_err(|error| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: error.to_string(),
                    })?;
                let conflicts =
                    merge_task_lock_conflicts(task_conflicts, reservation_check.conflicts);
                Ok(serde_json::json!({
                    "clear": conflicts.is_empty(),
                    "conflicts": conflicts,
                }))
            }
            "sleep" => {
                let seconds = input
                    .get("seconds")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: "missing `seconds`".to_string(),
                    })?;
                if !(0.0..=3600.0).contains(&seconds) {
                    return Err(DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: "`seconds` must be between 0 and 3600".to_string(),
                    });
                }
                let started_at = Instant::now();
                std::thread::sleep(Duration::from_secs_f64(seconds));
                Ok(serde_json::json!({
                    "slept_seconds": started_at.elapsed().as_secs_f64(),
                }))
            }
            // Materialize the workspace backlog for auto-dispatch.
            // Filters by `status: backlog`; accepted friction reports keep
            // `type: friction` and ship like other backlog tasks, while
            // untriaged `status: friction` reports remain absent. In automatic
            // mode, drops any backlog task group whose context overlaps files
            // already held by `in-progress`/`review` tasks. Sorts critical →
            // high → medium → low then by `created_at` ascending so older
            // high-priority work ships first. Caps at `max_tasks` (default 50).
            "list_backlog_tasks" => {
                let max_tasks = input
                    .get("max_tasks")
                    .and_then(Value::as_u64)
                    .unwrap_or(50)
                    .min(500) as usize;
                let explicit_task_ids: Vec<String> = input
                    .get("task_ids")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let (mut tasks, excluded_entries) = if explicit_task_ids.is_empty() {
                    let all_tasks = self.stores().tasks().list().map_err(|err| {
                        DispatchError::DeterministicActionFailed {
                            action: action.to_string(),
                            message: format!("list tasks: {err}"),
                        }
                    })?;
                    let task_lookup: BTreeMap<String, Task> = all_tasks
                        .iter()
                        .cloned()
                        .map(|task| (task.id.clone(), task))
                        .collect();
                    let lock_holders = active_task_lock_holders(task_lookup.values());
                    let mut backlog: Vec<Task> = all_tasks
                        .into_iter()
                        .filter(|task| task.status == TaskStatus::Backlog)
                        .collect();
                    backlog.sort_by(|a, b| {
                        let rank = |p: orbit_common::types::TaskPriority| match p {
                            orbit_common::types::TaskPriority::Critical => 0,
                            orbit_common::types::TaskPriority::High => 1,
                            orbit_common::types::TaskPriority::Medium => 2,
                            orbit_common::types::TaskPriority::Low => 3,
                        };
                        rank(a.priority)
                            .cmp(&rank(b.priority))
                            .then(a.created_at.cmp(&b.created_at))
                    });
                    let mut excluded = Vec::new();
                    if !lock_holders.is_empty() {
                        let direct_conflicts: BTreeMap<String, Vec<BacklogTaskConflict>> = backlog
                            .iter()
                            .filter_map(|task| {
                                let conflicts = task_overlap_conflicts(task, &lock_holders);
                                (!conflicts.is_empty()).then(|| (task.id.clone(), conflicts))
                            })
                            .collect();
                        let mut root_trigger: BTreeMap<String, Vec<BacklogTaskConflict>> =
                            BTreeMap::new();
                        for task in &backlog {
                            if let Some(conflicts) = direct_conflicts.get(&task.id) {
                                let root_id = task_root_id(task, &task_lookup);
                                // Backlog is already priority/age sorted; the first direct
                                // conflict in that order supplies group-member attribution.
                                root_trigger
                                    .entry(root_id)
                                    .or_insert_with(|| conflicts.clone());
                            }
                        }
                        if !root_trigger.is_empty() {
                            let mut kept = Vec::new();
                            for task in backlog {
                                let root_id = task_root_id(&task, &task_lookup);
                                if let Some(trigger_conflicts) = root_trigger.get(&root_id) {
                                    if let Some(conflicts) = direct_conflicts.get(&task.id) {
                                        excluded.push(BacklogTaskExclusion {
                                            id: task.id.clone(),
                                            reason: BacklogTaskExclusionReason::ContextLockConflict,
                                            conflicts: conflicts.clone(),
                                        });
                                    } else {
                                        excluded.push(BacklogTaskExclusion {
                                            id: task.id.clone(),
                                            reason: BacklogTaskExclusionReason::GroupMemberConflict,
                                            conflicts: trigger_conflicts.clone(),
                                        });
                                    }
                                } else {
                                    kept.push(task);
                                }
                            }
                            excluded.sort_by(|a, b| a.id.cmp(&b.id));
                            backlog = kept;
                        }
                    }
                    (backlog, Some(excluded))
                } else {
                    let tasks = explicit_task_ids
                        .iter()
                        .map(|task_id| {
                            self.get_task(task_id).map_err(|err| {
                                DispatchError::DeterministicActionFailed {
                                    action: action.to_string(),
                                    message: format!("load task {task_id}: {err}"),
                                }
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    (tasks, None)
                };
                tasks.truncate(max_tasks);
                let ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
                let bundles: Vec<Vec<String>> =
                    ids.iter().map(|task_id| vec![task_id.clone()]).collect();
                let task_objs: Vec<Value> = tasks
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "id": t.id,
                            "title": t.title,
                            "type": t.task_type.to_string(),
                            "priority": t.priority.to_string(),
                            "context_files": t.context_files,
                            "parent_id": t.parent_id,
                        })
                    })
                    .collect();
                let mut payload = serde_json::Map::new();
                payload.insert("task_count".to_string(), Value::from(task_objs.len()));
                payload.insert("task_ids".to_string(), serde_json::json!(ids));
                payload.insert("tasks".to_string(), serde_json::json!(task_objs));
                payload.insert("bundles".to_string(), serde_json::json!(bundles));
                // Keep this Rust serialization contract in sync with
                // crates/orbit-core/assets/activities/list_backlog_tasks.yaml.
                if let Some(excluded) = excluded_entries {
                    payload.insert(
                        "excluded".to_string(),
                        serde_json::to_value(excluded).map_err(|err| {
                            DispatchError::DeterministicActionFailed {
                                action: action.to_string(),
                                message: format!("serialize excluded backlog tasks: {err}"),
                            }
                        })?,
                    );
                }
                Ok(Value::Object(payload))
            }
            // Materialize an epic's working set for the orchestrator:
            // the epic task itself plus non-terminal subtasks
            // (`parent_id == epic_task_id` and status ∉ {done, archived}).
            // Full descriptions ride along because the orchestrator
            // reasons about dependency ordering from prose.
            "load_epic" => {
                let epic_id = input
                    .get("epic_task_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: "missing `epic_task_id`".to_string(),
                    })?;
                let epic = self.get_task(epic_id).map_err(|err| {
                    DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: format!("load epic {epic_id}: {err}"),
                    }
                })?;
                if epic.task_type != TaskType::Epic {
                    return Err(DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: format!(
                            "task `{epic_id}` has type `{}`; expected `epic`",
                            epic.task_type
                        ),
                    });
                }
                let subtasks = self
                    .list_tasks_filtered(None, None, Some(epic_id), None)
                    .map_err(|err| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: format!("list subtasks of {epic_id}: {err}"),
                    })?;
                let final_subtasks = subtasks
                    .iter()
                    .map(|t| {
                        (
                            t.id.clone(),
                            serde_json::json!({
                                "state": epic_state_for_task_status(t.status),
                                "status": t.status.to_string(),
                                "title": t.title,
                            }),
                        )
                    })
                    .collect::<serde_json::Map<String, Value>>();
                let open_subtasks = subtasks
                    .iter()
                    .filter(|task| !is_epic_terminal_status(task.status))
                    .collect::<Vec<_>>();
                let subtask_payload: Vec<Value> = open_subtasks
                    .iter()
                    .filter(|t| !matches!(t.status, TaskStatus::Done | TaskStatus::Archived))
                    .map(|t| {
                        serde_json::json!({
                            "id": t.id,
                            "title": t.title,
                            "description": t.description,
                            "type": t.task_type.to_string(),
                            "status": t.status.to_string(),
                            "context_files": t.context_files,
                        })
                    })
                    .collect();
                Ok(serde_json::json!({
                    "epic": {
                        "id": epic.id,
                        "title": epic.title,
                        "description": epic.description,
                        "type": epic.task_type.to_string(),
                        "status": epic.status.to_string(),
                    },
                    "subtasks": subtask_payload,
                    "all_terminal": open_subtasks.is_empty(),
                    "final_state": {
                        "epic_id": epic.id.clone(),
                        "subtasks": final_subtasks,
                    },
                }))
            }
            // Fold the deterministic final task-state snapshot into counters
            // + a human-readable one-liner. Pure aggregation — the
            // orchestrator's final response is audit-only.
            "summarize_epic" => {
                let state = input.get("state").cloned().unwrap_or(Value::Null);
                let subtasks_map = state
                    .get("subtasks")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                let mut done = 0u64;
                let mut failed = 0u64;
                let mut blocked = 0u64;
                let mut in_flight = 0u64;
                let mut unfinished_ids: Vec<String> = Vec::new();
                for (id, entry) in subtasks_map.iter() {
                    let entry_state = entry
                        .get("state")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    match entry_state {
                        "done" => done += 1,
                        "blocked" => {
                            blocked += 1;
                            unfinished_ids.push(id.clone());
                        }
                        "failed" => {
                            failed += 1;
                            unfinished_ids.push(id.clone());
                        }
                        "in_flight" | "pending" => {
                            in_flight += 1;
                            unfinished_ids.push(id.clone());
                        }
                        _ => {
                            unfinished_ids.push(id.clone());
                        }
                    }
                }
                let total = subtasks_map.len() as u64;
                let message = if total == 0 {
                    "epic had no subtasks".to_string()
                } else if unfinished_ids.is_empty() {
                    format!("all {total} subtasks done")
                } else {
                    format!(
                        "{done}/{total} done; {failed} failed, {blocked} blocked, \
                         {in_flight} in flight/pending"
                    )
                };
                Ok(serde_json::json!({
                    "total": total,
                    "done": done,
                    "failed": failed,
                    "blocked": blocked,
                    "in_flight": in_flight,
                    "unfinished_ids": unfinished_ids,
                    "message": message,
                }))
            }
            // Guard the auto-dispatch bundle output before fan_out.
            // Rejects duplicated task_ids, unknown ids, and oversize
            // bundles with a structured error so a misgrouped backlog
            // never silently dispatches.
            "validate_bundles" => {
                let bundles_raw = input
                    .get("bundles")
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or_else(|| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: "`bundles` must be an array".to_string(),
                    })?;
                let max_bundle_size = input
                    .get("max_bundle_size")
                    .and_then(Value::as_u64)
                    .unwrap_or(5) as usize;
                let known: std::collections::BTreeSet<String> = input
                    .get("known_task_ids")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();

                let mut seen: std::collections::BTreeSet<String> =
                    std::collections::BTreeSet::new();
                let mut violations: Vec<String> = Vec::new();
                let mut bundles: Vec<Vec<String>> = Vec::with_capacity(bundles_raw.len());
                for (idx, bundle) in bundles_raw.iter().enumerate() {
                    let items = bundle.as_array().ok_or_else(|| {
                        DispatchError::DeterministicActionFailed {
                            action: action.to_string(),
                            message: format!("bundle[{idx}] is not an array"),
                        }
                    })?;
                    if items.len() > max_bundle_size {
                        violations.push(format!(
                            "bundle[{idx}] size {} exceeds max_bundle_size {}",
                            items.len(),
                            max_bundle_size
                        ));
                    }
                    let mut bundle_ids: Vec<String> = Vec::with_capacity(items.len());
                    for item in items {
                        let id = item.as_str().ok_or_else(|| {
                            DispatchError::DeterministicActionFailed {
                                action: action.to_string(),
                                message: format!("bundle[{idx}] contains a non-string task_id"),
                            }
                        })?;
                        if !known.is_empty() && !known.contains(id) {
                            violations
                                .push(format!("bundle[{idx}] references unknown task_id {id}"));
                        }
                        if !seen.insert(id.to_string()) {
                            violations
                                .push(format!("task_id {id} appears in more than one bundle"));
                        }
                        bundle_ids.push(id.to_string());
                    }
                    bundles.push(bundle_ids);
                }
                if !violations.is_empty() {
                    return Err(DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: format!("invalid bundles: {}", violations.join("; ")),
                    });
                }
                Ok(serde_json::json!({
                    "bundles": bundles,
                    "bundle_count": bundles.len(),
                }))
            }
            // Thin passthrough over `orbit.task.locks.reserve`. Exists as a
            // dedicated action (rather than a `orbit_tool_call` config) so a
            // workflow inside a `loop:` with `break_when:` can reference
            // `steps.<id>.output.reserved` directly without leaking the
            // generic `{tool_name, args}` envelope into the activity's
            // input_schema.
            "reserve_locks" => self
                .run_tool_with_context_and_role(
                    "orbit.task.locks.reserve",
                    input.clone(),
                    Role::Admin,
                    tool_context,
                )
                .map_err(|err| DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: format!("{err}"),
                }),
            // Thin passthrough over `orbit.task.locks.release` so workflows
            // can free admission-window reservations after child runs finish.
            "release_locks" => self
                .run_tool_with_context_and_role(
                    "orbit.task.locks.release",
                    input.clone(),
                    Role::Admin,
                    tool_context,
                )
                .map_err(|err| DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: format!("{err}"),
                }),
            // Submit a child v2 Job and block on its terminal state.
            // Chains `orbit.pipeline.invoke` + `orbit.pipeline.wait` so
            // workflows can model "dispatch and join" as a single step
            // with `{status, run_id, pipeline?, error?}` output.
            "invoke_and_wait" => {
                let job_name = input
                    .get("job_name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: "missing `job_name`".to_string(),
                    })?
                    .to_string();
                let run_input = input
                    .get("run_input")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Default::default()));
                let mut invoke_args = serde_json::Map::new();
                invoke_args.insert("job_name".to_string(), Value::String(job_name.clone()));
                invoke_args.insert("input".to_string(), run_input);
                if let Some(priority) = input.get("priority").cloned() {
                    invoke_args.insert("priority".to_string(), priority);
                }

                let invoke_ctx = tool_context.clone();
                let invoke_output = self
                    .run_tool_with_context_and_role(
                        "orbit.pipeline.invoke",
                        Value::Object(invoke_args),
                        Role::Admin,
                        invoke_ctx,
                    )
                    .map_err(|err| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: format!("pipeline.invoke failed: {err}"),
                    })?;

                let run_id = invoke_output
                    .get("run_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: "pipeline.invoke returned no run_id".to_string(),
                    })?
                    .to_string();

                let mut wait_args = serde_json::Map::new();
                wait_args.insert(
                    "run_ids".to_string(),
                    Value::Array(vec![Value::String(run_id.clone())]),
                );
                if let Some(timeout) = input.get("timeout_seconds").cloned() {
                    wait_args.insert("timeout_seconds".to_string(), timeout);
                }
                if let Some(poll) = input.get("poll_interval_seconds").cloned() {
                    wait_args.insert("poll_interval_seconds".to_string(), poll);
                }

                let wait_output = self
                    .run_tool_with_context_and_role(
                        "orbit.pipeline.wait",
                        Value::Object(wait_args),
                        Role::Admin,
                        tool_context,
                    )
                    .map_err(|err| DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: format!("pipeline.wait failed: {err}"),
                    })?;

                let first = wait_output
                    .get("results")
                    .and_then(Value::as_array)
                    .and_then(|arr| arr.first())
                    .cloned()
                    .unwrap_or_else(|| {
                        serde_json::json!({
                            "run_id": run_id,
                            "status": "pending",
                        })
                    });
                Ok(first)
            }
            // Post-loop gate signal: the admission window never opened in
            // time. Emits a `gate.starvation` audit event with task_ids and
            // conflicting_files so an epic-orchestrator parent can decide
            // to replan, then fails the Run with a structured error.
            "gate_starvation_fail" => {
                let task_ids_vec: Vec<String> = input
                    .get("task_ids")
                    .and_then(Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let conflicts = input
                    .get("conflicts")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new()));
                let max_wait_seconds = input.get("max_wait_seconds").and_then(Value::as_f64);
                let conflicting_files: Vec<String> = conflicts
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|entry| {
                                entry
                                    .get("file")
                                    .and_then(Value::as_str)
                                    .map(ToOwned::to_owned)
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let payload = serde_json::json!({
                    "task_ids": task_ids_vec,
                    "conflicting_files": conflicting_files,
                    "conflicts": conflicts,
                    "max_wait_seconds": max_wait_seconds,
                });

                let execution_id = format!(
                    "audit-gate-starvation-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|duration| duration.as_nanos())
                        .unwrap_or(0)
                );
                let working_directory = self.paths().repo_root.to_string_lossy().into_owned();
                self.record_audit_event(&AuditEventInsertParams {
                    execution_id,
                    command: "gate.starvation".to_string(),
                    subcommand: None,
                    tool_name: None,
                    target_type: Some("task_bundle".to_string()),
                    target_id: task_ids_vec.first().cloned(),
                    role: "admin".to_string(),
                    status: AuditEventStatus::Failure,
                    exit_code: 1,
                    duration_ms: 0,
                    working_directory,
                    arguments_json: Some(serde_json::to_string(&payload).map_err(|error| {
                        DispatchError::DeterministicActionFailed {
                            action: action.to_string(),
                            message: format!("serialize gate.starvation payload: {error}"),
                        }
                    })?),
                    stdout_truncated: None,
                    stderr_truncated: None,
                    error_message: Some("gate.starvation".to_string()),
                    host: std::env::var("HOSTNAME").ok(),
                    pid: std::process::id(),
                    session_id: None,
                    task_id: task_ids_vec.first().cloned(),
                    job_run_id: None,
                    activity_id: None,
                    step_index: None,
                })
                .map_err(|err| DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: format!("record gate.starvation audit: {err}"),
                })?;

                Err(DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: format!(
                        "gate.starvation: admission window never opened for bundle {:?} \
                         (conflicting_files={:?}, max_wait_seconds={:?})",
                        task_ids_vec, conflicting_files, max_wait_seconds
                    ),
                })
            }
            other => Err(DispatchError::DeterministicActionNotRegistered(
                other.to_string(),
            )),
        }
    }

    fn resolve_cli_executor(&self, provider: &str) -> Result<ResolvedCliExecutor, DispatchError> {
        resolve_cli_executor(self, provider)
    }

    fn provider_cli_config(&self, _provider: &str) -> HashMap<String, String> {
        EnvironmentHost::agent_provider_config(self)
    }

    fn resolve_executor_sandbox(
        &self,
        provider: &str,
        fs_profile: Option<&str>,
    ) -> Result<Option<ResolvedSandbox>, DispatchError> {
        let executor = self.get_executor_def(provider).map_err(|err| {
            DispatchError::CliInvocationFailed(format!(
                "load executor `{provider}` for sandbox resolution: {err}"
            ))
        })?;
        let Some(executor) = executor else {
            return Ok(None);
        };
        let Some(kind) = executor.sandbox else {
            return Ok(None);
        };
        match kind {
            ExecutorSandboxKind::MacosSandboxExec => {
                #[cfg(not(target_os = "macos"))]
                {
                    return Err(DispatchError::CliInvocationFailed(format!(
                        "executor `{provider}` declares sandbox `macos-sandbox-exec` but current platform is `{}`",
                        std::env::consts::OS
                    )));
                }
                #[cfg(target_os = "macos")]
                {
                    let mut resolved =
                        resolve_fs_profile_absolute(self, fs_profile).map_err(|err| {
                            DispatchError::CliInvocationFailed(format!(
                                "resolve fsProfile for sandbox: {err}"
                            ))
                        })?;
                    append_provider_side_write_roots(self, provider, &mut resolved)?;
                    Ok(Some(ResolvedSandbox {
                        kind,
                        fs_profile: resolved,
                        allow_fallback: executor.allow_fallback,
                    }))
                }
            }
        }
    }

    fn task_context_for_agent_input(&self, input: &Value) -> Result<Option<Value>, DispatchError> {
        task_context_for_agent_input(self, input)
    }

    fn tool_context_for_activity(
        &self,
        fs_profile: Option<&str>,
        fs_audit: Option<Arc<dyn FsAuditLogger>>,
    ) -> ToolContext {
        let workspace_root = self
            .paths()
            .repo_root
            .canonicalize()
            .unwrap_or_else(|_| self.paths().repo_root.clone());

        ToolContext {
            cwd: std::env::current_dir()
                .ok()
                .map(|cwd| cwd.to_string_lossy().into_owned()),
            workspace_root: Some(workspace_root),
            policy_engine: Some(Arc::new(self.policy_engine().clone())),
            fs_profile: Some(fs_profile.unwrap_or(UNRESTRICTED_FS_PROFILE).to_string()),
            fs_audit,
            orbit_host: Some(build_orbit_tool_host(self, None)),
            ..Default::default()
        }
    }

    fn persist_invocation_trace(
        &self,
        job_run_id: &str,
        activity_id: &str,
        provider: &str,
        model: Option<&str>,
        input: &Value,
        trace: &InvocationTrace,
    ) -> Result<(), DispatchError> {
        let (agent, model) = self.canonical_agent_model_identity(Some(provider), model);
        let store = Store::open(&self.context.persistence().audit_db).map_err(|error| {
            DispatchError::JobExecution(format!("open invocation store: {error}"))
        })?;
        store
            .insert_invocation_trace_record(&InvocationInsertParams {
                job_run_id: job_run_id.to_string(),
                activity_id: activity_id.to_string(),
                agent: agent.unwrap_or_else(|| provider.to_ascii_lowercase()),
                model,
                task_ids: associated_task_ids(input),
                trace: trace.clone(),
            })
            .map_err(|error| {
                DispatchError::JobExecution(format!("persist invocation trace: {error}"))
            })?;

        if let Err(error) =
            token_scoreboard::write_token_scoreboard(&self.paths().scoreboard_dir, &store)
        {
            tracing::warn!(
                target: "orbit.core.scoreboard",
                error = %error,
                "failed to refresh tokens scoreboard",
            );
        }

        Ok(())
    }

    fn agent_role_config(&self, role: AgentRole) -> Option<AgentRoleConfig> {
        EnvironmentHost::agent_role_config(self, role)
    }

    fn api_key_for(&self, provider: &str) -> Result<String, DispatchError> {
        match provider {
            "anthropic" => {
                let key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
                    DispatchError::AgentLoopFailed(
                        "ANTHROPIC_API_KEY not set — export it before running a v2 agent_loop activity"
                            .to_string(),
                    )
                })?;
                if key.is_empty() {
                    return Err(DispatchError::AgentLoopFailed(
                        "ANTHROPIC_API_KEY is empty".to_string(),
                    ));
                }
                Ok(key)
            }
            other => Err(DispatchError::AgentLoopFailed(format!(
                "unsupported provider: {other}"
            ))),
        }
    }
}

fn associated_task_ids(input: &Value) -> Vec<String> {
    let mut task_ids = Vec::new();
    if let Some(task_id) = input.get("task_id").and_then(Value::as_str) {
        push_unique_task_id(&mut task_ids, task_id);
    }
    if let Some(items) = input.get("task_ids").and_then(Value::as_array) {
        for item in items {
            if let Some(task_id) = item.as_str() {
                push_unique_task_id(&mut task_ids, task_id);
            }
        }
    }
    if let Some(items) = input.get("tasks").and_then(Value::as_array) {
        for item in items {
            if let Some(task_id) = item.as_str() {
                push_unique_task_id(&mut task_ids, task_id);
                continue;
            }
            if let Some(task_id) = item
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| item.get("task_id").and_then(Value::as_str))
            {
                push_unique_task_id(&mut task_ids, task_id);
            }
        }
    }
    task_ids
}

fn task_context_for_agent_input(
    runtime: &OrbitRuntime,
    input: &Value,
) -> Result<Option<Value>, DispatchError> {
    let Some(task_id) = singular_task_id_from_input(input) else {
        return Ok(None);
    };
    let task = runtime.get_task(task_id).map_err(|err| {
        DispatchError::CliInvocationFailed(format!(
            "load task `{task_id}` for agent envelope: {err}"
        ))
    })?;
    Ok(Some(agent_task_context_json(
        &task,
        input,
        &runtime.paths().repo_root,
    )))
}

fn singular_task_id_from_input(input: &Value) -> Option<&str> {
    fn non_empty(value: &str) -> Option<&str> {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    }

    input
        .get("task_id")
        .and_then(Value::as_str)
        .and_then(non_empty)
        .or_else(|| {
            input
                .get("task")
                .and_then(|task| task.get("id"))
                .and_then(Value::as_str)
                .and_then(non_empty)
        })
        .or_else(|| {
            let items = input.get("task_ids")?.as_array()?;
            if items.len() == 1 {
                items.first()?.as_str().and_then(non_empty)
            } else {
                None
            }
        })
}

fn agent_task_context_json(task: &Task, input: &Value, fallback_repo_root: &Path) -> Value {
    let workspace_path = input
        .get("workspace_path")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| task.workspace_path.clone());
    let repo_root = input
        .get("repo_root")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| task.repo_root.clone());
    let prune_root = context_workspace_root(fallback_repo_root, workspace_path.as_deref());
    let canonical_context_files =
        canonicalize_context_files_for_read(&task.context_files, &prune_root);
    let (kept_context_files, _dropped) =
        prune_missing_context_files(&prune_root, canonical_context_files);

    serde_json::json!({
        "id": task.id.clone(),
        "title": task.title.clone(),
        "description": task.description.clone(),
        "acceptance_criteria": task.acceptance_criteria.clone(),
        "plan": task.plan.clone(),
        "context_files": kept_context_files,
        "pr_number": task.pr_number.clone(),
        "workspace_path": workspace_path,
        "repo_root": repo_root,
    })
}

fn push_unique_task_id(task_ids: &mut Vec<String>, task_id: &str) {
    let task_id = task_id.trim();
    if !task_id.is_empty() && !task_ids.iter().any(|existing| existing == task_id) {
        task_ids.push(task_id.to_string());
    }
}

/// Map a v2 provider name to the CLI executor that dispatches it. Env-var
/// overrides (`ORBIT_V2_CLI_<PROVIDER>`) let smokes substitute a fixture
/// binary for the real provider CLI; production normally comes from the
/// registered executor def, falling back to the provider name itself
/// (`claude`, `codex`, `gemini`, `ollama`) when no executor is registered.
fn resolve_cli_executor(
    runtime: &OrbitRuntime,
    provider: &str,
) -> Result<ResolvedCliExecutor, DispatchError> {
    let env_key = format!("ORBIT_V2_CLI_{}", provider.to_ascii_uppercase());
    let env_command = std::env::var(&env_key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if let Some(def) = runtime.get_executor_def(provider).map_err(|err| {
        DispatchError::CliInvocationFailed(format!("load executor `{provider}`: {err}"))
    })? {
        if !matches!(
            def.executor_type,
            ExecutorType::DirectAgent | ExecutorType::AgentCli
        ) {
            return Err(DispatchError::CliInvocationFailed(format!(
                "executor `{provider}` has type `{}`; backend: cli requires a direct_agent or agent_cli executor",
                def.executor_type
            )));
        }

        let command = env_command
            .or_else(|| {
                def.command
                    .as_ref()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
            .ok_or_else(|| {
                DispatchError::CliInvocationFailed(format!(
                    "executor `{provider}` is missing a command"
                ))
            })?;

        return Ok(ResolvedCliExecutor {
            command,
            args: def.args,
        });
    }

    if let Some(command) = env_command {
        return Ok(ResolvedCliExecutor {
            command,
            args: Vec::new(),
        });
    }

    match provider {
        "claude" | "codex" | "gemini" | "ollama" => Ok(ResolvedCliExecutor {
            command: provider.to_string(),
            args: Vec::new(),
        }),
        "openai_compat" => Err(DispatchError::CliInvocationFailed(
            "provider openai_compat has no CLI runtime (HTTP-only)".to_string(),
        )),
        other => Err(DispatchError::CliInvocationFailed(format!(
            "unknown provider `{other}` — no CLI runtime registered"
        ))),
    }
}

const MAX_TASK_PARENT_CHAIN_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct BacklogTaskExclusion {
    id: String,
    reason: BacklogTaskExclusionReason,
    conflicts: Vec<BacklogTaskConflict>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum BacklogTaskExclusionReason {
    ContextLockConflict,
    GroupMemberConflict,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct BacklogTaskConflict {
    requested_file: String,
    locking_task_id: String,
}

fn active_task_lock_holders<'a>(
    tasks: impl IntoIterator<Item = &'a Task>,
) -> BTreeMap<String, Vec<String>> {
    let mut holders: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for task in tasks {
        if matches!(task.status, TaskStatus::InProgress | TaskStatus::Review) {
            for file in existing_lock_context_files(task) {
                holders.entry(file).or_default().push(task.id.clone());
            }
        }
    }
    for locking_task_ids in holders.values_mut() {
        locking_task_ids.sort();
        locking_task_ids.dedup();
    }
    holders
}

fn is_epic_terminal_status(status: TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Done | TaskStatus::Blocked | TaskStatus::Archived | TaskStatus::Rejected
    )
}

fn epic_state_for_task_status(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Done | TaskStatus::Archived => "done",
        TaskStatus::Blocked | TaskStatus::Rejected => "blocked",
        TaskStatus::InProgress | TaskStatus::Review => "in_flight",
        TaskStatus::Proposed | TaskStatus::Friction | TaskStatus::Backlog | TaskStatus::Someday => {
            "pending"
        }
    }
}

fn task_overlap_conflicts(
    task: &Task,
    holders: &BTreeMap<String, Vec<String>>,
) -> Vec<BacklogTaskConflict> {
    let mut conflicts = Vec::new();
    for requested_file in existing_lock_context_files(task) {
        for (held_file, locking_task_ids) in holders {
            if workspace_relative_paths_overlap(&requested_file, held_file) {
                for locking_task_id in locking_task_ids {
                    conflicts.push(BacklogTaskConflict {
                        requested_file: requested_file.clone(),
                        locking_task_id: locking_task_id.clone(),
                    });
                }
            }
        }
    }
    conflicts.sort();
    conflicts.dedup();
    conflicts
}

fn existing_lock_context_files(task: &Task) -> Vec<String> {
    let workspace_root = task
        .workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let canonical = task
        .context_files
        .iter()
        .filter_map(|entry| canonical_selector_in_workspace(entry, &workspace_root).ok())
        .collect::<Vec<_>>();
    let (kept, _dropped) = prune_missing_context_files(&workspace_root, canonical);
    kept
}

/// Resolve the activity's fsProfile against the active policy, then expand
/// every workspace-relative `read` / `modify` rule to an absolute path under
/// the workspace root. The kernel's `subpath` predicate is meaningless for
/// relative paths, so this is the layer that turns Orbit's policy into a
/// payload `sandbox-exec` can enforce.
fn resolve_fs_profile_absolute(
    runtime: &OrbitRuntime,
    fs_profile: Option<&str>,
) -> Result<ResolvedFsProfile, orbit_common::types::OrbitError> {
    let profile_name = fs_profile.unwrap_or(UNRESTRICTED_FS_PROFILE);
    let resolved = runtime
        .policy_engine()
        .def()
        .effective_profile(profile_name)?;
    let workspace_root = runtime
        .paths()
        .repo_root
        .canonicalize()
        .unwrap_or_else(|_| runtime.paths().repo_root.clone());
    let workspace_str = workspace_root.display().to_string();

    Ok(ResolvedFsProfile {
        name: resolved.name,
        read: resolved
            .read
            .into_iter()
            .map(|rule| absolutize_rule(&workspace_str, &rule))
            .collect(),
        modify: resolved
            .modify
            .into_iter()
            .map(|rule| absolutize_rule(&workspace_str, &rule))
            .collect(),
    })
}

#[cfg(target_os = "macos")]
fn append_provider_side_write_roots(
    runtime: &OrbitRuntime,
    provider: &str,
    resolved: &mut ResolvedFsProfile,
) -> Result<(), DispatchError> {
    // Codex is the only `backend: cli` provider that ships its own writable
    // root surface (`--add-dir` fed from `writable_dirs_json`). Claude and
    // Gemini have no analogous CLI flag — their startup-time writes are
    // confined to their state directories, which `compile_macos_sandbox_profile`
    // already grants via the per-provider state-dir allowances. If a future
    // provider gains a side-root surface, generalize this branch rather than
    // duplicating it. See T20260428-14.
    if provider != "codex" {
        return Ok(());
    }

    let config = EnvironmentHost::agent_provider_config(runtime);
    let Some(raw_dirs) = config.get("writable_dirs_json") else {
        return Ok(());
    };
    let writable_dirs: Vec<String> = serde_json::from_str(raw_dirs).map_err(|err| {
        DispatchError::CliInvocationFailed(format!(
            "parse codex writable_dirs_json for sandbox: {err}"
        ))
    })?;
    if writable_dirs.is_empty() {
        return Ok(());
    }

    let workspace_root = runtime
        .paths()
        .repo_root
        .canonicalize()
        .unwrap_or_else(|_| runtime.paths().repo_root.clone());
    let workspace_str = workspace_root.display().to_string();
    for dir in writable_dirs {
        let Some(root) = absolutize_side_write_root(&workspace_str, &dir) else {
            continue;
        };
        // Append even when the root already appears earlier: SBPL is
        // last-match-wins, and these host-owned roots must land after
        // policy-derived denies such as `.orbit/**`.
        resolved.modify.push(root);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn absolutize_side_write_root(workspace_root: &str, path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let absolute = if PathBuf::from(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        let trimmed = trimmed.trim_start_matches("./");
        if trimmed.is_empty() || trimmed == "." {
            PathBuf::from(workspace_root)
        } else {
            PathBuf::from(workspace_root).join(trimmed)
        }
    };
    let normalized = absolute.canonicalize().unwrap_or(absolute);
    Some(normalized.display().to_string())
}

fn absolutize_rule(workspace_root: &str, rule: &str) -> String {
    let (negated, body) = rule
        .strip_prefix('!')
        .map(|rest| (true, rest))
        .unwrap_or((false, rule));
    let trimmed = body.trim_start_matches("./");
    let absolute = if PathBuf::from(trimmed).is_absolute() {
        trimmed.to_string()
    } else if trimmed.is_empty() || trimmed == "." {
        workspace_root.to_string()
    } else {
        format!("{}/{}", workspace_root.trim_end_matches('/'), trimmed)
    };
    if negated {
        format!("!{absolute}")
    } else {
        absolute
    }
}

fn task_root_id(task: &Task, task_lookup: &BTreeMap<String, Task>) -> String {
    let mut path = vec![task.id.clone()];
    let mut root_id = task.id.clone();
    let mut next_parent_id = task.parent_id.clone();

    for _ in 0..MAX_TASK_PARENT_CHAIN_DEPTH {
        let Some(parent_id) = next_parent_id else {
            return root_id;
        };

        if let Some(cycle_start) = path.iter().position(|task_id| task_id == &parent_id) {
            return path[cycle_start..].iter().min().cloned().unwrap_or(root_id);
        }

        let Some(parent) = task_lookup.get(&parent_id) else {
            return root_id;
        };

        root_id = parent.id.clone();
        path.push(parent.id.clone());
        next_parent_id = parent.parent_id.clone();
    }

    root_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use orbit_common::types::{ExecutorDef, ExecutorType, TaskPriority};
    use orbit_engine::activity_job::V2RuntimeHost;
    use orbit_tools::ToolContext;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    use crate::command::task::TaskAddParams;

    #[test]
    fn run_planning_duel_is_registered_for_v2_deterministic_dispatch() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let err = runtime
            .run_deterministic(
                "run_planning_duel",
                &json!({}),
                &json!({}),
                ToolContext::default(),
            )
            .expect_err("empty input should fail validation inside the action");

        match err {
            DispatchError::DeterministicActionFailed { action, message } => {
                assert_eq!(action, "run_planning_duel");
                assert!(
                    message.contains("missing required input.task_id"),
                    "unexpected validation message: {message}"
                );
            }
            other => panic!("expected registered action failure, got {other}"),
        }
    }

    #[test]
    fn task_context_for_agent_input_embeds_canonical_task_with_input_overrides() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "Envelope task".to_string(),
                description: "Task description for agent context.".to_string(),
                acceptance_criteria: vec!["Agent can recover the task id.".to_string()],
                plan: "Read the task and implement it.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");

        let context = runtime
            .task_context_for_agent_input(&json!({
                "task_id": task.id.clone(),
                "workspace_path": "/override/worktree",
                "repo_root": "/override/repo"
            }))
            .expect("build task context")
            .expect("task context present");

        assert_eq!(context["id"], task.id);
        assert_eq!(context["title"], "Envelope task");
        assert_eq!(
            context["description"],
            "Task description for agent context."
        );
        assert_eq!(
            context["acceptance_criteria"][0],
            "Agent can recover the task id."
        );
        assert_eq!(context["plan"], "Read the task and implement it.");
        assert_eq!(context["workspace_path"], "/override/worktree");
        assert_eq!(context["repo_root"], "/override/repo");
    }

    fn seed_executor(
        runtime: &OrbitRuntime,
        name: &str,
        sandbox: Option<orbit_common::types::ExecutorSandboxKind>,
    ) {
        let now = Utc::now();
        runtime
            .upsert_executor_def(&ExecutorDef {
                name: name.to_string(),
                executor_type: ExecutorType::DirectAgent,
                command: Some(name.to_string()),
                args: vec!["exec".to_string(), "--json".to_string()],
                stdout_format: None,
                models: HashMap::new(),
                timeout_seconds: None,
                env: HashMap::new(),
                sandbox,
                allow_fallback: false,
                created_at: now,
                updated_at: now,
            })
            .expect("seed executor");
    }

    fn seeded_runtime_with_executor(
        sandbox: Option<orbit_common::types::ExecutorSandboxKind>,
    ) -> OrbitRuntime {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        seed_executor(&runtime, "codex", sandbox);
        runtime
    }

    fn runtime_with_workspace_layout() -> (tempfile::TempDir, OrbitRuntime, PathBuf) {
        let root = tempdir().expect("create tempdir");
        let global = root.path().join("home/.orbit");
        let workspace = root.path().join("repo/.orbit");
        std::fs::create_dir_all(&global).expect("global orbit dir");
        std::fs::create_dir_all(&workspace).expect("workspace orbit dir");
        let runtime = OrbitRuntime::from_roots(&global, &workspace).expect("build runtime");
        let repo_root = root.path().join("repo");
        (root, runtime, repo_root)
    }

    fn write_workspace_file(repo_root: &Path, relative_path: &str) {
        let path = repo_root.join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, "test fixture\n").expect("write workspace file");
    }

    fn seed_list_backlog_task(
        runtime: &OrbitRuntime,
        title: &str,
        status: TaskStatus,
        priority: TaskPriority,
        task_type: TaskType,
        parent_id: Option<String>,
        context_files: Vec<&str>,
    ) -> Task {
        runtime
            .add_task(TaskAddParams {
                parent_id,
                title: title.to_string(),
                description: format!("Fixture task: {title}"),
                acceptance_criteria: vec!["Fixture task is observable.".to_string()],
                plan: "Fixture plan.".to_string(),
                context_files: context_files.into_iter().map(str::to_string).collect(),
                workspace_path: Some(".".to_string()),
                priority,
                task_type: Some(task_type),
                status: Some(status),
                ..Default::default()
            })
            .expect("seed task")
    }

    fn seed_accepted_friction_task(
        runtime: &OrbitRuntime,
        title: &str,
        priority: TaskPriority,
        context_files: Vec<&str>,
    ) -> Task {
        let report = seed_list_backlog_task(
            runtime,
            title,
            TaskStatus::Friction,
            priority,
            TaskType::Friction,
            None,
            context_files,
        );
        runtime
            .approve_task(
                &report.id,
                Some("Accepted friction report.".to_string()),
                None,
            )
            .expect("accept friction task")
    }

    fn list_backlog_tasks(runtime: &OrbitRuntime, input: Value) -> Value {
        runtime
            .run_deterministic(
                "list_backlog_tasks",
                &json!({}),
                &input,
                ToolContext::default(),
            )
            .expect("list backlog tasks")
    }

    fn excluded_entry<'a>(output: &'a Value, task_id: &str) -> &'a Value {
        output["excluded"]
            .as_array()
            .expect("excluded array")
            .iter()
            .find(|entry| entry["id"] == task_id)
            .expect("excluded entry")
    }

    #[test]
    fn list_backlog_tasks_preserves_existing_fields_without_conflicts() {
        let (_root, runtime, repo_root) = runtime_with_workspace_layout();
        write_workspace_file(&repo_root, "crates/alpha/src/lib.rs");
        write_workspace_file(&repo_root, "crates/beta/src/lib.rs");
        let medium = seed_list_backlog_task(
            &runtime,
            "Medium backlog",
            TaskStatus::Backlog,
            TaskPriority::Medium,
            TaskType::Task,
            None,
            vec!["crates/alpha/src/lib.rs"],
        );
        let high = seed_list_backlog_task(
            &runtime,
            "High backlog",
            TaskStatus::Backlog,
            TaskPriority::High,
            TaskType::Task,
            None,
            vec!["crates/beta/src/lib.rs"],
        );

        let output = list_backlog_tasks(&runtime, json!({}));

        assert_eq!(output["task_count"], json!(2));
        assert_eq!(output["task_ids"], json!([high.id, medium.id]));
        assert_eq!(
            output["tasks"],
            json!([
                {
                    "id": high.id,
                    "title": "High backlog",
                    "type": "task",
                    "priority": "high",
                    "context_files": high.context_files,
                    "parent_id": null
                },
                {
                    "id": medium.id,
                    "title": "Medium backlog",
                    "type": "task",
                    "priority": "medium",
                    "context_files": medium.context_files,
                    "parent_id": null
                }
            ])
        );
        assert_eq!(output["excluded"], json!([]));
    }

    #[test]
    fn list_backlog_tasks_includes_accepted_friction_reports() {
        let (_root, runtime, repo_root) = runtime_with_workspace_layout();
        write_workspace_file(&repo_root, "crates/friction/src/lib.rs");
        let friction = seed_accepted_friction_task(
            &runtime,
            "Accepted friction",
            TaskPriority::Medium,
            vec!["crates/friction/src/lib.rs"],
        );

        let output = list_backlog_tasks(&runtime, json!({}));

        assert_eq!(output["task_count"], json!(1));
        assert_eq!(output["task_ids"], json!([friction.id]));
        assert_eq!(output["bundles"], json!([[friction.id]]));
        assert_eq!(
            output["tasks"],
            json!([{
                "id": friction.id,
                "title": "Accepted friction",
                "type": "friction",
                "priority": "medium",
                "context_files": friction.context_files,
                "parent_id": null
            }])
        );
        assert_eq!(output["excluded"], json!([]));
    }

    #[test]
    fn list_backlog_tasks_omits_untriaged_friction_reports() {
        let (_root, runtime, repo_root) = runtime_with_workspace_layout();
        write_workspace_file(&repo_root, "crates/friction/src/lib.rs");
        let friction = seed_list_backlog_task(
            &runtime,
            "Untriaged friction",
            TaskStatus::Friction,
            TaskPriority::Medium,
            TaskType::Friction,
            None,
            vec!["crates/friction/src/lib.rs"],
        );
        let friction_id = friction.id.clone();

        let output = list_backlog_tasks(&runtime, json!({}));

        assert_eq!(output["task_count"], json!(0));
        assert_eq!(output["task_ids"], json!([]));
        assert_eq!(output["tasks"], json!([]));
        assert_eq!(output["bundles"], json!([]));
        assert_eq!(output["excluded"], json!([]));
        assert!(
            output["task_ids"]
                .as_array()
                .expect("task_ids")
                .iter()
                .all(|task_id| task_id != &json!(friction_id))
        );
    }

    #[test]
    fn list_backlog_tasks_reports_direct_context_lock_conflicts() {
        let (_root, runtime, repo_root) = runtime_with_workspace_layout();
        write_workspace_file(&repo_root, "crates/foo/src/lib.rs");
        let locking = seed_list_backlog_task(
            &runtime,
            "Locking task",
            TaskStatus::InProgress,
            TaskPriority::Medium,
            TaskType::Task,
            None,
            vec!["crates/foo/src/lib.rs"],
        );
        let backlog = seed_list_backlog_task(
            &runtime,
            "Backlog task",
            TaskStatus::Backlog,
            TaskPriority::Medium,
            TaskType::Task,
            None,
            vec!["crates/foo/src/lib.rs"],
        );

        let output = list_backlog_tasks(&runtime, json!({}));

        assert_eq!(output["task_count"], json!(0));
        assert_eq!(output["task_ids"], json!([]));
        assert_eq!(output["tasks"], json!([]));
        assert_eq!(output["bundles"], json!([]));
        assert_eq!(
            output["excluded"],
            json!([{
                "id": backlog.id,
                "reason": "context_lock_conflict",
                "conflicts": [{
                    "requested_file": backlog.context_files[0],
                    "locking_task_id": locking.id
                }]
            }])
        );
    }

    #[test]
    fn list_backlog_tasks_reports_group_member_conflicts_with_trigger_conflicts() {
        let (_root, runtime, repo_root) = runtime_with_workspace_layout();
        write_workspace_file(&repo_root, "docs/parent.md");
        write_workspace_file(&repo_root, "crates/foo/src/lib.rs");
        write_workspace_file(&repo_root, "crates/bar/src/lib.rs");
        let foo_lock = seed_list_backlog_task(
            &runtime,
            "Foo lock",
            TaskStatus::InProgress,
            TaskPriority::Medium,
            TaskType::Task,
            None,
            vec!["crates/foo/src/lib.rs"],
        );
        let bar_lock = seed_list_backlog_task(
            &runtime,
            "Bar lock",
            TaskStatus::InProgress,
            TaskPriority::Medium,
            TaskType::Task,
            None,
            vec!["crates/bar/src/lib.rs"],
        );
        let parent = seed_list_backlog_task(
            &runtime,
            "Parent",
            TaskStatus::Backlog,
            TaskPriority::Medium,
            TaskType::Task,
            None,
            vec!["docs/parent.md"],
        );
        let low_child = seed_list_backlog_task(
            &runtime,
            "Low child",
            TaskStatus::Backlog,
            TaskPriority::Medium,
            TaskType::Task,
            Some(parent.id.clone()),
            vec!["crates/foo/src/lib.rs"],
        );
        let high_child = seed_list_backlog_task(
            &runtime,
            "High child",
            TaskStatus::Backlog,
            TaskPriority::High,
            TaskType::Task,
            Some(parent.id.clone()),
            vec!["crates/bar/src/lib.rs"],
        );

        let output = list_backlog_tasks(&runtime, json!({}));

        assert_eq!(output["task_count"], json!(0));
        assert_eq!(output["excluded"].as_array().expect("excluded").len(), 3);
        assert_eq!(
            excluded_entry(&output, &parent.id),
            &json!({
                "id": parent.id,
                "reason": "group_member_conflict",
                "conflicts": [{
                    "requested_file": high_child.context_files[0],
                    "locking_task_id": bar_lock.id
                }]
            })
        );
        assert_eq!(
            excluded_entry(&output, &high_child.id),
            &json!({
                "id": high_child.id,
                "reason": "context_lock_conflict",
                "conflicts": [{
                    "requested_file": high_child.context_files[0],
                    "locking_task_id": bar_lock.id
                }]
            })
        );
        assert_eq!(
            excluded_entry(&output, &low_child.id),
            &json!({
                "id": low_child.id,
                "reason": "context_lock_conflict",
                "conflicts": [{
                    "requested_file": low_child.context_files[0],
                    "locking_task_id": foo_lock.id
                }]
            })
        );
    }

    #[test]
    fn list_backlog_tasks_reports_accepted_friction_context_lock_conflicts() {
        let (_root, runtime, repo_root) = runtime_with_workspace_layout();
        write_workspace_file(&repo_root, "crates/friction/src/lib.rs");
        let locking = seed_list_backlog_task(
            &runtime,
            "Locking task",
            TaskStatus::InProgress,
            TaskPriority::Medium,
            TaskType::Task,
            None,
            vec!["crates/friction/src/lib.rs"],
        );
        let friction = seed_accepted_friction_task(
            &runtime,
            "Accepted friction",
            TaskPriority::Medium,
            vec!["crates/friction/src/lib.rs"],
        );

        let output = list_backlog_tasks(&runtime, json!({}));

        assert_eq!(output["task_count"], json!(0));
        assert_eq!(output["task_ids"], json!([]));
        assert_eq!(output["tasks"], json!([]));
        assert_eq!(output["bundles"], json!([]));
        assert_eq!(
            output["excluded"],
            json!([{
                "id": friction.id,
                "reason": "context_lock_conflict",
                "conflicts": [{
                    "requested_file": friction.context_files[0],
                    "locking_task_id": locking.id
                }]
            }])
        );
    }

    #[test]
    fn list_backlog_tasks_does_not_report_untriaged_friction_tasks_as_excluded() {
        let (_root, runtime, repo_root) = runtime_with_workspace_layout();
        write_workspace_file(&repo_root, "crates/foo/src/lib.rs");
        let locking = seed_list_backlog_task(
            &runtime,
            "Locking task",
            TaskStatus::InProgress,
            TaskPriority::Medium,
            TaskType::Task,
            None,
            vec!["crates/foo/src/lib.rs"],
        );
        let friction = seed_list_backlog_task(
            &runtime,
            "Friction task",
            TaskStatus::Friction,
            TaskPriority::Medium,
            TaskType::Friction,
            None,
            vec!["crates/foo/src/lib.rs"],
        );
        let backlog = seed_list_backlog_task(
            &runtime,
            "Backlog task",
            TaskStatus::Backlog,
            TaskPriority::Medium,
            TaskType::Task,
            None,
            vec!["crates/foo/src/lib.rs"],
        );

        let output = list_backlog_tasks(&runtime, json!({}));

        assert_eq!(
            output["excluded"],
            json!([{
                "id": backlog.id,
                "reason": "context_lock_conflict",
                "conflicts": [{
                    "requested_file": backlog.context_files[0],
                    "locking_task_id": locking.id
                }]
            }])
        );
        assert!(
            output["excluded"]
                .as_array()
                .expect("excluded")
                .iter()
                .all(|entry| entry["id"] != friction.id)
        );
    }

    #[test]
    fn list_backlog_tasks_does_not_report_max_tasks_truncation_as_excluded() {
        let (_root, runtime, repo_root) = runtime_with_workspace_layout();
        for index in 0..3 {
            let path = format!("docs/task-{index}.md");
            write_workspace_file(&repo_root, &path);
            seed_list_backlog_task(
                &runtime,
                &format!("Task {index}"),
                TaskStatus::Backlog,
                TaskPriority::Medium,
                TaskType::Task,
                None,
                vec![&path],
            );
        }

        let output = list_backlog_tasks(&runtime, json!({ "max_tasks": 2 }));

        assert_eq!(output["task_count"], json!(2));
        assert_eq!(output["task_ids"].as_array().expect("task_ids").len(), 2);
        assert_eq!(output["excluded"], json!([]));
    }

    #[test]
    fn list_backlog_tasks_omits_excluded_for_explicit_task_ids() {
        let (_root, runtime, repo_root) = runtime_with_workspace_layout();
        write_workspace_file(&repo_root, "crates/foo/src/lib.rs");
        seed_list_backlog_task(
            &runtime,
            "Locking task",
            TaskStatus::InProgress,
            TaskPriority::Medium,
            TaskType::Task,
            None,
            vec!["crates/foo/src/lib.rs"],
        );
        let backlog = seed_list_backlog_task(
            &runtime,
            "Backlog task",
            TaskStatus::Backlog,
            TaskPriority::Medium,
            TaskType::Task,
            None,
            vec!["crates/foo/src/lib.rs"],
        );

        let output = list_backlog_tasks(&runtime, json!({ "task_ids": [backlog.id] }));

        assert_eq!(output["task_count"], json!(1));
        assert_eq!(output["task_ids"], json!([backlog.id]));
        assert!(output.get("excluded").is_none());
    }

    #[test]
    fn resolve_executor_sandbox_returns_none_when_executor_has_no_sandbox() {
        let runtime = seeded_runtime_with_executor(None);
        let resolved = runtime
            .resolve_executor_sandbox("codex", None)
            .expect("resolve");
        assert!(resolved.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_executor_sandbox_returns_descriptor_with_absolutized_modify_paths() {
        let runtime = seeded_runtime_with_executor(Some(
            orbit_common::types::ExecutorSandboxKind::MacosSandboxExec,
        ));
        let resolved = runtime
            .resolve_executor_sandbox("codex", None)
            .expect("resolve")
            .expect("descriptor");
        assert_eq!(
            resolved.kind,
            orbit_common::types::ExecutorSandboxKind::MacosSandboxExec
        );
        let workspace_root = runtime
            .paths()
            .repo_root
            .canonicalize()
            .unwrap_or_else(|_| runtime.paths().repo_root.clone());
        let workspace_str = workspace_root.display().to_string();
        for entry in &resolved.fs_profile.modify {
            let body = entry.strip_prefix('!').unwrap_or(entry);
            assert!(
                body.starts_with('/') || body == workspace_str,
                "modify entry must be absolutized: {entry}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_executor_sandbox_appends_codex_side_write_roots_after_policy_denies() {
        let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
        seed_executor(
            &runtime,
            "codex",
            Some(orbit_common::types::ExecutorSandboxKind::MacosSandboxExec),
        );

        let resolved = runtime
            .resolve_executor_sandbox("codex", None)
            .expect("resolve")
            .expect("descriptor");
        let modify = &resolved.fs_profile.modify;
        let workspace_orbit = runtime
            .paths()
            .orbit_dir
            .canonicalize()
            .unwrap_or_else(|_| runtime.paths().orbit_dir.clone())
            .display()
            .to_string();
        let workspace_orbit_deny = format!("!{workspace_orbit}/**");
        let deny_pos = modify
            .iter()
            .position(|entry| entry == &workspace_orbit_deny)
            .unwrap_or_else(|| {
                panic!(
                    "default policy should deny workspace .orbit writes via {workspace_orbit_deny}; modify={modify:?}"
                )
            });
        let allow_pos = modify
            .iter()
            .rposition(|entry| entry == &workspace_orbit)
            .expect("codex side write root should re-allow workspace .orbit");

        assert!(
            deny_pos < allow_pos,
            "codex side write root must be appended after policy deny: {modify:?}"
        );
        let global_orbit = runtime
            .paths()
            .global_dir
            .canonicalize()
            .unwrap_or_else(|_| runtime.paths().global_dir.clone())
            .display()
            .to_string();
        assert!(
            modify.iter().any(|entry| entry == &global_orbit),
            "codex side write roots should include global .orbit: {modify:?}"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn resolve_executor_sandbox_errors_on_non_macos_platform() {
        let runtime = seeded_runtime_with_executor(Some(
            orbit_common::types::ExecutorSandboxKind::MacosSandboxExec,
        ));
        let err = runtime
            .resolve_executor_sandbox("codex", None)
            .expect_err("expected platform-mismatch error");
        let message = format!("{err}");
        assert!(
            message.contains("macos-sandbox-exec"),
            "error must name the sandbox kind: {message}"
        );
    }

    #[test]
    fn cli_executor_resolution_preserves_registered_static_args() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let now = Utc::now();
        runtime
            .upsert_executor_def(&ExecutorDef {
                name: "codex".to_string(),
                executor_type: ExecutorType::DirectAgent,
                command: Some("codex".to_string()),
                args: vec!["exec".to_string(), "--json".to_string()],
                stdout_format: None,
                models: HashMap::new(),
                timeout_seconds: None,
                env: HashMap::new(),
                sandbox: None,
                allow_fallback: false,
                created_at: now,
                updated_at: now,
            })
            .expect("seed executor");

        let resolved = runtime
            .resolve_cli_executor("codex")
            .expect("resolve codex executor");

        assert_eq!(resolved.command, "codex");
        assert_eq!(resolved.args, ["exec", "--json"]);
    }
}
