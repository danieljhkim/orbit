use orbit_agent::Agent;
use orbit_types::{JobRunState, JobStep, OrbitError, StepCondition};
use serde_json::Value;
use tracing::info;

use crate::context::{EngineHost, ExecutionContext, JobRunHost, RuntimeHost, TaskAutomationUpdate};

pub(super) fn extract_task_id(input: &Value) -> Option<&str> {
    input
        .as_object()
        .and_then(|map| map.get("task_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn normalize_agent_label(agent_cli: &str) -> String {
    std::path::Path::new(agent_cli)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(agent_cli)
        .to_ascii_lowercase()
}

pub(super) fn release_task_locks_for_job_input<H: RuntimeHost>(
    host: &H,
    input: &Value,
) -> Result<(), OrbitError> {
    if let Some(task_id) = extract_task_id(input) {
        let _ = host.release_file_locks(task_id)?;
    }
    Ok(())
}

pub(super) fn json_value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub(super) fn merge_job_input(
    default_input: Option<&Value>,
    input: Value,
) -> Result<Value, OrbitError> {
    let mut merged = match default_input {
        None => serde_json::Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(other) => {
            return Err(OrbitError::InvalidInput(format!(
                "job default_input must be an object, got {}",
                json_value_type_name(other)
            )));
        }
    };

    let input_map = match input {
        Value::Object(map) => map,
        other => {
            return Err(OrbitError::InvalidInput(format!(
                "job run input must be an object, got {}",
                json_value_type_name(&other)
            )));
        }
    };

    for (key, value) in input_map {
        merged.insert(key, value);
    }

    Ok(Value::Object(merged))
}

pub(super) fn should_run_step(
    condition: StepCondition,
    previous_step_state: Option<JobRunState>,
) -> bool {
    match condition {
        StepCondition::Always => true,
        StepCondition::OnSuccess => {
            previous_step_state.is_none_or(|state| matches!(state, JobRunState::Success))
        }
        StepCondition::OnFailure => previous_step_state.is_some_and(step_state_records_failure),
        StepCondition::OnTimeout => {
            previous_step_state.is_some_and(|state| matches!(state, JobRunState::Timeout))
        }
    }
}

pub(super) fn step_state_records_failure(state: JobRunState) -> bool {
    matches!(
        state,
        JobRunState::Failed | JobRunState::Timeout | JobRunState::Cancelled
    )
}

pub(super) fn step_state_records_incident(state: JobRunState) -> bool {
    matches!(state, JobRunState::Failed | JobRunState::Timeout)
}

pub(super) fn run_was_cancelled<H: JobRunHost>(host: &H, run_id: &str) -> Result<bool, OrbitError> {
    Ok(host
        .get_job_run(run_id)?
        .is_some_and(|run| run.state == JobRunState::Cancelled))
}

/// Returns `true` if the accumulated input contains `"loop_exit": true`.
pub(super) fn check_loop_exit<H: crate::context::TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> bool {
    // Primary: check for explicit loop_exit signal in piped input.
    let explicit = input
        .as_object()
        .and_then(|map| map.get("loop_exit"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if explicit {
        return true;
    }

    // Fallback: if the agent persisted pr_status to the task but crashed before
    // returning structured output (with loop_exit), check the task directly.
    if let Some(task_id) = extract_task_id(input)
        && let Ok(task) = host.get_task(task_id)
        && let Some(ref pr_status) = task.pr_status
    {
        let normalized = crate::executor::automation::review::normalize_review_decision(pr_status);
        if normalized == "APPROVED" {
            return true;
        }
    }

    false
}

pub(super) fn log_step_completion(
    step_index: usize,
    iteration: u32,
    step: &JobStep,
    state: JobRunState,
    duration_ms: Option<u64>,
    error_code: Option<&str>,
    error_message: Option<&str>,
) {
    if step_state_records_incident(state) {
        info!(
            step_index,
            iteration,
            target_id = %step.target_id,
            target_type = %step.target_type,
            state = %state,
            duration_ms = ?duration_ms,
            error_code = error_code.unwrap_or(""),
            error_message = error_message.unwrap_or(""),
            "step failed"
        );
    } else {
        info!(
            step_index,
            iteration,
            target_id = %step.target_id,
            target_type = %step.target_type,
            state = %state,
            duration_ms = ?duration_ms,
            "step completed"
        );
    }
}

/// When a step's `agent_cli` is empty, try to resolve it from the task's
/// `agent` and `model` fields so the original implementer handles the step
/// (e.g. in a review-loop where the fix should go back to the same agent).
pub(super) fn resolve_step_agent_from_task<H: EngineHost>(
    host: &H,
    step: &JobStep,
    input: &Value,
) -> Option<JobStep> {
    if !step.agent_cli.trim().is_empty() {
        return None;
    }
    let task_id = extract_task_id(input)?;
    let task = host.get_task(task_id).ok()?;
    let agent = task
        .actor_identity
        .agent_name()
        .filter(|a| !a.trim().is_empty())?;
    let mut resolved = step.clone();
    resolved.agent_cli = agent.to_string();
    if resolved.model.is_none() {
        resolved.model = task.actor_identity.agent_model().map(ToOwned::to_owned);
    }
    Some(resolved)
}

pub(super) fn record_task_agent_context<H: EngineHost>(
    host: &H,
    execution: &ExecutionContext,
) -> Result<(), OrbitError> {
    if execution.agent_cli.trim().is_empty() {
        return Ok(());
    }
    let Some(task_id) = extract_task_id(&execution.input) else {
        return Ok(());
    };

    host.apply_task_automation_update(
        task_id,
        TaskAutomationUpdate {
            agent: Some(normalize_agent_label(&execution.agent_cli)),
            model: resolved_model_name(host, execution),
            ..Default::default()
        },
    )
}

pub(super) fn resolved_model_name<H: EngineHost>(
    host: &H,
    execution: &ExecutionContext,
) -> Option<String> {
    let config = host
        .agent_config_for(&execution.agent_cli, execution.model.as_deref())
        .ok()?;
    let model_from_config = config.model.clone();
    let agent = Agent::new(&config).ok();
    agent
        .and_then(|agent| agent.model_name().map(ToOwned::to_owned))
        .or(model_from_config)
}
