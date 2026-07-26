use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{Duration, Instant};

use orbit_common::types::{OrbitError, Role, optional_string_list_alias, unmet_task_dependencies};
use orbit_engine::DispatchError;
use orbit_engine::{StateExecutionContext, execute_deterministic_action};
use orbit_tools::ToolContext;
use serde_json::Value;

use crate::OrbitRuntime;
use crate::runtime::task_locks::{
    emit_expired_reservation_events, merge_task_lock_conflicts, parse_task_ids,
    requested_task_files, task_lock_conflicts, workspace_orbit_dir, workspace_task_reservation_id,
};

use super::{backlog_exclusion, pipeline_actions, triage};

/// Every deterministic action name this dispatch table can execute.
///
/// [ORB-10385] This is the runtime's advertised capability list. It backs
/// [`is_deterministic_action_registered`], which job validation consults
/// before a run's first step so a catalog asset naming an action this binary
/// does not implement fails admission instead of a terminal hook. Retired
/// stubs (`promote_agent_main`, `revert_on_red`) stay listed on purpose: they
/// *are* dispatchable and answer with an actionable retirement message.
///
/// Adding an arm below without adding its name here makes validation reject a
/// job the runtime could actually run; adding a name here without an arm
/// reintroduces exactly the skew this list exists to prevent. The seeded-asset
/// coverage tests in `command/job/catalog.rs` pin the direction that broke in
/// production — every shipped deterministic activity's action must be
/// registered here — and [ORB-10410] adds behavioral cover for the two actions
/// the skew actually reached: `pr_failure_handoff` (the `task_pr_pipeline`
/// terminal hook, in `command/job/exec.rs`) and `worktree_gc` (below).
pub(super) const REGISTERED_DETERMINISTIC_ACTIONS: &[&str] = &[
    "apply_triage_dispositions",
    "context_conflict_check",
    "gate_starvation_fail",
    "git_commit",
    "git_merge",
    "git_push",
    "git_rebase",
    "independent_review_guard",
    "invoke_and_wait",
    "list_backlog_tasks",
    "list_triage_candidates",
    "load_epic",
    "orbit_tool_call",
    "pipeline_success_guard",
    "pr_failure_handoff",
    "pr_open",
    "pr_prepare",
    "pr_promote",
    "promote_agent_main",
    "release_locks",
    "reserve_locks",
    "resolve_workspace_ship_input",
    "revert_on_red",
    "run_auto_task_scheduler",
    "run_planning_duel",
    "sleep",
    "summarize_epic",
    "update_task",
    "validate_bundles",
    "worktree_gc",
    "worktree_setup",
];

/// Whether `action` is dispatchable by this runtime — the capability probe
/// behind `V2RuntimeHost::has_deterministic_action` [ORB-10385].
pub(super) fn is_deterministic_action_registered(action: &str) -> bool {
    REGISTERED_DETERMINISTIC_ACTIONS.contains(&action)
}

pub(super) fn run_deterministic(
    runtime: &OrbitRuntime,
    action: &str,
    config: &Value,
    input: &Value,
    tool_context: ToolContext,
) -> Result<Value, DispatchError> {
    // Reject anything the advertised capability list does not claim, so
    // `has_deterministic_action` can never report an action this function then
    // refuses [ORB-10385].
    if !is_deterministic_action_registered(action) {
        return Err(DispatchError::DeterministicActionNotRegistered(
            action.to_string(),
        ));
    }
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
        // Forwarded to orbit-engine's automation registry. `pr_failure_handoff`
        // and `worktree_gc` were shipped as activity assets and referenced by
        // `task_pr_pipeline` / `worktree_gc_pipeline` while this arm still
        // omitted them, so both dispatched as "not registered" — the skew
        // ORB-10385 fixes. Keep this list in sync with
        // `REGISTERED_DETERMINISTIC_ACTIONS`.
        "git_commit" | "git_merge" | "git_push" | "git_rebase" | "pr_failure_handoff"
        | "pr_open" | "pr_prepare" | "pr_promote" | "run_planning_duel" | "update_task"
        | "worktree_gc" | "worktree_setup" => {
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
                .check_task_reservation_conflicts(orbit_store::TaskReservationCheckParams {
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
        // Fire every due, enabled auto-task definition and mint a task from
        // its template [ORB-10149]. Reads definitions from this workspace's
        // `.orbit/auto_tasks/`; catch-up collapses and `skip_if_open` dedupe
        // are enforced in the scheduler core.
        "run_auto_task_scheduler" => crate::auto_tasks::run_scheduler_action_json(runtime, input)
            .map_err(|error| DispatchError::DeterministicActionFailed {
                action: action.to_string(),
                message: error.to_string(),
            }),
        // ADR-0223: scheduled shipment resolves only the active runtime's
        // canonical ship input; cross-workspace enumeration stays in the
        // legacy CLI sweep and `workflow.auto_ship` is deliberately ignored.
        "resolve_workspace_ship_input" => resolve_workspace_ship_input(runtime, action),
        // Materialize the workspace backlog for auto-dispatch.
        // Filters by `status: backlog`. In automatic mode, drops any backlog
        // task group whose context overlaps files
        // already held by `in-progress`/`review` tasks. Sorts critical →
        // high → medium → low then by `created_at` ascending so older
        // high-priority work ships first. Caps at `max_tasks` (default 50).
        "list_backlog_tasks" => backlog_exclusion::list_backlog_tasks(runtime, action, input),
        // Materialize blocked tasks attributable to a terminally-failed job
        // run for the triage pipeline [ORB-10129]. Human-blocked tasks (no
        // `job_run_id`, or a non-failed run) never appear; tasks whose
        // re-backlog budget is exhausted take the gave-up path here.
        "list_triage_candidates" => triage::list_triage_candidates(runtime, action, input),
        // Apply the triage agent's per-task verdicts under deterministic
        // bounds: candidates-only, `environmental`-only re-backlog, durable
        // re-backlog budget, idempotent under overlap [ORB-10129].
        "apply_triage_dispositions" => triage::apply_triage_dispositions(runtime, action, input),
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
        // Fail a workflow if one or more child pipeline wait results did not
        // reach `succeeded`.
        "pipeline_success_guard" => pipeline_actions::pipeline_success_guard(action, input),
        // Require the independent reviewer to return a structured verdict for
        // the exact candidate SHA snapshotted by the PR pipeline.
        "independent_review_guard" => pipeline_actions::independent_review_guard(action, input),
        // Post-loop gate signal: the admission window never opened in
        // time. Emits a `gate.starvation` audit event with task_ids and
        // conflicting_files so an epic-orchestrator parent can decide
        // to replan, then fails the Run with a structured error.
        "gate_starvation_fail" => pipeline_actions::gate_starvation_fail(runtime, action, input),
        other => {
            // Unreachable for a well-formed table: the guard above already
            // rejected every unlisted name. Reaching here means a name was
            // added to `REGISTERED_DETERMINISTIC_ACTIONS` without an arm —
            // the capability list over-promising [ORB-10385].
            debug_assert!(
                !is_deterministic_action_registered(other),
                "`{other}` is in REGISTERED_DETERMINISTIC_ACTIONS but has no dispatch arm"
            );
            Err(DispatchError::DeterministicActionNotRegistered(
                other.to_string(),
            ))
        }
    }
}

fn resolve_workspace_ship_input(
    runtime: &OrbitRuntime,
    action: &str,
) -> Result<Value, DispatchError> {
    if let Some(binding) = runtime.workspace_runtime_binding() {
        return crate::command::workflow::build_ship_input(
            binding.ship_mode,
            runtime.workflow_base_branch(),
            &[],
            false,
            None,
        )
        .map_err(|error| DispatchError::DeterministicActionFailed {
            action: action.to_string(),
            message: format!("resolve workspace ship input: {error}"),
        });
    }

    crate::command::workflow::build_ship_input(
        crate::command::workflow::ShipMode::Local,
        runtime.workflow_base_branch(),
        &[],
        false,
        None,
    )
    .map_err(|error| DispatchError::DeterministicActionFailed {
        action: action.to_string(),
        message: format!("resolve workspace ship input: {error}"),
    })
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
    let tasks = runtime.stores().tasks().list_tasks()?;
    let status_by_id = runtime.task_status_index()?;
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

// `pub(super)` (not private): the sibling `tests/dispatch.rs` unit-tests this
// pure parsing helper directly rather than through a full lock-conflict
// integration setup — see docs/design-patterns/test_layout.md migration
// recipe step 6.
pub(super) fn waiting_locks_from_reserve_output(output: &Value) -> Vec<String> {
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
