use orbit_types::{Activity, Job, JobStep, OrbitError};
use serde_json::{Value, json};
use tracing::{debug, info, trace};

use crate::context::{
    ACTIVITY_EXECUTION_FAILED, AttemptOutcome, DirectActivityRunOutcome, EngineHost,
    ExecutionContext, ExecutorHost, ExecutorLookupHost, RuntimeHost, input_workspace_path,
    is_transient_error, redact_attempt_outcome,
};
use crate::template::TemplateContext;
use orbit_store::validate_instance_against_schema;

pub fn run_activity_direct<H: EngineHost + ExecutorLookupHost>(
    host: &H,
    activity: &Activity,
    agent_cli: &str,
    timeout_seconds: u64,
    debug: bool,
) -> Result<DirectActivityRunOutcome, OrbitError> {
    info!(
        activity_id = %activity.id,
        spec_type = %activity.spec_type,
        timeout_seconds,
        "direct activity invoked"
    );
    let execution = ExecutionContext {
        activity: activity.clone(),
        job: None,
        agent_cli: agent_cli.to_string(),
        model: None,
        model_tier: None,
        timeout_seconds,
        env_extra: vec![],
        env_set: std::collections::HashMap::new(),
        input: json!({}),
        debug,
        steps_outputs: std::collections::HashMap::new(),
        run_id: None,
        step_index: None,
        state_dir: None,
    };
    let outcome = execute_single_attempt(host, &execution);
    Ok(DirectActivityRunOutcome {
        state: outcome.state,
        duration_ms: outcome.duration_ms,
        error_code: outcome.error_code,
        error_message: outcome.error_message,
        protocol_violation: outcome.protocol_violation,
    })
}

pub fn build_execution_context_for_step<H: RuntimeHost>(
    host: &H,
    job: &Job,
    step: &JobStep,
    input: Value,
    debug: bool,
    steps_outputs: std::collections::HashMap<String, Value>,
    run_id: Option<&str>,
    step_index: Option<u32>,
) -> Result<ExecutionContext, OrbitError> {
    let effective_target_id = resolve_activity_variant(&step.target_id, host.graph_editing());
    let activity = host.validate_activity_target_exists(step.target_type, &effective_target_id)?;
    validate_activity_input_schema(&activity, &input)?;
    let state_dir = match run_id {
        Some(run_id) => {
            orbit_store::state_io::resolve_active_run_state_dir(host.data_root(), run_id)?
                .ok_or_else(|| OrbitError::JobRunNotFound(run_id.to_string()))
                .map(Some)?
        }
        None => None,
    };
    Ok(ExecutionContext {
        activity,
        job: Some(job.clone()),
        agent_cli: step
            .agent_cli
            .clone()
            .trim()
            .is_empty()
            .then(|| step.executor.clone())
            .flatten()
            .unwrap_or_else(|| step.agent_cli.clone()),
        model: step.model.clone(),
        model_tier: step.model_tier.clone(),
        timeout_seconds: step.timeout_seconds,
        env_extra: step.env_extra.clone(),
        env_set: step.env_set.clone(),
        input,
        debug,
        steps_outputs,
        run_id: run_id.map(ToOwned::to_owned),
        step_index,
        state_dir,
    })
}

/// Execute a step with automatic retry on transient failures.
///
/// `max_attempts` is the total number of attempts (including the first). Zero or one means
/// no retry — the step runs exactly once. Backoff doubles after each failed attempt starting
/// at `backoff_seconds`.
pub fn execute_with_retry<H: EngineHost + ExecutorLookupHost>(
    host: &H,
    execution: &ExecutionContext,
    max_attempts: u32,
    backoff_seconds: u64,
) -> AttemptOutcome {
    execute_with_retry_inner(
        || execute_single_attempt(host, execution),
        &execution.activity.id,
        max_attempts,
        backoff_seconds,
    )
}

pub(crate) fn execute_with_retry_inner<F>(
    mut attempt_fn: F,
    step_id: &str,
    max_attempts: u32,
    backoff_seconds: u64,
) -> AttemptOutcome
where
    F: FnMut() -> AttemptOutcome,
{
    let effective_max = max_attempts.max(1);
    let mut attempt = 0_u32;
    let mut accumulated_duration_ms: u64 = 0;
    loop {
        attempt += 1;
        let outcome = attempt_fn();
        if let Some(d) = outcome.duration_ms {
            accumulated_duration_ms += d;
        }
        let is_retryable = outcome
            .error_code
            .as_deref()
            .is_some_and(is_transient_error);
        if attempt >= effective_max || !is_retryable {
            let mut outcome = outcome;
            outcome.retry_count = attempt - 1;
            if attempt > 1 {
                outcome.duration_ms = Some(accumulated_duration_ms);
            }
            return outcome;
        }
        // Exponential backoff: delay = backoff_seconds * 2^(attempt-1).
        // The bit-shift `.min(30)` caps the exponent so the shift never
        // overflows a u64 (2^31 would exceed u64 range when multiplied).
        let delay_seconds = backoff_seconds.saturating_mul(1_u64 << (attempt - 1).min(30));
        debug!(
            step_id,
            attempt,
            effective_max,
            delay_seconds,
            error_code = outcome.error_code.as_deref().unwrap_or("unknown"),
            "retrying transient activity failure"
        );
        std::thread::sleep(std::time::Duration::from_secs(delay_seconds));
    }
}

pub fn execute_single_attempt<H: EngineHost + ExecutorLookupHost>(
    host: &H,
    execution: &ExecutionContext,
) -> AttemptOutcome {
    let registry = host.activity_executor_registry();
    let spec_type = execution.activity.spec_type.as_str();
    let supported_spec_types = registry.supported_spec_types().join(", ");
    let dispatch_key = execution
        .activity
        .executor
        .as_deref()
        .filter(|name| registry.get(name).is_some())
        .unwrap_or(spec_type);
    debug!(
        activity_id = %execution.activity.id,
        spec_type,
        executor_key = dispatch_key,
        "activity attempt started"
    );
    if spec_type == "agent_invoke" {
        debug!(
            activity_id = %execution.activity.id,
            spec_type,
            agent_cli = %execution.agent_cli,
            model = ?execution.model,
            timeout_seconds = execution.timeout_seconds,
            request = ?execution.input,
            "agent request prepared"
        );
    } else {
        trace!(
            activity_id = %execution.activity.id,
            spec_type,
            request = ?execution.input,
            "activity request prepared"
        );
    }
    let outcome = registry
        .get(dispatch_key)
        .map(|executor| executor.execute(ExecutorHost::new(host), execution))
        .unwrap_or_else(|| unsupported_spec_type_outcome(dispatch_key, &supported_spec_types));
    let outcome = redact_attempt_outcome(outcome);
    debug!(
        activity_id = %execution.activity.id,
        spec_type,
        state = %outcome.state,
        duration_ms = ?outcome.duration_ms,
        error_code = ?outcome.error_code,
        "activity attempt completed"
    );
    if spec_type == "agent_invoke" {
        debug!(
            activity_id = %execution.activity.id,
            spec_type,
            state = %outcome.state,
            response = ?outcome.response_json,
            "agent response received"
        );
    } else {
        trace!(
            activity_id = %execution.activity.id,
            spec_type,
            state = %outcome.state,
            response = ?outcome.response_json,
            "activity response received"
        );
    }
    outcome
}

fn unsupported_spec_type_outcome(spec_type: &str, supported_spec_types: &str) -> AttemptOutcome {
    AttemptOutcome::failed(
        ACTIVITY_EXECUTION_FAILED,
        format!("unsupported activity spec_type '{spec_type}' (supported: {supported_spec_types})"),
    )
}

pub(crate) fn execution_template_context_with_env(
    execution: &ExecutionContext,
    env_pairs: Vec<(String, String)>,
) -> TemplateContext {
    let mut env = env_pairs
        .into_iter()
        .collect::<std::collections::HashMap<_, _>>();
    env.insert("ORBIT_TASK_ACTOR_KIND".to_string(), "agent".to_string());
    if let Some(actor_label) = execution.activity.created_by.as_ref() {
        env.insert("ORBIT_TASK_ACTOR_LABEL".to_string(), actor_label.clone());
    }

    TemplateContext {
        input: execution.input.clone(),
        env,
        workspace_path: execution
            .activity
            .workspace_path
            .clone()
            .or_else(|| input_workspace_path(&execution.input)),
        item: None,
        iteration: None,
        steps: execution.steps_outputs.clone(),
    }
}

/// Resolve the effective activity ID based on feature flags.
///
/// When `graph_editing` is disabled, `implement_change` is swapped to
/// `implement_change_classic` which omits graph tools from the agent's
/// tool list and instruction. All other activity IDs pass through unchanged.
fn resolve_activity_variant(target_id: &str, graph_editing: bool) -> String {
    if !graph_editing && target_id == "implement_change" {
        "implement_change_classic".to_string()
    } else {
        target_id.to_string()
    }
}

pub fn validate_activity_input_schema(
    activity: &Activity,
    input: &Value,
) -> Result<(), OrbitError> {
    let context = format!(
        "job run input does not match activity '{}' input schema",
        activity.id
    );
    match validate_instance_against_schema(&activity.input_schema_json, input, &context) {
        Ok(()) => Ok(()),
        Err(OrbitError::AgentProtocolViolation(message)) => Err(OrbitError::InvalidInput(message)),
        Err(other) => Err(other),
    }
}

pub fn validate_activity_output_schema(
    activity: &Activity,
    output: &Value,
) -> Result<(), OrbitError> {
    let context = format!(
        "activity '{}' output does not match output schema",
        activity.id
    );
    validate_instance_against_schema(&activity.output_schema_json, output, &context)
}

pub fn activity_skill_refs_from_spec_config(
    spec_config: &Value,
) -> Result<Vec<String>, OrbitError> {
    ensure_spec_config_object(spec_config)?;
    let Some(raw_refs) = spec_config.get("skill_refs") else {
        return Ok(Vec::new());
    };
    serde_json::from_value(raw_refs.clone()).map_err(|error| {
        OrbitError::InvalidInput(format!(
            "activity spec_config.skill_refs must be an array of strings: {error}"
        ))
    })
}

fn ensure_spec_config_object(spec_config: &Value) -> Result<(), OrbitError> {
    if spec_config.is_object() {
        Ok(())
    } else {
        Err(OrbitError::InvalidInput(
            "activity spec_config must be a JSON object".to_string(),
        ))
    }
}
