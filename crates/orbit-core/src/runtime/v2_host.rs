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

use orbit_common::types::Task;
use orbit_common::types::{
    AuditEventStatus, Role, TaskStatus, TaskType, UNRESTRICTED_FS_PROFILE,
    prune_missing_context_files,
};
use orbit_common::utility::path::workspace_relative_paths_overlap;
use orbit_common::utility::selector::canonical_selector_in_workspace;
use orbit_engine::activity_job::{DispatchError, V2RuntimeHost};
use orbit_engine::{StateExecutionContext, execute_deterministic_action};
use orbit_store::AuditEventInsertParams;
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
            "git_merge" | "git_push" | "pr_open" | "run_planning_duel" | "update_task"
            | "worktree_setup" => execute_deterministic_action(
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
            // Materialize the workspace backlog for `dispatch_agent`.
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
                let subtask_payload: Vec<Value> = subtasks
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
                }))
            }
            // Fold the orchestrator's final state snapshot into counters
            // + a human-readable one-liner. Pure aggregation — the
            // decision history already lives in `orbit.state` per the
            // role prompt.
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
            // Guard the `dispatch_agent`'s bundle output before fan_out.
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

    fn resolve_cli_command(&self, provider: &str) -> Result<String, DispatchError> {
        resolve_cli_command(provider)
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

/// Map a v2 provider name to the CLI command that dispatches it. Env-var
/// overrides (`ORBIT_V2_CLI_<PROVIDER>`) let smokes substitute a fixture
/// binary for the real provider CLI; production defaults to the provider
/// name itself (`claude`, `codex`, `gemini`, `ollama`) which resolves via
/// `$PATH`.
fn resolve_cli_command(provider: &str) -> Result<String, DispatchError> {
    let env_key = format!("ORBIT_V2_CLI_{}", provider.to_ascii_uppercase());
    if let Ok(value) = std::env::var(&env_key)
        && !value.is_empty()
    {
        return Ok(value);
    }
    match provider {
        "claude" | "codex" | "gemini" | "ollama" => Ok(provider.to_string()),
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
    use orbit_engine::activity_job::V2RuntimeHost;
    use orbit_tools::ToolContext;
    use serde_json::json;

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
}
