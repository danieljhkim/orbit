use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use orbit_common::types::{
    OrbitError, Role, build_task_status_index, optional_string_list_alias, unmet_task_dependencies,
};
use orbit_engine::DispatchError;
use orbit_engine::{StateExecutionContext, execute_deterministic_action};
use orbit_tools::ToolContext;
use serde_json::Value;

use crate::OrbitRuntime;
use crate::runtime::orbit_tool_host::{
    emit_expired_reservation_events, merge_task_lock_conflicts, parse_task_ids,
    requested_task_files, task_lock_conflicts, workspace_orbit_dir, workspace_task_reservation_id,
};

use super::{backlog_exclusion, pipeline_actions};

pub(super) fn run_deterministic(
    runtime: &OrbitRuntime,
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

            runtime
                .run_tool_with_context_and_role(tool_name, args, Role::Admin, tool_context)
                .map_err(|err| DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: format!("{err}"),
                })
        }
        "git_commit" | "git_merge" | "git_push" | "pr_open" | "run_planning_duel"
        | "update_task" | "worktree_setup" => {
            let state_context = StateExecutionContext {
                run_id: input
                    .get("run_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
                ..StateExecutionContext::default()
            };
            execute_deterministic_action(
                runtime,
                action,
                input,
                false,
                &HashMap::new(),
                Some(&state_context),
            )
            .map_err(|err| DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!("{err}"),
            })
        }
        // Retired Phase 4 stubs. These used to return structured skipped
        // success, which made unavailable git/API behavior look like a
        // completed deterministic action. Keep the action names registered
        // so legacy assets fail with an actionable message instead of an
        // "unknown action" error.
        "promote_agent_main" => {
            let target = input
                .get("target_branch")
                .and_then(Value::as_str)
                .unwrap_or("main");
            let source = input
                .get("source_branch")
                .and_then(Value::as_str)
                .unwrap_or("agent-main");
            Err(DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!(
                    "deterministic action `promote_agent_main` is a retired stub; refusing to report promotion from `{source}` to `{target}` as skipped success. Use shipped `git_merge` plus `git_push`, or the `pr_open` workflow, for supported v2 git flow."
                ),
            })
        }
        "revert_on_red" => {
            let sha = input
                .get("commit_sha")
                .and_then(Value::as_str)
                .unwrap_or("");
            Err(DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!(
                    "deterministic action `revert_on_red` is a retired stub; no automatic revert implementation ships today, so commit `{sha}` was not reverted. Use an explicit git revert/manual incident task or add a real deterministic action before wiring this workflow."
                ),
            })
        }
        "context_conflict_check" => {
            let task_ids = parse_task_ids(input).map_err(|error| {
                DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: error.to_string(),
                }
            })?;
            let requested_files = requested_task_files(runtime, &task_ids).map_err(|error| {
                DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: error.to_string(),
                }
            })?;
            let task_conflicts = task_lock_conflicts(runtime, &task_ids, &requested_files)
                .map_err(|error| DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: error.to_string(),
                })?;
            runtime
                .reconcile_stale_owned_reservations_for_files(&requested_files, 32)
                .map_err(|error| DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: error.to_string(),
                })?;
            let reservation_check = runtime
                .stores()
                .task_reservations()
                .check(orbit_store::TaskReservationCheckParams {
                    workspace_orbit_dir: workspace_orbit_dir(runtime),
                    workspace_id: workspace_task_reservation_id(runtime).map_err(|error| {
                        DispatchError::DeterministicActionFailed {
                            action: action.to_string(),
                            message: error.to_string(),
                        }
                    })?,
                    requested_files,
                })
                .map_err(|error| DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: error.to_string(),
                })?;
            emit_expired_reservation_events(runtime, &reservation_check.expired_reservations)
                .map_err(|error| DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: error.to_string(),
                })?;
            let conflicts = merge_task_lock_conflicts(task_conflicts, reservation_check.conflicts);
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
        // Filters by `status: backlog`; legacy untriaged
        // `status: friction` reports remain absent. In automatic
        // mode, drops any backlog task group whose context overlaps files
        // already held by `in-progress`/`review` tasks. Sorts critical →
        // high → medium → low then by `created_at` ascending so older
        // high-priority work ships first. Caps at `max_tasks` (default 50).
        "list_backlog_tasks" => backlog_exclusion::list_backlog_tasks(runtime, action, input),
        // Materialize an epic's working set for the orchestrator:
        // the epic task itself plus non-terminal subtasks
        // (`parent_id == epic_task_id` and status not done, review,
        // blocked, archived, or rejected).
        // Full descriptions ride along because the orchestrator
        // reasons about dependency ordering from prose.
        "load_epic" => backlog_exclusion::load_epic(runtime, action, input),
        // Fold the deterministic final task-state snapshot into counters
        // + a human-readable one-liner. Pure aggregation — the
        // orchestrator's final response is audit-only.
        "summarize_epic" => backlog_exclusion::summarize_epic(input),
        // Guard the auto-dispatch bundle output before fan_out.
        // Rejects duplicated task_ids, unknown ids, and oversize
        // bundles with a structured error so a misgrouped backlog
        // never silently dispatches.
        "validate_bundles" => pipeline_actions::validate_bundles(action, input),
        // Thin passthrough over `orbit.task.locks.reserve`. Exists as a
        // dedicated action (rather than a `orbit_tool_call` config) so a
        // workflow inside a `loop:` with `break_when:` can reference
        // `steps.<id>.output.reserved` directly without leaking the
        // generic `{tool_name, args}` envelope into the activity's
        // input_schema.
        "reserve_locks" => {
            let waiting_on_deps =
                unmet_dependency_ids_for_input(runtime, input).map_err(|err| {
                    DispatchError::DeterministicActionFailed {
                        action: action.to_string(),
                        message: format!("{err}"),
                    }
                })?;
            if !waiting_on_deps.is_empty() {
                update_run_waiting_reasons(
                    runtime,
                    input,
                    Some(waiting_on_deps.clone()),
                    None,
                    action,
                )?;
                return Ok(serde_json::json!({
                    "reserved": false,
                    "waiting_on_deps": waiting_on_deps,
                    "conflicts": [],
                }));
            }

            let output = runtime
                .run_tool_with_context_and_role(
                    "orbit.task.locks.reserve",
                    input.clone(),
                    Role::Admin,
                    tool_context,
                )
                .map_err(|err| DispatchError::DeterministicActionFailed {
                    action: action.to_string(),
                    message: format!("{err}"),
                })?;
            let waiting_on_locks = waiting_locks_from_reserve_output(&output);
            update_run_waiting_reasons(runtime, input, None, non_empty(waiting_on_locks), action)?;
            Ok(output)
        }
        // Thin passthrough over `orbit.task.locks.release` so workflows
        // can free admission-window reservations after child runs finish.
        "release_locks" => runtime
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
            pipeline_actions::invoke_and_wait(runtime, action, input, tool_context)
        }
        // Join already-submitted child v2 Jobs without keeping the
        // dispatching agent activity open.
        "pipeline_wait" => pipeline_actions::pipeline_wait(runtime, action, input, tool_context),
        // Fail a workflow if one or more child pipeline wait results did not
        // reach `succeeded`.
        "pipeline_success_guard" => pipeline_actions::pipeline_success_guard(action, input),
        // Post-loop gate signal: the admission window never opened in
        // time. Emits a `gate.starvation` audit event with task_ids and
        // conflicting_files so an epic-orchestrator parent can decide
        // to replan, then fails the Run with a structured error.
        "gate_starvation_fail" => pipeline_actions::gate_starvation_fail(runtime, action, input),
        other => Err(DispatchError::DeterministicActionNotRegistered(
            other.to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::task::TaskAddParams;
    use chrono::Utc;
    use orbit_common::types::{PipelineState, TaskPriority, TaskStatus, TaskType};
    use orbit_engine::V2RuntimeHost;
    use orbit_tools::ToolContext;
    use serde_json::json;

    fn seed_task(
        runtime: &OrbitRuntime,
        title: &str,
        status: TaskStatus,
        dependencies: Vec<String>,
    ) -> String {
        runtime
            .add_task(TaskAddParams {
                title: title.to_string(),
                description: format!("Fixture task: {title}"),
                acceptance_criteria: vec!["Fixture task is observable.".to_string()],
                dependencies,
                plan: "Fixture plan.".to_string(),
                workspace_path: Some(".".to_string()),
                priority: TaskPriority::Medium,
                task_type: Some(TaskType::Chore),
                status: Some(status),
                ..Default::default()
            })
            .expect("seed task")
            .id
    }

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
    fn pipeline_wait_is_registered_for_v2_deterministic_dispatch() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let err = runtime
            .run_deterministic(
                "pipeline_wait",
                &json!({}),
                &json!({}),
                ToolContext::default(),
            )
            .expect_err("empty input should fail validation inside the action");

        match err {
            DispatchError::DeterministicActionFailed { action, message } => {
                assert_eq!(action, "pipeline_wait");
                assert!(
                    message.contains("missing `run_ids`"),
                    "unexpected validation message: {message}"
                );
            }
            other => panic!("expected registered action failure, got {other}"),
        }
    }

    #[test]
    fn promote_agent_main_stub_is_loudly_fenced() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let err = runtime
            .run_deterministic(
                "promote_agent_main",
                &json!({}),
                &json!({
                    "source_branch": "agent-main",
                    "target_branch": "main",
                }),
                ToolContext::default(),
            )
            .expect_err("retired promotion stub should fail loudly");

        match err {
            DispatchError::DeterministicActionFailed { action, message } => {
                assert_eq!(action, "promote_agent_main");
                assert!(message.contains("retired stub"), "{message}");
                assert!(message.contains("git_merge"), "{message}");
                assert!(message.contains("git_push"), "{message}");
            }
            other => panic!("expected registered action failure, got {other}"),
        }
    }

    #[test]
    fn revert_on_red_stub_is_loudly_fenced() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let err = runtime
            .run_deterministic(
                "revert_on_red",
                &json!({}),
                &json!({
                    "commit_sha": "abc123",
                    "branch": "agent-main",
                }),
                ToolContext::default(),
            )
            .expect_err("retired revert stub should fail loudly");

        match err {
            DispatchError::DeterministicActionFailed { action, message } => {
                assert_eq!(action, "revert_on_red");
                assert!(message.contains("retired stub"), "{message}");
                assert!(message.contains("manual incident task"), "{message}");
            }
            other => panic!("expected registered action failure, got {other}"),
        }
    }

    #[test]
    fn reserve_locks_records_unmet_dependencies_in_run_state() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let dependency = seed_task(&runtime, "Dependency", TaskStatus::Backlog, Vec::new());
        let blocked = seed_task(
            &runtime,
            "Blocked",
            TaskStatus::Backlog,
            vec![dependency.clone()],
        );
        let run = runtime
            .stores()
            .jobs()
            .insert_run("task_gate_pipeline", 1, Utc::now(), Some(json!({})), None)
            .expect("insert run");
        runtime
            .stores()
            .jobs()
            .write_run_state(
                &run.run_id,
                &PipelineState::new(run.run_id.clone(), run.job_id.clone(), json!({})),
            )
            .expect("write state");

        let output = runtime
            .run_deterministic(
                "reserve_locks",
                &json!({}),
                &json!({
                    "run_id": run.run_id,
                    "task_ids": [blocked],
                }),
                ToolContext::default(),
            )
            .expect("reserve locks");

        assert_eq!(output["reserved"], json!(false));
        assert_eq!(output["waiting_on_deps"], json!([dependency]));
        let state = runtime
            .read_run_state(&run.run_id)
            .expect("read run state")
            .expect("state exists");
        assert_eq!(state.waiting_on_deps, Some(vec![dependency]));
        assert_eq!(state.waiting_on_locks, None);
    }

    #[test]
    fn waiting_locks_from_reserve_output_extracts_unique_conflict_files() {
        let locks = waiting_locks_from_reserve_output(&json!({
            "reserved": false,
            "conflicts": [
                { "file": "file:src/lib.rs", "held_by_id": "ORB-1" },
                { "file": "file:src/lib.rs", "held_by_id": "reservation-1" },
                { "file": "dir:crates/orbit-core/src", "held_by_id": "ORB-2" }
            ],
        }));

        assert_eq!(
            locks,
            vec![
                "dir:crates/orbit-core/src".to_string(),
                "file:src/lib.rs".to_string()
            ]
        );
    }
}

fn unmet_dependency_ids_for_input(
    runtime: &OrbitRuntime,
    input: &Value,
) -> Result<Vec<String>, OrbitError> {
    let Some(raw_task_ids) =
        optional_string_list_alias(input, &["task_ids", "taskIds", "task-ids"])?
    else {
        return Ok(Vec::new());
    };
    let task_ids = parse_task_ids(&serde_json::json!({ "task_ids": raw_task_ids }))?;
    let tasks = runtime.stores().tasks().list()?;
    let status_by_id = build_task_status_index(&tasks);
    let task_by_id = tasks
        .into_iter()
        .map(|task| (task.id.clone(), task))
        .collect::<BTreeMap<_, _>>();
    let mut unmet = BTreeSet::new();
    for task_id in task_ids {
        let task = task_by_id
            .get(&task_id)
            .ok_or_else(|| OrbitError::not_found(crate::NotFoundKind::Task, task_id.clone()))?;
        for dependency in unmet_task_dependencies(task, &status_by_id) {
            unmet.insert(dependency.id);
        }
    }
    Ok(unmet.into_iter().collect())
}

fn waiting_locks_from_reserve_output(output: &Value) -> Vec<String> {
    output
        .get("conflicts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|conflict| conflict.get("file").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn update_run_waiting_reasons(
    runtime: &OrbitRuntime,
    input: &Value,
    waiting_on_deps: Option<Vec<String>>,
    waiting_on_locks: Option<Vec<String>>,
    action: &str,
) -> Result<(), DispatchError> {
    let Some(run_id) = input.get("run_id").and_then(Value::as_str) else {
        return Ok(());
    };
    let Some(mut state) =
        runtime
            .read_run_state(run_id)
            .map_err(|err| DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: format!("{err}"),
            })?
    else {
        return Ok(());
    };
    state.set_waiting_reasons(waiting_on_deps, waiting_on_locks);
    runtime
        .stores()
        .jobs()
        .write_run_state(run_id, &state)
        .map_err(|err| DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("{err}"),
        })
}

fn non_empty(values: Vec<String>) -> Option<Vec<String>> {
    (!values.is_empty()).then_some(values)
}
