//! `impl V2RuntimeHost for OrbitRuntime` — the orbit-core side of the v2
//! dispatch boundary.
//!
//! The trait surface is deliberately small: orbit-core owns deterministic
//! action dispatch (which needs the live `ToolContext` + tool registry),
//! provider credential sourcing (env / config access), and the CLI-command
//! resolution for `backend: cli` (workspace-scoped env / config overrides).
//! HTTP agent-loop transport and CLI subprocess execution both live in
//! `orbit-engine`, so this module never names orbit-agent types.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
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
use serde_json::Value;

use super::orbit_tool_host::{
    emit_expired_reservation_events, merge_task_lock_conflicts, parse_task_ids,
    requested_task_files, task_lock_conflicts, workspace_orbit_dir,
};
use crate::OrbitRuntime;
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
            // Filters `status: backlog`, excludes `type: friction`
            // (per CLAUDE.md: friction is reserved for agent self-reports,
            // not shippable work), and in automatic mode drops any backlog
            // task group whose context overlaps files already held by
            // `in-progress`/`review` tasks. Sorts critical → high → medium →
            // low then by `created_at` ascending so older high-priority work
            // ships first. Caps at `max_tasks` (default 50).
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
                let mut tasks = if explicit_task_ids.is_empty() {
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
                    let locked_files = active_task_lock_files(task_lookup.values());
                    let mut backlog: Vec<Task> = all_tasks
                        .into_iter()
                        .filter(|task| {
                            task.status == TaskStatus::Backlog && !task.task_type.is_friction()
                        })
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
                    if !locked_files.is_empty() {
                        let tainted_roots: BTreeSet<String> = backlog
                            .iter()
                            .filter(|task| task_overlaps_locked_files(task, &locked_files))
                            .map(|task| task_root_id(task, &task_lookup))
                            .collect();
                        if !tainted_roots.is_empty() {
                            backlog.retain(|task| {
                                !tainted_roots.contains(&task_root_id(task, &task_lookup))
                            });
                        }
                    }
                    backlog
                } else {
                    explicit_task_ids
                        .iter()
                        .map(|task_id| {
                            self.get_task(task_id).map_err(|err| {
                                DispatchError::DeterministicActionFailed {
                                    action: action.to_string(),
                                    message: format!("load task {task_id}: {err}"),
                                }
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?
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
                Ok(serde_json::json!({
                    "task_count": task_objs.len(),
                    "task_ids": ids,
                    "tasks": task_objs,
                    "bundles": bundles,
                }))
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

fn active_task_lock_files<'a>(tasks: impl IntoIterator<Item = &'a Task>) -> BTreeSet<String> {
    let mut locked_files = BTreeSet::new();
    for task in tasks {
        if matches!(task.status, TaskStatus::InProgress | TaskStatus::Review) {
            locked_files.extend(existing_lock_context_files(task));
        }
    }
    locked_files
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

fn task_overlaps_locked_files(task: &Task, locked_files: &BTreeSet<String>) -> bool {
    existing_lock_context_files(task)
        .iter()
        .any(|requested_file| {
            locked_files
                .iter()
                .any(|held_file| workspace_relative_paths_overlap(requested_file, held_file))
        })
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
    use orbit_common::types::{ExecutorDef, ExecutorType};
    use orbit_engine::activity_job::V2RuntimeHost;
    use orbit_tools::ToolContext;
    use serde_json::json;
    use std::collections::HashMap;
    #[cfg(target_os = "macos")]
    use tempfile::tempdir;

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

    #[cfg(target_os = "macos")]
    fn runtime_with_workspace_layout() -> (tempfile::TempDir, OrbitRuntime) {
        let root = tempdir().expect("create tempdir");
        let global = root.path().join("home/.orbit");
        let workspace = root.path().join("repo/.orbit");
        std::fs::create_dir_all(&global).expect("global orbit dir");
        std::fs::create_dir_all(&workspace).expect("workspace orbit dir");
        let runtime = OrbitRuntime::from_roots(&global, &workspace).expect("build runtime");
        (root, runtime)
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
        let (_root, runtime) = runtime_with_workspace_layout();
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
