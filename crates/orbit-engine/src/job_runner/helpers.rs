use std::fs;
use std::path::{Path, PathBuf};

use orbit_agent::Agent;
use orbit_types::{
    InvocationTrace, JobRunState, JobStep, KnowledgeRunMetrics, OrbitError, StepCondition,
    infer_agent_family_from_model,
};
use serde_json::Value;
use tiktoken_rs::cl100k_base;
use tracing::info;

use crate::context::{
    EnvironmentHost, ExecutionContext, ExecutorLookupHost, JobRunHost, RuntimeHost,
    TaskAutomationUpdate, TaskReadHost, TaskWriteHost, execution_working_directory_with_task,
};

pub(super) fn extract_task_id(input: &Value) -> Option<&str> {
    input
        .as_object()
        .and_then(|map| map.get("task_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn extract_batch_id(input: &Value) -> Option<&str> {
    input
        .as_object()
        .and_then(|map| map.get("batch_id").or_else(|| map.get("run_id")))
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

pub(crate) fn build_step_input(step: &JobStep, current_input: &Value) -> Result<Value, OrbitError> {
    merge_job_input(step.default_input.as_ref(), current_input.clone())
}

pub(crate) fn should_run_step(
    condition: &StepCondition,
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
        StepCondition::Expr(_) => {
            // Expression conditions are evaluated by the condition module with
            // full template context; this helper only handles keyword variants.
            // Callers should route Expr through condition::evaluate_condition().
            true
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
pub(crate) fn check_loop_exit<H: TaskReadHost + ?Sized>(host: &H, input: &Value) -> bool {
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

/// Resolve a step's effective `agent_cli` / `model` at execution time.
///
/// Precedence (highest first):
///
/// 1. An explicit, non-empty `step.agent_cli` — always wins.
/// 2. `agent_cli_from_input` in `step.default_input`: the named key is looked
///    up in the current job input and used as the agent CLI. `model_from_input`
///    is consulted the same way to fill a missing `step.model`. This is the
///    general-purpose hook that lets workflows like `duel` randomize agent
///    assignment per run without patching the step at load time.
/// 3. The task's actor identity (original implementer). Used by review-loop
///    steps that should route fixes back to the agent that wrote the code.
///
/// Returns `None` if no override is needed (the caller should use `step` as-is).
pub(super) fn resolve_step_agent<H: TaskReadHost + ?Sized>(
    host: &H,
    step: &JobStep,
    input: &Value,
) -> Option<JobStep> {
    if !step.agent_cli.trim().is_empty() || step.executor.is_some() {
        return None;
    }
    if let Some(resolved) = resolve_step_agent_from_input(step, input) {
        return Some(resolved);
    }
    resolve_step_agent_from_task(host, step, input)
}

/// Populate `step.agent_cli` / `step.model` from named keys in the current
/// job input, when the step's `default_input` contains `agent_cli_from_input`
/// / `model_from_input`.
///
/// Returns `None` when the step has no `agent_cli_from_input` hook, when the
/// declared key is missing or not a string, or when the value is an empty
/// string (so the next precedence layer can try).
pub(super) fn resolve_step_agent_from_input(step: &JobStep, input: &Value) -> Option<JobStep> {
    if !step.agent_cli.trim().is_empty() || step.executor.is_some() {
        return None;
    }
    let defaults = step.default_input.as_ref()?.as_object()?;
    let key = defaults
        .get("agent_cli_from_input")
        .and_then(Value::as_str)?;
    let map = input.as_object()?;
    let agent_cli = map
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    let mut resolved = step.clone();
    resolved.agent_cli = agent_cli.to_string();

    if resolved.model.is_none()
        && let Some(model_key) = defaults.get("model_from_input").and_then(Value::as_str)
        && let Some(model) = map
            .get(model_key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    {
        resolved.model = Some(model.to_string());
    }

    Some(resolved)
}

/// When a step's `agent_cli` is empty, try to resolve it from the task's
/// `agent` and `model` fields so the original implementer handles the step
/// (e.g. in a review-loop where the fix should go back to the same agent).
pub(super) fn resolve_step_agent_from_task<H: TaskReadHost + ?Sized>(
    host: &H,
    step: &JobStep,
    input: &Value,
) -> Option<JobStep> {
    if !step.agent_cli.trim().is_empty() || step.executor.is_some() {
        return None;
    }
    let task_id = extract_task_id(input)?;
    let task = host.get_task(task_id).ok()?;
    let agent = task
        .agent
        .clone()
        .or_else(|| {
            task.model
                .as_deref()
                .and_then(infer_agent_family_from_model)
        })
        .filter(|a| !a.trim().is_empty())?;
    let mut resolved = step.clone();
    resolved.agent_cli = agent;
    if resolved.model.is_none() {
        resolved.model = task.model.clone();
    }
    Some(resolved)
}

pub(super) fn record_task_agent_context<
    H: TaskWriteHost + RuntimeHost + EnvironmentHost + ExecutorLookupHost,
>(
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

pub(super) fn resolved_model_name<H: RuntimeHost + EnvironmentHost + ExecutorLookupHost>(
    host: &H,
    execution: &ExecutionContext,
) -> Option<String> {
    let requested = execution
        .model
        .clone()
        .or_else(|| resolved_model_from_executor_tier(host, execution));
    let config = host
        .agent_config_for(&execution.agent_cli, requested.as_deref())
        .ok()?;
    let model_from_config = config.model.clone().or(requested);
    let agent = Agent::new(&config).ok();
    let resolved = agent
        .and_then(|agent| agent.model_name().map(ToOwned::to_owned))
        .or(model_from_config);
    host.canonical_model_name(&execution.agent_cli, resolved.as_deref())
}

fn resolved_model_from_executor_tier<H: RuntimeHost + ExecutorLookupHost>(
    host: &H,
    execution: &ExecutionContext,
) -> Option<String> {
    let tier = execution
        .model_tier
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let def = host.get_executor_def(&execution.agent_cli).ok().flatten();
    if let Some(model) = def.as_ref().and_then(|def| def.model_for_tier(tier)) {
        return Some(model.to_string());
    }
    match tier {
        "strong" => host
            .resolved_agent_model_pair(&execution.agent_cli)
            .map(|pair| pair.orchestrator),
        "weak" => host
            .resolved_agent_model_pair(&execution.agent_cli)
            .map(|pair| pair.helper),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PreparedImplementChangeMetrics {
    raw_read_token_baseline: u64,
}

pub(super) fn prepare_implement_change_metrics<H: TaskReadHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
) -> Result<Option<PreparedImplementChangeMetrics>, OrbitError> {
    if execution.activity.id != "implement_change" {
        return Ok(None);
    }

    let Some(task_id) = extract_task_id(&execution.input) else {
        return Ok(None);
    };
    let task = host.get_task(task_id)?;
    let workspace_root = execution_working_directory_with_task(host, execution)
        .map(PathBuf::from)
        .ok_or_else(|| {
            OrbitError::Execution(
                "implement_change metrics require an effective workspace_path".to_string(),
            )
        })?;

    let mut raw_read_token_baseline = 0_u64;
    for context_file in task.context_files {
        let path = resolve_context_path(&workspace_root, &context_file);
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        raw_read_token_baseline = raw_read_token_baseline.saturating_add(tokenize_text(&content)?);
    }

    Ok(Some(PreparedImplementChangeMetrics {
        raw_read_token_baseline,
    }))
}

pub(super) fn build_knowledge_run_metrics(
    prepared: &PreparedImplementChangeMetrics,
    trace: &InvocationTrace,
) -> Result<KnowledgeRunMetrics, OrbitError> {
    let actual_fs_read_tokens_during_run = trace
        .tool_calls
        .iter()
        .filter(|call| call.tool_name == "fs.read")
        .try_fold(0_u64, |acc, call| {
            let payload_tokens = call
                .result_payload
                .as_ref()
                .map(tokenize_json_value)
                .transpose()?
                .unwrap_or(0);
            Ok::<u64, OrbitError>(acc.saturating_add(payload_tokens))
        })?;

    let pack_payload = trace
        .tool_calls
        .iter()
        .find(|call| call.tool_name == "orbit.graph.pack")
        .and_then(|call| call.result_payload.as_ref());

    let knowledge_unavailable = pack_payload
        .and_then(|payload| payload.get("kind"))
        .and_then(Value::as_str)
        == Some("knowledge_unavailable");

    let knowledge_pack_used = pack_payload.is_some() && !knowledge_unavailable;
    let knowledge_pack_tokens = if knowledge_pack_used {
        pack_payload.map(tokenize_json_value).transpose()?
    } else {
        None
    };
    let knowledge_pack_unresolved_count = if knowledge_pack_used {
        pack_payload
            .and_then(|payload| payload.get("unresolved_selectors"))
            .and_then(Value::as_array)
            .map(|items| items.len() as u32)
            .unwrap_or(0)
    } else {
        0
    };

    Ok(KnowledgeRunMetrics {
        raw_read_token_baseline: prepared.raw_read_token_baseline,
        knowledge_pack_tokens,
        compression_ratio: knowledge_pack_tokens
            .and_then(|tokens| safe_ratio(prepared.raw_read_token_baseline, tokens)),
        actual_fs_read_tokens_during_run,
        double_read_rate: safe_ratio(
            actual_fs_read_tokens_during_run,
            prepared.raw_read_token_baseline,
        ),
        knowledge_pack_used,
        knowledge_pack_unresolved_count,
        total_llm_input_tokens: trace
            .usage
            .input
            .saturating_add(trace.usage.cache_read)
            .saturating_add(trace.usage.cache_create),
    })
}

pub(super) fn tokenize_text(text: &str) -> Result<u64, OrbitError> {
    let encoder = cl100k_base()
        .map_err(|error| OrbitError::Execution(format!("load cl100k_base: {error}")))?;
    Ok(encoder.encode_with_special_tokens(text).len() as u64)
}

fn tokenize_json_value(value: &Value) -> Result<u64, OrbitError> {
    let serialized = serde_json::to_string(value)
        .map_err(|error| OrbitError::Execution(format!("serialize tool result: {error}")))?;
    tokenize_text(&serialized)
}

fn safe_ratio(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator != 0).then(|| numerator as f64 / denominator as f64)
}

fn resolve_context_path(workspace_root: &Path, context_file: &str) -> PathBuf {
    let path = Path::new(context_file);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}
