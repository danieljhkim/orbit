use std::collections::BTreeMap;

use orbit_types::OrbitError;
use serde_json::{Value, json};

use super::{artifacts, metrics, roles};
use crate::context::{ActivityInvocationResult, RuntimeHost, TaskHost};
use crate::executor::automation::input::{input_string_field, required_input_string};

fn join_activity_result(
    result: std::thread::Result<Result<ActivityInvocationResult, OrbitError>>,
    label: &str,
) -> Result<ActivityInvocationResult, OrbitError> {
    match result {
        Ok(inner) => inner,
        Err(_) => Err(OrbitError::Execution(format!(
            "{label} activity thread panicked"
        ))),
    }
}

pub(crate) fn run_planning_duel<H: RuntimeHost + TaskHost + Sync + ?Sized>(
    host: &H,
    input: &Value,
    debug: bool,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let job_run_id = input_string_field(input, "job_run_id")
        .or_else(|| input_string_field(input, "run_id"))
        .ok_or_else(|| OrbitError::InvalidInput("missing required input.run_id".to_string()))?;

    artifacts::cleanup_stale_planning_duel_artifacts(host, task_id)?;

    let roles_output = roles::select_planning_duel_roles(host, &json!({ "task_id": task_id }))?;
    let planning_roles = roles::parse_planning_duel_roles(&roles_output)?;

    let planner_activity = roles::planner_activity();
    let planner_a_input = roles::planner_input(task_id);
    let planner_b_input = roles::planner_input(task_id);
    let (planner_a_result, planner_b_result) = std::thread::scope(|scope| {
        let planner_a = planning_roles.planner_a.clone();
        let planner_b = planning_roles.planner_b.clone();
        let planner_activity_a = planner_activity.clone();
        let planner_activity_b = planner_activity.clone();
        let handle_a = scope.spawn(move || {
            host.invoke_activity(
                planner_activity_a,
                &planner_a.agent,
                Some(planner_a.model.as_str()),
                planner_a_input,
                roles::PLANNER_TIMEOUT_SECONDS,
                debug,
            )
        });
        let handle_b = scope.spawn(move || {
            host.invoke_activity(
                planner_activity_b,
                &planner_b.agent,
                Some(planner_b.model.as_str()),
                planner_b_input,
                roles::PLANNER_TIMEOUT_SECONDS,
                debug,
            )
        });
        (
            join_activity_result(handle_a.join(), "planner_a"),
            join_activity_result(handle_b.join(), "planner_b"),
        )
    });
    let planner_a_result = planner_a_result?;
    let planner_b_result = planner_b_result?;

    let planner_artifacts = host.get_task_artifacts(task_id)?;
    let plan_artifacts = artifacts::planning_duel_plan_artifacts(&planner_artifacts)?;
    let _ = artifacts::plan_artifact_for_assignment(&plan_artifacts, &planning_roles.planner_a)?;
    let _ = artifacts::plan_artifact_for_assignment(&plan_artifacts, &planning_roles.planner_b)?;

    let arbiter_result = host.invoke_activity(
        roles::arbiter_activity(),
        &planning_roles.arbiter.agent,
        Some(planning_roles.arbiter.model.as_str()),
        roles::arbiter_input(task_id),
        roles::ARBITER_TIMEOUT_SECONDS,
        debug,
    )?;

    let artifacts_after_arbiter = host.get_task_artifacts(task_id)?;
    let winner = artifacts::winner_artifact_from_artifacts(&artifacts_after_arbiter)?;

    let role_metrics = BTreeMap::from([
        (
            "planner_a".to_string(),
            metrics::role_metrics_from_invocation(
                &planning_roles.planner_a,
                roles::PLANNER_ACTIVITY_ID,
                &planner_a_result,
            ),
        ),
        (
            "planner_b".to_string(),
            metrics::role_metrics_from_invocation(
                &planning_roles.planner_b,
                roles::PLANNER_ACTIVITY_ID,
                &planner_b_result,
            ),
        ),
        (
            "arbiter".to_string(),
            metrics::role_metrics_from_invocation(
                &planning_roles.arbiter,
                roles::ARBITER_ACTIVITY_ID,
                &arbiter_result,
            ),
        ),
    ]);

    let _ = artifacts::writeback_planning_duel_task(
        host,
        &json!({
            "task_id": task_id,
            "planning_duel_roles": roles_output["planning_duel_roles"].clone(),
        }),
    )?;
    let _ = metrics::record_planning_duel_scores(
        host,
        &json!({
            "task_id": task_id,
            "job_run_id": job_run_id,
            "roles": role_metrics,
        }),
    )?;

    Ok(json!({
        "task_id": task_id,
        "run_id": job_run_id,
        "winner_agent_cli": winner.winner_agent_cli,
        "winner_model": winner.winner_model,
        "recorded": true,
    }))
}
