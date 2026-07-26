use std::collections::BTreeMap;
use std::sync::Arc;

use crate::activity_job::{
    V2AgentDispatchOverride, V2AuditWriter, V2DispatchInput, dispatch_v2_activity,
};
use orbit_common::types::{
    ActivityV2, InvocationTrace, OrbitError, PlanningRoleAssignment, RoleSlot,
};
use serde_json::{Value, json};

use super::types::PlanningDuelPlanArtifact;
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

fn dispatch_duel_agent_activity<H: RuntimeHost + ?Sized>(
    host: &H,
    activity_name: &str,
    activity: &ActivityV2,
    provider: &str,
    model: &str,
    input: Value,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
) -> Result<ActivityInvocationResult, OrbitError> {
    let v2_host = host.v2_runtime_host()?;
    let outcome = dispatch_v2_activity(V2DispatchInput {
        activity_name,
        spec: &activity.spec,
        fs_profile: activity.fs_profile.as_deref(),
        input: input.clone(),
        audit: audit.clone(),
        run_id,
        agent_override: Some(V2AgentDispatchOverride {
            provider,
            model: Some(model),
        }),
        host: Some(v2_host),
    })
    .map_err(|error| {
        OrbitError::Execution(format!(
            "planning duel v2 activity '{}' dispatch failed: {error}",
            activity_name
        ))
    })?;

    if !outcome.success {
        return Err(OrbitError::Execution(format!(
            "planning duel v2 activity '{}' failed: {}",
            activity_name,
            outcome
                .message
                .unwrap_or_else(|| "agent invocation did not succeed".to_string())
        )));
    }

    let (trace, duration_ms) = match outcome.invocation.as_ref() {
        Some(invocation) => {
            if let Err(error) = v2_host.persist_invocation_trace(
                run_id,
                activity_name,
                &invocation.provider,
                invocation.model.as_deref(),
                &input,
                &invocation.trace,
            ) {
                audit.note_telemetry_failure("invocation_trace", Some(activity_name), &error);
            }
            (invocation.trace.clone(), invocation.trace.duration_ms)
        }
        None => (InvocationTrace::default(), 0),
    };
    let exit_code = outcome
        .output
        .get("exit_code")
        .and_then(Value::as_i64)
        .and_then(|code| i32::try_from(code).ok());
    let duration_ms = outcome
        .output
        .get("duration_ms")
        .and_then(Value::as_u64)
        .unwrap_or(duration_ms);

    Ok(ActivityInvocationResult {
        response_json: Some(outcome.output),
        invocation_trace: trace,
        exit_code,
        duration_ms,
    })
}

fn require_plan_artifact_for_assignment<'a>(
    plan_artifacts: &'a [PlanningDuelPlanArtifact],
    assignment: &PlanningRoleAssignment,
    slot: RoleSlot,
    invocation: &ActivityInvocationResult,
) -> Result<&'a PlanningDuelPlanArtifact, OrbitError> {
    artifacts::plan_artifact_for_assignment(plan_artifacts, assignment, slot).map_err(|error| {
        OrbitError::Execution(format!(
            "{error}; {}",
            planner_invocation_diagnostics(invocation)
        ))
    })
}

fn planner_invocation_diagnostics(invocation: &ActivityInvocationResult) -> String {
    let mut parts = vec![
        format!("exit_code={:?}", invocation.exit_code),
        format!("duration_ms={}", invocation.duration_ms),
    ];

    if let Some(response) = invocation.response_json.as_ref().and_then(Value::as_object) {
        for key in [
            "provider",
            "exit_code",
            "timed_out",
            "stdout_blob_ref",
            "stderr_blob_ref",
            "error",
            "error_message",
        ] {
            if let Some(value) = response.get(key) {
                parts.push(format!("{key}={}", diagnostic_value(value)));
            }
        }
        if let Some(stdout_text) = response.get("stdout_text").and_then(Value::as_str) {
            parts.push(format!(
                "stdout_text={}",
                compact_diagnostic_text(stdout_text)
            ));
        }
    }

    let tool_calls = invocation
        .invocation_trace
        .tool_calls
        .iter()
        .map(|call| call.tool_name.trim())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    if !tool_calls.is_empty() {
        parts.push(format!("tool_calls={}", tool_calls.join(",")));
    }

    format!("child invocation diagnostics: {}", parts.join(", "))
}

fn diagnostic_value(value: &Value) -> String {
    value
        .as_str()
        .map(compact_diagnostic_text)
        .unwrap_or_else(|| value.to_string())
}

fn compact_diagnostic_text(value: &str) -> String {
    const LIMIT: usize = 240;
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= LIMIT {
        compact
    } else {
        format!("{}...", compact.chars().take(LIMIT).collect::<String>())
    }
}

pub(crate) fn run_planning_duel<H: RuntimeHost + TaskHost + Sync + ?Sized>(
    host: &H,
    input: &Value,
    _debug: bool,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let job_run_id = input_string_field(input, "job_run_id")
        .or_else(|| input_string_field(input, "run_id"))
        .ok_or_else(|| OrbitError::InvalidInput("missing required input.run_id".to_string()))?;
    let run_id = job_run_id.as_str();

    let _ = host.get_task(task_id)?;

    artifacts::cleanup_stale_planning_duel_artifacts(host, task_id)?;

    let roles_input = {
        let mut m = serde_json::Map::new();
        m.insert("task_id".to_string(), json!(task_id));
        if let Some(v) = input_string_field(input, "planner_a_family") {
            m.insert("planner_a_family".to_string(), json!(v));
        }
        if let Some(v) = input_string_field(input, "planner_b_family") {
            m.insert("planner_b_family".to_string(), json!(v));
        }
        if let Some(v) = input_string_field(input, "arbiter_family") {
            m.insert("arbiter_family".to_string(), json!(v));
        }
        Value::Object(m)
    };
    let roles_output = roles::select_planning_duel_roles(host, &roles_input)?;
    let planning_roles = roles::parse_planning_duel_roles(&roles_output)?;

    let planner_activity = host.v2_activity(roles::PLANNER_ACTIVITY_ID)?;
    let audit = host.v2_audit_writer(run_id)?;
    let planner_a_input = roles::planner_input_for_slot(task_id, RoleSlot::PlannerA);
    let planner_b_input = roles::planner_input_for_slot(task_id, RoleSlot::PlannerB);
    let planner_a_model = roles_output["planner_a_model"]
        .as_str()
        .ok_or_else(|| OrbitError::Execution("missing planner_a_model".to_string()))?
        .to_string();
    let planner_b_model = roles_output["planner_b_model"]
        .as_str()
        .ok_or_else(|| OrbitError::Execution("missing planner_b_model".to_string()))?
        .to_string();
    let (planner_a_result, planner_b_result) = std::thread::scope(|scope| {
        let planner_a = planning_roles.planner_a.clone();
        let planner_b = planning_roles.planner_b.clone();
        let planner_activity_a = planner_activity.clone();
        let planner_activity_b = planner_activity.clone();
        let audit_a = audit.clone();
        let audit_b = audit.clone();
        let handle_a = scope.spawn(move || {
            dispatch_duel_agent_activity(
                host,
                roles::PLANNER_ACTIVITY_ID,
                &planner_activity_a,
                planner_a.family.as_str(),
                planner_a_model.as_str(),
                planner_a_input,
                run_id,
                audit_a,
            )
        });
        let handle_b = scope.spawn(move || {
            dispatch_duel_agent_activity(
                host,
                roles::PLANNER_ACTIVITY_ID,
                &planner_activity_b,
                planner_b.family.as_str(),
                planner_b_model.as_str(),
                planner_b_input,
                run_id,
                audit_b,
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
    let _ = require_plan_artifact_for_assignment(
        &plan_artifacts,
        &planning_roles.planner_a,
        RoleSlot::PlannerA,
        &planner_a_result,
    )?;
    let _ = require_plan_artifact_for_assignment(
        &plan_artifacts,
        &planning_roles.planner_b,
        RoleSlot::PlannerB,
        &planner_b_result,
    )?;
    let arbiter_model = roles_output["arbiter_model"]
        .as_str()
        .ok_or_else(|| OrbitError::Execution("missing arbiter_model".to_string()))?
        .to_string();

    let arbiter_activity = host.v2_activity(roles::ARBITER_ACTIVITY_ID)?;
    let arbiter_result = dispatch_duel_agent_activity(
        host,
        roles::ARBITER_ACTIVITY_ID,
        &arbiter_activity,
        planning_roles.arbiter.family.as_str(),
        arbiter_model.as_str(),
        roles::arbiter_input(task_id),
        run_id,
        audit,
    )?;

    let artifacts_after_arbiter = host.get_task_artifacts(task_id)?;
    let winner =
        artifacts::winner_artifact_from_artifacts(&artifacts_after_arbiter, Some(&planning_roles))?;

    let role_metrics = BTreeMap::from([
        (
            "planner_a".to_string(),
            metrics::role_metrics_from_invocation(
                &planning_roles.planner_a,
                RoleSlot::PlannerA,
                roles::PLANNER_ACTIVITY_ID,
                &planner_a_result,
            ),
        ),
        (
            "planner_b".to_string(),
            metrics::role_metrics_from_invocation(
                &planning_roles.planner_b,
                RoleSlot::PlannerB,
                roles::PLANNER_ACTIVITY_ID,
                &planner_b_result,
            ),
        ),
        (
            "arbiter".to_string(),
            metrics::role_metrics_from_invocation(
                &planning_roles.arbiter,
                RoleSlot::Arbiter,
                roles::ARBITER_ACTIVITY_ID,
                &arbiter_result,
            ),
        ),
    ]);

    let writeback = artifacts::writeback_planning_duel_task(
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
        "task_status": writeback["task_status"].clone(),
        "winner_family": winner.winner_family,
        "winner_slot": winner.winner_slot.map(|slot| slot.as_str().to_string()),
        "recorded": true,
    }))
}
