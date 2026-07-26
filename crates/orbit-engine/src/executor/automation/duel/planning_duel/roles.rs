use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use orbit_common::types::{
    AgentFamily, OrbitError, PlanningRoleAssignment, PlanningRoles, RoleSlot,
};
use serde_json::{Value, json};

use crate::context::RuntimeHost;

use crate::executor::automation::input::{input_string_field, required_input_string};

use super::super::{role_permutation_at, validate_role_permutation};

pub(super) const PLANNER_ACTIVITY_ID: &str = "propose_duel_plan";
pub(super) const ARBITER_ACTIVITY_ID: &str = "arbitrate_duel_plan";

thread_local! {
    static TEST_PERMUTATION_QUEUE: RefCell<VecDeque<[usize; 3]>> =
        const { RefCell::new(VecDeque::new()) };
}

fn next_permutation<H: RuntimeHost + ?Sized>(host: &H) -> Result<[usize; 3], OrbitError> {
    let family_count = host.duel_candidate_families().len();
    let from_test = TEST_PERMUTATION_QUEUE.with(|cell| cell.borrow_mut().pop_front());
    if let Some(perm) = from_test {
        return validate_role_permutation(perm, family_count, "select_planning_duel_roles");
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    role_permutation_at(family_count, nanos as usize)
}

fn orchestrator_model_for<H: RuntimeHost + ?Sized>(
    host: &H,
    family: &str,
) -> Result<String, OrbitError> {
    if let Some(model) = host.duel_orchestrator_model(family) {
        return Ok(model);
    }
    host.resolved_agent_model_pair(family)
        .map(|pair| pair.orchestrator)
        .ok_or_else(|| {
            OrbitError::Execution(format!(
                "no registered model pair for agent family '{family}'"
            ))
        })
}

fn build_role_assignment<H: RuntimeHost + ?Sized>(
    host: &H,
    family: &str,
) -> Result<PlanningRoleAssignment, OrbitError> {
    let _ = orchestrator_model_for(host, family)?;
    Ok(PlanningRoleAssignment {
        family: AgentFamily::parse(family)?,
    })
}

// pub(crate) widened for tests/ layout under ORB-00225; test reaches via exposed surface.
pub(crate) fn build_roles_output<H: RuntimeHost + ?Sized>(
    host: &H,
    perm: [usize; 3],
) -> Result<Value, OrbitError> {
    let families = host.duel_candidate_families();
    let perm = validate_role_permutation(perm, families.len(), "select_planning_duel_roles")?;
    let planner_a = families[perm[0]].as_str();
    let planner_b = families[perm[1]].as_str();
    let arbiter = families[perm[2]].as_str();

    let started_at = Utc::now().to_rfc3339();

    Ok(json!({
        "planner_a_agent_cli": planner_a,
        "planner_a_model": orchestrator_model_for(host, planner_a)?,
        "planner_b_agent_cli": planner_b,
        "planner_b_model": orchestrator_model_for(host, planner_b)?,
        "arbiter_agent_cli": arbiter,
        "arbiter_model": orchestrator_model_for(host, arbiter)?,
        "planning_duel_started_at": started_at,
        "planning_duel_roles": {
            "planner_a": build_role_assignment(host, planner_a)?,
            "planner_b": build_role_assignment(host, planner_b)?,
            "arbiter": build_role_assignment(host, arbiter)?,
        }
    }))
}

pub(super) fn planner_input_for_slot(task_id: &str, slot: RoleSlot) -> Value {
    json!({ "task_id": task_id, "planning_duel_slot": slot.as_str() })
}

pub(super) fn arbiter_input(task_id: &str) -> Value {
    json!({ "task_id": task_id, "planning_duel_slot": RoleSlot::Arbiter.as_str() })
}

pub(super) fn parse_planning_duel_roles(input: &Value) -> Result<PlanningRoles, OrbitError> {
    serde_json::from_value(input.get("planning_duel_roles").cloned().ok_or_else(|| {
        OrbitError::InvalidInput("missing required input.planning_duel_roles".to_string())
    })?)
    .map_err(|err| OrbitError::InvalidInput(format!("invalid planning_duel_roles payload: {err}")))
}

pub(super) fn select_planning_duel_roles<H: RuntimeHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;

    let pa = input_string_field(input, "planner_a_family");
    let pb = input_string_field(input, "planner_b_family");
    let ar = input_string_field(input, "arbiter_family");

    let roles_output = if let (Some(a), Some(b), Some(c)) =
        (pa.as_deref(), pb.as_deref(), ar.as_deref())
    {
        // explicit assignment path (CLI or direct workflow); all-or-nothing already enforced by caller,
        // but defend here for partial YAML / direct activity calls
        if a == b || a == c || b == c {
            let dup = if a == b || a == c { a } else { b };
            return Err(OrbitError::InvalidInput(format!(
                "select_planning_duel_roles explicit roles must use distinct families; '{dup}' appears more than once"
            )));
        }

        let families = host.duel_candidate_families();
        let ia = families.iter().position(|f| f == a).ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "planner_a_family value '{a}' is not in [duel] candidates {families:?}"
            ))
        })?;
        let ib = families.iter().position(|f| f == b).ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "planner_b_family value '{b}' is not in [duel] candidates {families:?}"
            ))
        })?;
        let ic = families.iter().position(|f| f == c).ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "arbiter_family value '{c}' is not in [duel] candidates {families:?}"
            ))
        })?;

        let perm = [ia, ib, ic];
        validate_role_permutation(perm, families.len(), "select_planning_duel_roles")?;
        build_roles_output(host, perm)?
    } else if pa.is_some() || pb.is_some() || ar.is_some() {
        let mut missing = vec![];
        if pa.is_none() {
            missing.push("planner_a_family");
        }
        if pb.is_none() {
            missing.push("planner_b_family");
        }
        if ar.is_none() {
            missing.push("arbiter_family");
        }
        return Err(OrbitError::InvalidInput(format!(
            "select_planning_duel_roles explicit roles require all three of planner_a_family, planner_b_family, arbiter_family; missing {}",
            missing.join(", ")
        )));
    } else {
        let perm = next_permutation(host)?;
        build_roles_output(host, perm)?
    };

    Ok(json!({
        "task_id": task_id,
        "planning_duel_started_at": roles_output["planning_duel_started_at"].clone(),
        "planner_a_agent_cli": roles_output["planner_a_agent_cli"].clone(),
        "planner_a_model": roles_output["planner_a_model"].clone(),
        "planner_b_agent_cli": roles_output["planner_b_agent_cli"].clone(),
        "planner_b_model": roles_output["planner_b_model"].clone(),
        "arbiter_agent_cli": roles_output["arbiter_agent_cli"].clone(),
        "arbiter_model": roles_output["arbiter_model"].clone(),
        "planning_duel_roles": roles_output["planning_duel_roles"].clone(),
    }))
}
