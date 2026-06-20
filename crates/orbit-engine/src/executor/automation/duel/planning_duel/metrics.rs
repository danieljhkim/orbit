use orbit_common::types::{
    OrbitError, PlanningDuelRun, PlanningEfficiency, PlanningOutcome, PlanningRoleAssignment,
    PlanningRoles, RoleSlot,
};
use orbit_store::planning_duel_scoreboard;
use serde_json::{Value, json};

use crate::context::{ActivityInvocationResult, RuntimeHost, TaskHost};
use crate::executor::automation::input::{input_string_field, required_input_string};

use super::artifacts::{
    plan_artifact_by_path, plan_artifact_for_assignment, planning_duel_plan_artifacts,
    winner_artifact_from_artifacts, winner_assignment, winner_slot_for_assignment,
};
use super::types::{PlanningDuelEfficiency, PlanningDuelRoleMetrics, into_efficiency_metrics};

fn efficiency_from_invocation(invocation: &ActivityInvocationResult) -> PlanningDuelEfficiency {
    let trace = &invocation.invocation_trace;
    PlanningDuelEfficiency {
        invocation_count: 1,
        // The direct activity runner already records the authoritative elapsed
        // wall clock separately from the invocation trace payload.
        wall_clock_ms: invocation.duration_ms,
        tool_call_count: trace.tool_calls.len() as u64,
        input_tokens: trace.usage.input,
        cache_read_tokens: trace.usage.cache_read,
        cache_create_tokens: trace.usage.cache_create,
        output_tokens: trace.usage.output,
        total_tokens: trace
            .usage
            .input
            .saturating_add(trace.usage.cache_read)
            .saturating_add(trace.usage.cache_create)
            .saturating_add(trace.usage.output),
        byte_proxy_total: trace.tool_calls.iter().map(|call| call.result_bytes).sum(),
    }
}

pub(super) fn role_metrics_from_invocation(
    role: &PlanningRoleAssignment,
    slot: RoleSlot,
    activity_id: &str,
    invocation: &ActivityInvocationResult,
) -> PlanningDuelRoleMetrics {
    PlanningDuelRoleMetrics {
        family: role.family,
        slot,
        activity_id: activity_id.to_string(),
        efficiency: efficiency_from_invocation(invocation),
    }
}

pub(super) fn record_planning_duel_scores<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let job_run_id = input_string_field(input, "job_run_id")
        .or_else(|| input_string_field(input, "run_id"))
        .ok_or_else(|| OrbitError::InvalidInput("missing required input.job_run_id".to_string()))?;

    let planner_a_role = serde_json::from_value::<PlanningDuelRoleMetrics>(
        input
            .get("roles")
            .and_then(|roles| roles.get("planner_a"))
            .cloned()
            .ok_or_else(|| {
                OrbitError::InvalidInput("missing required input.roles.planner_a".to_string())
            })?,
    )
    .map_err(|err| OrbitError::InvalidInput(format!("invalid roles.planner_a payload: {err}")))?;
    let planner_b_role = serde_json::from_value::<PlanningDuelRoleMetrics>(
        input
            .get("roles")
            .and_then(|roles| roles.get("planner_b"))
            .cloned()
            .ok_or_else(|| {
                OrbitError::InvalidInput("missing required input.roles.planner_b".to_string())
            })?,
    )
    .map_err(|err| OrbitError::InvalidInput(format!("invalid roles.planner_b payload: {err}")))?;
    let arbiter_role = serde_json::from_value::<PlanningDuelRoleMetrics>(
        input
            .get("roles")
            .and_then(|roles| roles.get("arbiter"))
            .cloned()
            .ok_or_else(|| {
                OrbitError::InvalidInput("missing required input.roles.arbiter".to_string())
            })?,
    )
    .map_err(|err| OrbitError::InvalidInput(format!("invalid roles.arbiter payload: {err}")))?;

    let PlanningDuelRoleMetrics {
        family: planner_a_family,
        slot: _,
        activity_id: _,
        efficiency: planner_a_efficiency,
    } = planner_a_role;
    let PlanningDuelRoleMetrics {
        family: planner_b_family,
        slot: _,
        activity_id: _,
        efficiency: planner_b_efficiency,
    } = planner_b_role;
    let PlanningDuelRoleMetrics {
        family: arbiter_family,
        slot: _,
        activity_id: _,
        efficiency: arbiter_efficiency,
    } = arbiter_role;
    let roles = PlanningRoles {
        planner_a: PlanningRoleAssignment {
            family: planner_a_family,
        },
        planner_b: PlanningRoleAssignment {
            family: planner_b_family,
        },
        arbiter: PlanningRoleAssignment {
            family: arbiter_family,
        },
    };
    let artifacts = host.get_task_artifacts(task_id)?;
    let plan_artifacts = planning_duel_plan_artifacts(&artifacts)?;
    let winner = winner_artifact_from_artifacts(&artifacts, Some(&roles))?;
    let winner_assignment = winner_assignment(&winner);
    let winner_plan = plan_artifact_by_path(&plan_artifacts, &winner.artifact_path)?;
    if winner_plan.author.family != winner_assignment.family {
        return Err(OrbitError::InvalidInput(format!(
            "winner artifact `{}` is authored by {} instead of declared winner {}",
            winner.artifact_path, winner_plan.author.family, winner_assignment.family
        )));
    }
    let winner_slot = winner_slot_for_assignment(&roles, &winner_assignment)?;
    if winner.arbiter_family != roles.arbiter.family {
        return Err(OrbitError::InvalidInput(format!(
            "winner artifact arbiter {} does not match recorded arbiter {}",
            winner.arbiter_family, roles.arbiter.family
        )));
    }
    let planner_a_artifact_path =
        plan_artifact_for_assignment(&plan_artifacts, &roles.planner_a, RoleSlot::PlannerA)?
            .path
            .clone();
    let planner_b_artifact_path =
        plan_artifact_for_assignment(&plan_artifacts, &roles.planner_b, RoleSlot::PlannerB)?
            .path
            .clone();

    let completed_at = chrono::Utc::now();
    let run = PlanningDuelRun {
        run_id: job_run_id,
        task_id: task_id.to_string(),
        completed_at,
        roles,
        planner_a_artifact_path,
        planner_b_artifact_path,
        outcome: PlanningOutcome {
            winner: winner_slot.planner_slot().ok_or_else(|| {
                OrbitError::InvalidInput("planning duel winner cannot be arbiter".to_string())
            })?,
            arbiter_rationale: winner.arbiter_rationale,
        },
        efficiency: PlanningEfficiency {
            planner_a: into_efficiency_metrics(planner_a_efficiency),
            planner_b: into_efficiency_metrics(planner_b_efficiency),
            arbiter: into_efficiency_metrics(arbiter_efficiency),
        },
    };

    planning_duel_scoreboard::append_run(host.scoreboard_dir(), &run)?;

    Ok(json!({
        "run_id": run.run_id,
        "recorded": true,
    }))
}
