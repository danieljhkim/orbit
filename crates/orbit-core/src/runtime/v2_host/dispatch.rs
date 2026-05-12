use std::collections::HashMap;
use std::time::{Duration, Instant};

use orbit_common::types::Role;
use orbit_engine::activity_job::DispatchError;
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
        | "update_task" | "worktree_setup" => execute_deterministic_action(
            runtime,
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
        "reserve_locks" => runtime
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
}
