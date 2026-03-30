use std::io::Write;

use orbit_agent::{Agent, AgentRequest, AgentResponseStatus, parse_and_validate_response};
use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{AgentResponseEnvelope, JobRunState, OrbitError};
use serde_json::Value;
use tempfile::NamedTempFile;

use super::ActivityExecutor;
use crate::context::{
    AGENT_COMMIT_FAILED, AGENT_INVOCATION_FAILED, AGENT_OUTPUT_MISSING, AGENT_PROTOCOL_VIOLATION,
    AGENT_PROVIDER_OVERLOAD, AGENT_RATE_LIMIT, AGENT_TIMEOUT, AGENT_TRANSPORT_FAILURE,
    AgentProtocolHost, AttemptOutcome, EngineHost, EnvironmentHost, ExecutionContext,
    apply_env_set, execution_working_directory, execution_working_directory_with_task,
};

pub struct AgentExecutor;

impl ActivityExecutor for AgentExecutor {
    fn spec_type(&self) -> &str {
        "agent_invoke"
    }

    fn execute(&self, host: &dyn EngineHost, execution: &ExecutionContext) -> AttemptOutcome {
        // Resolve working directory here where EngineHost (which includes
        // TaskHost) is available, so we can fall back to task.workspace_path.
        let working_dir = execution_working_directory_with_task(host, execution);
        let outcome = execute_with_cwd(host, execution, working_dir);

        // When the agent exited 0 but produced no JSON output, attempt to
        // recover a synthetic result from the task state. This makes the
        // pipeline resilient to agent output formatting issues for activities
        // that persist their results via orbit.task.update.
        if outcome.error_code.as_deref() == Some(AGENT_OUTPUT_MISSING) {
            if let Some(recovered) = try_recover_from_task(host, execution, &outcome) {
                return recovered;
            }
            // For activities with no required output fields (side-effect-only
            // activities that persist results via task updates), synthesize an
            // empty success result so the pipeline continues.
            if let Some(recovered) = try_recover_empty_result(host, execution, &outcome) {
                return recovered;
            }
        }

        outcome
    }
}

/// Attempt to recover a synthetic agent result from task state when the agent
/// exited successfully but produced no JSON envelope.
fn try_recover_from_task(
    host: &dyn EngineHost,
    execution: &ExecutionContext,
    original: &AttemptOutcome,
) -> Option<AttemptOutcome> {
    let task_id = execution.input.get("task_id")?.as_str()?;
    let task = host.get_task(task_id).ok()?;

    // Build candidate fields from task state.
    let mut candidates = serde_json::Map::new();

    let status_str = task.status.cli_name().to_string();
    candidates.insert("status".to_string(), Value::String(status_str));

    if !task.execution_summary.is_empty() {
        candidates.insert(
            "execution_summary".to_string(),
            Value::String(task.execution_summary.clone()),
        );
    }

    if let Some(pr_status) = task.pr_status.as_ref() {
        candidates.insert("pr_status".to_string(), Value::String(pr_status.clone()));
    }

    // Require at least execution_summary or pr_status for recovery — bare status
    // alone is not evidence the agent actually persisted meaningful output.
    if !candidates.contains_key("execution_summary") && !candidates.contains_key("pr_status") {
        return None;
    }

    // Filter the synthetic result to only include fields declared in the activity's
    // output schema. This prevents leaking undeclared fields (e.g. "status") into
    // downstream step input when the schema uses additionalProperties: false or
    // simply doesn't declare them.
    let result = filter_to_output_schema(&execution.activity, candidates);

    // After filtering, re-check that we still have meaningful content.
    if !result.contains_key("execution_summary") && !result.contains_key("pr_status") {
        return None;
    }

    // Status is always "success" because recovery only runs on the exit_code == 0
    // (AGENT_OUTPUT_MISSING) path — the agent process succeeded, it just didn't
    // produce the expected JSON envelope.
    let envelope = AgentResponseEnvelope {
        schema_version: 1,
        status: "success".to_string(),
        result: Some(Value::Object(result)),
        error: None,
        duration_ms: original.duration_ms.unwrap_or(0),
    };

    // Validate the synthetic envelope against the activity's skill output schema.
    // If validation fails, fall through to the original AGENT_OUTPUT_MISSING failure
    // rather than returning an invalid payload.
    if let Err(_) = host.validate_skill_output_schema(&execution.activity, &envelope) {
        return None;
    }

    Some(AttemptOutcome {
        state: JobRunState::Success,
        exit_code: original.exit_code,
        duration_ms: original.duration_ms,
        response_json: serde_json::to_value(&envelope).ok(),
        error_code: None,
        error_message: None,
        protocol_violation: false,
        retry_count: 0,
    })
}

/// Attempt to recover by synthesizing an empty `{}` result when the activity's
/// output schema has no required fields. This covers side-effect-only activities
/// (e.g. dispatch steps that persist results via `orbit.task.update`) where the
/// agent's JSON output is not needed by downstream steps.
fn try_recover_empty_result(
    host: &dyn EngineHost,
    execution: &ExecutionContext,
    original: &AttemptOutcome,
) -> Option<AttemptOutcome> {
    let envelope = AgentResponseEnvelope {
        schema_version: 1,
        status: "success".to_string(),
        result: Some(Value::Object(serde_json::Map::new())),
        error: None,
        duration_ms: original.duration_ms.unwrap_or(0),
    };

    // Only recover if the empty result validates against the output schema.
    // Activities with required output fields will fail validation and fall
    // through to the original AGENT_OUTPUT_MISSING error.
    if host
        .validate_skill_output_schema(&execution.activity, &envelope)
        .is_err()
    {
        return None;
    }

    Some(AttemptOutcome {
        state: JobRunState::Success,
        exit_code: original.exit_code,
        duration_ms: original.duration_ms,
        response_json: serde_json::to_value(&envelope).ok(),
        error_code: None,
        error_message: None,
        protocol_violation: false,
        retry_count: 0,
    })
}

/// Filter a candidate result map to only include keys declared in the activity's
/// `output_schema_json.properties`. If the schema has no `properties` object
/// (e.g. it is empty or uses a freeform schema), all candidates are kept.
fn filter_to_output_schema(
    activity: &orbit_types::Activity,
    candidates: serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    let Some(props) = activity
        .output_schema_json
        .get("properties")
        .and_then(|v| v.as_object())
    else {
        return candidates;
    };
    candidates
        .into_iter()
        .filter(|(key, _)| props.contains_key(key))
        .collect()
}

pub fn execute<H: EnvironmentHost + AgentProtocolHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
) -> AttemptOutcome {
    let working_dir = execution_working_directory(execution);
    execute_with_cwd(host, execution, working_dir)
}

fn execute_with_cwd<H: EnvironmentHost + AgentProtocolHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
    working_dir: Option<String>,
) -> AttemptOutcome {
    let invocation = match build_agent_invocation(host, execution) {
        Ok(invocation) => invocation,
        Err(outcome) => return outcome,
    };
    let exec_result = match execute_agent_process(host, execution, invocation, working_dir) {
        Ok(result) => result,
        Err(outcome) => return outcome,
    };

    if agent_process_was_interrupted(&exec_result) && exec_result.stdout.trim().is_empty() {
        return AttemptOutcome {
            state: JobRunState::Cancelled,
            exit_code: exec_result.exit_code,
            duration_ms: Some(exec_result.duration_ms),
            response_json: None,
            error_code: None,
            error_message: Some(exec_result.stderr.trim().to_string()),
            protocol_violation: false,
            retry_count: 0,
        };
    }

    if orbit_agent::is_timeout(&exec_result) && exec_result.stdout.trim().is_empty() {
        return AttemptOutcome {
            state: JobRunState::Timeout,
            exit_code: exec_result.exit_code,
            duration_ms: Some(exec_result.duration_ms),
            response_json: None,
            error_code: Some(AGENT_TIMEOUT.to_string()),
            error_message: Some(format_timeout_error_message(&exec_result)),
            protocol_violation: false,
            retry_count: 0,
        };
    }

    match parse_and_validate_response(&exec_result) {
        Ok((envelope, state)) => {
            // Detect synthesized success: agent exited 0 but produced no parseable JSON
            // envelope (result is None only when synthesize_response was used). This is
            // retryable — the agent may succeed on a subsequent attempt.
            if state == AgentResponseStatus::Success
                && envelope.result.is_none()
                && exec_result.exit_code == Some(0)
                && !orbit_agent::is_timeout(&exec_result)
            {
                return AttemptOutcome {
                    state: JobRunState::Failed,
                    exit_code: exec_result.exit_code,
                    duration_ms: Some(exec_result.duration_ms),
                    response_json: None,
                    error_code: Some(AGENT_OUTPUT_MISSING.to_string()),
                    error_message: Some(
                        "agent exited successfully but produced no JSON result envelope"
                            .to_string(),
                    ),
                    protocol_violation: false,
                    retry_count: 0,
                };
            }
            process_agent_response(host, execution, &exec_result, envelope, state)
        }
        Err(OrbitError::AgentProtocolViolation(message)) => AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: exec_result.exit_code,
            duration_ms: Some(exec_result.duration_ms),
            response_json: None,
            error_code: Some(AGENT_PROTOCOL_VIOLATION.to_string()),
            error_message: Some(message),
            protocol_violation: true,
            retry_count: 0,
        },
        Err(err) => AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: exec_result.exit_code,
            duration_ms: Some(exec_result.duration_ms),
            response_json: None,
            error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
            error_message: Some(err.to_string()),
            protocol_violation: false,
            retry_count: 0,
        },
    }
}

fn agent_process_was_interrupted(exec_result: &orbit_types::ExecutionResult) -> bool {
    !exec_result.success && exec_result.stderr.contains("process interrupted by signal")
}

fn build_agent_invocation<H: EnvironmentHost + AgentProtocolHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
) -> Result<orbit_agent::AgentResponse, AttemptOutcome> {
    let config = host
        .agent_config_for(&execution.agent_cli, execution.model.as_deref())
        .map_err(invocation_failed_outcome)?;
    let agent = Agent::new(&config).map_err(invocation_failed_outcome)?;
    let stdin_payload = host
        .build_agent_stdin_envelope_payload(execution)
        .map_err(invocation_failed_outcome)?;

    let output_schema = Some(execution.activity.output_schema_json.clone());

    let invocation = agent
        .invoke(
            match &execution.job {
                Some(job) => AgentRequest::job(
                    job.job_id.clone(),
                    execution.activity.id.clone(),
                    stdin_payload,
                    output_schema,
                ),
                None => AgentRequest::activity(
                    execution.activity.id.clone(),
                    stdin_payload,
                    output_schema,
                ),
            }
            .with_verbose(execution.debug),
        )
        .map_err(invocation_failed_outcome)?;

    let missing_env = host.missing_required_environment_vars(invocation.required_env_vars);
    if !missing_env.is_empty() {
        let vars = missing_env.join(", ");
        return Err(AttemptOutcome::failed(
            AGENT_INVOCATION_FAILED,
            format!(
                "missing required environment variable(s) for provider '{}': {vars}. \
configure .orbit/config.toml [execution.env].pass and set these variables in the parent shell.",
                invocation.runtime_key
            ),
        ));
    }

    Ok(invocation)
}

fn execute_agent_process<H: EnvironmentHost + AgentProtocolHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
    invocation: orbit_agent::AgentResponse,
    working_dir: Option<String>,
) -> Result<orbit_types::ExecutionResult, AttemptOutcome> {
    let (args, _stdout_schema_file) =
        prepare_exec_args(&invocation).map_err(invocation_failed_outcome)?;

    let resolved_model = resolve_model_for_env(host, execution);
    let environment_mode = apply_env_set(
        inject_proc_allowed_programs(
            inject_agent_identity(
                inject_activity_tools(
                    host.execution_environment_mode(&execution.env_extra),
                    &execution.activity.tools,
                ),
                execution,
                resolved_model.as_deref(),
            ),
            &execution.activity.proc_allowed_programs,
        ),
        &execution.env_set,
    );

    run_process(
        &ExecRequest {
            program: invocation.program,
            args,
            current_dir: working_dir,
            timeout_ms: Some(execution.timeout_seconds.saturating_mul(1000)),
            stdin_mode: StdinMode::Bytes(invocation.stdin),
            environment_mode,
            debug: execution.debug,
        },
        &NoSandbox,
    )
    .map_err(invocation_failed_outcome)
}

fn inject_activity_tools(mode: EnvironmentMode, tools: &[String]) -> EnvironmentMode {
    if tools.is_empty() {
        return mode;
    }
    let tools_str = tools.join(",");
    match mode {
        EnvironmentMode::ClearAndSet(mut pairs) => {
            pairs.push(("ORBIT_ACTIVITY_TOOLS".to_string(), tools_str));
            EnvironmentMode::ClearAndSet(pairs)
        }
        EnvironmentMode::Inherit => {
            let mut pairs: Vec<(String, String)> = std::env::vars().collect();
            pairs.push(("ORBIT_ACTIVITY_TOOLS".to_string(), tools_str));
            EnvironmentMode::ClearAndSet(pairs)
        }
    }
}

fn inject_proc_allowed_programs(mode: EnvironmentMode, programs: &[String]) -> EnvironmentMode {
    if programs.is_empty() {
        return mode;
    }
    let programs_str = programs.join(",");
    match mode {
        EnvironmentMode::ClearAndSet(mut pairs) => {
            pairs.push(("ORBIT_PROC_ALLOWED_PROGRAMS".to_string(), programs_str));
            EnvironmentMode::ClearAndSet(pairs)
        }
        EnvironmentMode::Inherit => {
            let mut pairs: Vec<(String, String)> = std::env::vars().collect();
            pairs.push(("ORBIT_PROC_ALLOWED_PROGRAMS".to_string(), programs_str));
            EnvironmentMode::ClearAndSet(pairs)
        }
    }
}

fn inject_agent_identity(
    mode: EnvironmentMode,
    execution: &ExecutionContext,
    resolved_model: Option<&str>,
) -> EnvironmentMode {
    let agent = normalize_agent_label(&execution.agent_cli);
    if agent.is_empty() {
        return mode;
    }
    let model = resolved_model.unwrap_or_default();
    let inject = |pairs: &mut Vec<(String, String)>| {
        pairs.push(("ORBIT_AGENT_NAME".to_string(), agent.clone()));
        if !model.is_empty() {
            pairs.push(("ORBIT_AGENT_MODEL".to_string(), model.to_string()));
        }
    };
    match mode {
        EnvironmentMode::ClearAndSet(mut pairs) => {
            inject(&mut pairs);
            EnvironmentMode::ClearAndSet(pairs)
        }
        EnvironmentMode::Inherit => {
            let mut pairs: Vec<(String, String)> = std::env::vars().collect();
            inject(&mut pairs);
            EnvironmentMode::ClearAndSet(pairs)
        }
    }
}

/// Resolve the effective model name for environment injection.
///
/// Mirrors the logic in `job_runner::resolved_model_name` — queries the agent
/// config and asks the provider for its canonical model name. Falls back to
/// the config-level model when the provider cannot be instantiated.
fn resolve_model_for_env<H: EnvironmentHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
) -> Option<String> {
    let config = host
        .agent_config_for(&execution.agent_cli, execution.model.as_deref())
        .ok()?;
    let model_from_config = config.model.clone();
    let agent = Agent::new(&config).ok();
    agent
        .and_then(|a| a.model_name().map(ToOwned::to_owned))
        .or(model_from_config)
}

fn normalize_agent_label(agent_cli: &str) -> String {
    std::path::Path::new(agent_cli)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(agent_cli)
        .to_ascii_lowercase()
}

fn process_agent_response<H: EnvironmentHost + AgentProtocolHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
    exec_result: &orbit_types::ExecutionResult,
    envelope: AgentResponseEnvelope,
    state: AgentResponseStatus,
) -> AttemptOutcome {
    let run_state = match state {
        AgentResponseStatus::Success => JobRunState::Success,
        AgentResponseStatus::Failed => JobRunState::Failed,
        AgentResponseStatus::Timeout => JobRunState::Timeout,
    };
    let error_code = envelope.error.as_ref().map(|error| error.code.clone());
    let error_message = envelope.error.as_ref().map(|error| error.message.clone());

    if let Some(outcome) =
        validate_agent_success(host, execution, exec_result, &envelope, run_state)
    {
        return outcome;
    }

    AttemptOutcome {
        state: run_state,
        exit_code: exec_result.exit_code,
        duration_ms: Some(exec_result.duration_ms),
        response_json: serde_json::to_value(envelope).ok(),
        error_code,
        error_message,
        protocol_violation: false,
        retry_count: 0,
    }
}

fn validate_agent_success<H: EnvironmentHost + AgentProtocolHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
    exec_result: &orbit_types::ExecutionResult,
    envelope: &AgentResponseEnvelope,
    run_state: JobRunState,
) -> Option<AttemptOutcome> {
    if run_state == JobRunState::Success
        && let Err(err) = host.validate_skill_output_schema(&execution.activity, envelope)
    {
        return Some(AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: exec_result.exit_code,
            duration_ms: Some(exec_result.duration_ms),
            response_json: serde_json::to_value(envelope).ok(),
            error_code: Some(AGENT_PROTOCOL_VIOLATION.to_string()),
            error_message: Some(err.to_string()),
            protocol_violation: true,
            retry_count: 0,
        });
    }
    if run_state == JobRunState::Success
        && let Some(result) = envelope.result.as_ref()
        && let Err(err) = host.execute_commit_request_if_present(result)
    {
        let (error_code, protocol_violation) = match err {
            OrbitError::AgentProtocolViolation(_) => (AGENT_PROTOCOL_VIOLATION.to_string(), true),
            _ => (AGENT_COMMIT_FAILED.to_string(), false),
        };
        return Some(AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: exec_result.exit_code,
            duration_ms: Some(exec_result.duration_ms),
            response_json: serde_json::to_value(envelope).ok(),
            error_code: Some(error_code),
            error_message: Some(err.to_string()),
            protocol_violation,
            retry_count: 0,
        });
    }

    None
}

fn invocation_failed_outcome(err: OrbitError) -> AttemptOutcome {
    let message = err.to_string();
    let error_code = classify_invocation_error(&message);
    AttemptOutcome::failed(&error_code, message)
}

/// Returns true if `message` contains `code` as a standalone numeric token — not
/// immediately preceded or followed by another ASCII digit.  This prevents bare
/// substrings like "500" from matching unrelated numbers such as "5001" or "15004".
fn contains_status_code(message: &str, code: &str) -> bool {
    let bytes = message.as_bytes();
    let code_bytes = code.as_bytes();
    let code_len = code_bytes.len();
    let msg_len = bytes.len();

    if msg_len < code_len {
        return false;
    }

    let mut i = 0;
    while i <= msg_len - code_len {
        if bytes[i..i + code_len] == *code_bytes {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_digit();
            let after_ok = i + code_len == msg_len || !bytes[i + code_len].is_ascii_digit();
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn classify_invocation_error(message: &str) -> String {
    let lower = message.to_lowercase();
    if lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("failed to connect")
        || lower.contains("network error")
        || lower.contains("websocket")
        || lower.contains("tls error")
    {
        AGENT_TRANSPORT_FAILURE.to_string()
    } else if lower.contains("429")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
    {
        AGENT_RATE_LIMIT.to_string()
    } else if contains_status_code(&lower, "500")
        || contains_status_code(&lower, "502")
        || contains_status_code(&lower, "503")
        || contains_status_code(&lower, "504")
        || lower.contains("overloaded")
        || lower.contains("service unavailable")
        || lower.contains("internal server error")
    {
        AGENT_PROVIDER_OVERLOAD.to_string()
    } else {
        AGENT_INVOCATION_FAILED.to_string()
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::classify_invocation_error;
    use crate::context::{
        AGENT_INVOCATION_FAILED, AGENT_OUTPUT_MISSING, AGENT_PROVIDER_OVERLOAD, AGENT_RATE_LIMIT,
        AGENT_TRANSPORT_FAILURE,
    };

    #[test]
    fn transport_failure_patterns_classify_correctly() {
        assert_eq!(
            classify_invocation_error("connection refused to 127.0.0.1:8080"),
            AGENT_TRANSPORT_FAILURE
        );
        assert_eq!(
            classify_invocation_error("failed to connect: timeout"),
            AGENT_TRANSPORT_FAILURE
        );
        assert_eq!(
            classify_invocation_error("websocket handshake failed"),
            AGENT_TRANSPORT_FAILURE
        );
    }

    #[test]
    fn provider_overload_patterns_classify_correctly() {
        assert_eq!(
            classify_invocation_error("HTTP 500 internal server error"),
            AGENT_PROVIDER_OVERLOAD
        );
        assert_eq!(
            classify_invocation_error("provider is overloaded, try again later"),
            AGENT_PROVIDER_OVERLOAD
        );
        assert_eq!(
            classify_invocation_error("503 Service Unavailable"),
            AGENT_PROVIDER_OVERLOAD
        );
    }

    #[test]
    fn rate_limit_patterns_classify_correctly() {
        assert_eq!(
            classify_invocation_error("HTTP 429 Too Many Requests"),
            AGENT_RATE_LIMIT
        );
        assert_eq!(
            classify_invocation_error("rate limit exceeded"),
            AGENT_RATE_LIMIT
        );
    }

    #[test]
    fn unrecognized_error_falls_back_to_invocation_failed() {
        assert_eq!(
            classify_invocation_error("missing required environment variable ANTHROPIC_API_KEY"),
            AGENT_INVOCATION_FAILED
        );
        assert_eq!(
            classify_invocation_error("binary not found: claude"),
            AGENT_INVOCATION_FAILED
        );
    }

    #[test]
    fn http_status_codes_do_not_match_longer_numeric_substrings() {
        // "5001" must not match 500
        assert_eq!(
            classify_invocation_error("provider returned error code 5001"),
            AGENT_INVOCATION_FAILED
        );
        // "15004" must not match 500 or 504
        assert_eq!(
            classify_invocation_error("request id 15004 was rejected"),
            AGENT_INVOCATION_FAILED
        );
        // "50200" must not match 502
        assert_eq!(
            classify_invocation_error("batch 50200 exceeded quota"),
            AGENT_INVOCATION_FAILED
        );
        // "5030" must not match 503
        assert_eq!(
            classify_invocation_error("invoice 5030 pending"),
            AGENT_INVOCATION_FAILED
        );
    }

    #[test]
    fn http_status_codes_match_at_token_boundaries() {
        // Code at the start of the message
        assert_eq!(
            classify_invocation_error("500 internal server error"),
            AGENT_PROVIDER_OVERLOAD
        );
        // Code preceded by non-digit
        assert_eq!(
            classify_invocation_error("request failed: 502 bad gateway"),
            AGENT_PROVIDER_OVERLOAD
        );
        // Code at end of message
        assert_eq!(
            classify_invocation_error("upstream returned http status 504"),
            AGENT_PROVIDER_OVERLOAD
        );
        // Mixed-case passthrough (lowercased before matching)
        assert_eq!(
            classify_invocation_error("Got HTTP 503 from upstream"),
            AGENT_PROVIDER_OVERLOAD
        );
    }

    // ── try_recover_from_task tests ──────────────────────────────────────

    use super::try_recover_from_task;
    use crate::context::{
        AgentProtocolHost, AttemptOutcome, EnvironmentHost, ExecutionContext, JobRunHost,
        RuntimeHost, TaskAutomationUpdate, TaskHost,
    };
    use chrono::Utc;
    use orbit_store::JobRunStepParams;
    use orbit_tools::ToolContext;
    use orbit_types::{
        Activity, ActorIdentity, AgentResponseEnvelope, Job, JobRun, JobRunState, JobTargetType,
        OrbitError, OrbitEvent, Role, Task, TaskPriority, TaskStatus, TaskType,
    };
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Minimal fake that implements EngineHost. Only `get_task` is exercised;
    /// all other trait methods are stubs.
    struct FakeEngineHost {
        task: Mutex<Option<Task>>,
    }

    impl FakeEngineHost {
        fn with_task(task: Task) -> Self {
            Self {
                task: Mutex::new(Some(task)),
            }
        }

        fn empty() -> Self {
            Self {
                task: Mutex::new(None),
            }
        }
    }

    impl TaskHost for FakeEngineHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            self.task
                .lock()
                .expect("fake task mutex poisoned")
                .clone()
                .filter(|t| t.id == task_id)
                .ok_or_else(|| OrbitError::TaskNotFound(task_id.to_string()))
        }
        fn list_tasks_filtered(
            &self,
            status: Option<TaskStatus>,
            priority: Option<TaskPriority>,
            parent_id: Option<&str>,
            _batch_id: Option<&str>,
        ) -> Result<Vec<Task>, OrbitError> {
            let tasks = self
                .task
                .lock()
                .expect("fake task mutex poisoned")
                .clone()
                .into_iter()
                .filter(|task| match status {
                    Some(value) => task.status == value,
                    None => true,
                })
                .filter(|task| match priority {
                    Some(value) => task.priority == value,
                    None => true,
                })
                .filter(|task| match parent_id {
                    Some(value) => task.parent_id.as_deref() == Some(value),
                    None => true,
                })
                .collect();
            Ok(tasks)
        }
        fn start_task(
            &self,
            _: &str,
            _: Option<String>,
            _: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!()
        }
        fn update_task_from_activity(
            &self,
            _: &str,
            _: TaskStatus,
            _: Option<String>,
            _: Option<String>,
            _: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!()
        }
        fn apply_task_automation_update(
            &self,
            _: &str,
            _: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            Ok(())
        }
    }

    impl JobRunHost for FakeEngineHost {
        fn list_pending_or_running_job_runs(&self, _: &str) -> Result<Vec<JobRun>, OrbitError> {
            Ok(vec![])
        }
        fn insert_job_run(
            &self,
            _: &str,
            _: u32,
            _: chrono::DateTime<Utc>,
            _: Option<serde_json::Value>,
            _: Option<String>,
        ) -> Result<JobRun, OrbitError> {
            unimplemented!()
        }
        fn mark_job_run_running(
            &self,
            _: &str,
            _: chrono::DateTime<Utc>,
            _: u32,
        ) -> Result<bool, OrbitError> {
            unimplemented!()
        }
        fn abandon_job_run(&self, _: &str, _: chrono::DateTime<Utc>) -> Result<bool, OrbitError> {
            unimplemented!()
        }
        fn complete_job_run_step(&self, _: &str, _: &JobRunStepParams) -> Result<bool, OrbitError> {
            unimplemented!()
        }
        fn finalize_job_run(
            &self,
            _: &str,
            _: JobRunState,
            _: chrono::DateTime<Utc>,
            _: Option<u64>,
        ) -> Result<bool, OrbitError> {
            unimplemented!()
        }
        fn get_job_run(&self, _: &str) -> Result<Option<JobRun>, OrbitError> {
            Ok(None)
        }
    }

    impl AgentProtocolHost for FakeEngineHost {
        fn build_agent_stdin_envelope_payload(
            &self,
            _: &ExecutionContext,
        ) -> Result<Vec<u8>, OrbitError> {
            unimplemented!()
        }
        fn validate_skill_output_schema(
            &self,
            _: &Activity,
            _: &AgentResponseEnvelope,
        ) -> Result<(), OrbitError> {
            Ok(())
        }
        fn execute_commit_request_if_present(&self, _: &Value) -> Result<(), OrbitError> {
            Ok(())
        }
    }

    impl EnvironmentHost for FakeEngineHost {
        fn agent_provider_config(&self) -> HashMap<String, String> {
            HashMap::new()
        }
        fn execution_env_inherit(&self) -> bool {
            true
        }
        fn hydrated_env_allowlist(&self, _: &[String]) -> Vec<(String, String)> {
            vec![]
        }
        fn orbit_root(&self) -> Option<String> {
            None
        }
        fn cli_command_environment(&self, _: &[String]) -> Vec<(String, String)> {
            vec![]
        }
        fn missing_required_environment_vars(&self, _: &[&str]) -> Vec<String> {
            vec![]
        }
    }

    impl RuntimeHost for FakeEngineHost {
        fn record_event(&self, _: OrbitEvent) -> Result<(), OrbitError> {
            Ok(())
        }
        fn repo_root(&self) -> Result<String, OrbitError> {
            Ok(".".to_string())
        }
        fn data_root(&self) -> &std::path::Path {
            std::path::Path::new(".")
        }
        fn acquire_file_locks(&self, _: &str, _: &str, _: &[&str]) -> Result<(), OrbitError> {
            Ok(())
        }
        fn release_file_locks(&self, _: &str) -> Result<usize, OrbitError> {
            Ok(0)
        }
        fn cleanup_stale_file_locks(&self) -> Result<usize, OrbitError> {
            Ok(0)
        }
        fn run_job_now_with_input_debug(
            &self,
            _: &str,
            _: Value,
            _: bool,
        ) -> Result<crate::context::JobRunResult, OrbitError> {
            unimplemented!()
        }
        fn validate_activity_target_exists(
            &self,
            _: JobTargetType,
            _: &str,
        ) -> Result<Activity, OrbitError> {
            unimplemented!()
        }
        fn get_job(&self, _: &str) -> Result<Option<Job>, OrbitError> {
            Ok(None)
        }
        fn run_tool_with_context_and_role(
            &self,
            _: &str,
            _: Value,
            _: Role,
            _: ToolContext,
        ) -> Result<Value, OrbitError> {
            Ok(json!({}))
        }
        fn maybe_create_failure_task(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: Option<&str>,
        ) -> Result<(), OrbitError> {
            Ok(())
        }
        fn scoring_enabled(&self) -> bool {
            false
        }
        fn scoreboard_dir(&self) -> &std::path::Path {
            std::path::Path::new(".")
        }
    }

    fn make_task(execution_summary: &str, pr_status: Option<&str>) -> Task {
        Task {
            id: "T20260328-031712".to_string(),
            parent_id: None,
            title: "test task".to_string(),
            description: "desc".to_string(),
            acceptance_criteria: vec![],
            plan: String::new(),
            execution_summary: execution_summary.to_string(),
            context_files: vec![],
            workspace_path: None,
            repo_root: None,
            assigned_to: None,
            created_by: Some("test".to_string()),
            actor_identity: ActorIdentity::agent("claude", "opus-4.6"),
            status: TaskStatus::Review,
            priority: TaskPriority::High,
            task_type: TaskType::Feature,
            pr_number: None,
            pr_status: pr_status.map(|s| s.to_string()),
            proposed_by: None,
            source_task_id: None,
            batch_id: None,
            complexity: None,
            comments: vec![],
            history: vec![],
            review_threads: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_execution_context(task_id: &str) -> ExecutionContext {
        ExecutionContext {
            activity: Activity {
                id: "implement_fix".to_string(),
                spec_type: "agent_invoke".to_string(),
                description: "test".to_string(),
                input_schema_json: json!({}),
                output_schema_json: json!({}),
                spec_config: json!({}),
                tools: vec![],
                proc_allowed_programs: vec![],
                workspace_path: None,
                created_by: None,
                is_active: true,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            job: None,
            agent_cli: "claude".to_string(),
            model: None,
            timeout_seconds: 60,
            env_extra: vec![],
            env_set: HashMap::new(),
            input: json!({ "task_id": task_id }),
            debug: false,
        }
    }

    fn make_failure_outcome() -> AttemptOutcome {
        AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: Some(0),
            duration_ms: Some(5000),
            response_json: None,
            error_code: Some(AGENT_OUTPUT_MISSING.to_string()),
            error_message: Some(
                "agent exited successfully but produced no JSON result envelope".to_string(),
            ),
            protocol_violation: false,
            retry_count: 0,
        }
    }

    #[test]
    fn recover_succeeds_when_task_has_execution_summary() {
        let task = make_task("Applied fixes to the codebase", None);
        let host = FakeEngineHost::with_task(task);
        let execution = make_execution_context("T20260328-031712");
        let original = make_failure_outcome();

        let recovered = try_recover_from_task(&host, &execution, &original);
        assert!(recovered.is_some(), "recovery should succeed");

        let outcome = recovered.unwrap();
        assert_eq!(outcome.state, JobRunState::Success);
        assert!(outcome.error_code.is_none());
        assert!(outcome.error_message.is_none());
        assert_eq!(outcome.exit_code, Some(0));
        assert_eq!(outcome.duration_ms, Some(5000));

        // Verify the synthetic envelope contains expected fields
        let envelope: AgentResponseEnvelope =
            serde_json::from_value(outcome.response_json.unwrap()).unwrap();
        assert_eq!(envelope.status, "success");
        let result = envelope.result.unwrap();
        assert_eq!(result["execution_summary"], "Applied fixes to the codebase");
        assert_eq!(result["status"], "review");
    }

    #[test]
    fn recover_succeeds_when_task_has_pr_status() {
        let task = make_task("", Some("approve"));
        let host = FakeEngineHost::with_task(task);
        let execution = make_execution_context("T20260328-031712");
        let original = make_failure_outcome();

        let recovered = try_recover_from_task(&host, &execution, &original);
        assert!(
            recovered.is_some(),
            "recovery should succeed with pr_status"
        );

        let outcome = recovered.unwrap();
        assert_eq!(outcome.state, JobRunState::Success);
        let envelope: AgentResponseEnvelope =
            serde_json::from_value(outcome.response_json.unwrap()).unwrap();
        let result = envelope.result.unwrap();
        assert_eq!(result["pr_status"], "approve");
        // execution_summary should not be present when empty
        assert!(result.get("execution_summary").is_none());
    }

    #[test]
    fn recover_returns_none_when_task_lacks_execution_summary_and_pr_status() {
        let task = make_task("", None);
        let host = FakeEngineHost::with_task(task);
        let execution = make_execution_context("T20260328-031712");
        let original = make_failure_outcome();

        let recovered = try_recover_from_task(&host, &execution, &original);
        assert!(
            recovered.is_none(),
            "recovery should return None when task has no execution_summary or pr_status"
        );
    }

    #[test]
    fn recover_returns_none_when_task_not_found() {
        let host = FakeEngineHost::empty();
        let execution = make_execution_context("T20260328-031712");
        let original = make_failure_outcome();

        let recovered = try_recover_from_task(&host, &execution, &original);
        assert!(
            recovered.is_none(),
            "recovery should return None when task does not exist"
        );
    }

    #[test]
    fn recover_returns_none_when_input_has_no_task_id() {
        let task = make_task("some summary", None);
        let host = FakeEngineHost::with_task(task);
        let mut execution = make_execution_context("T20260328-031712");
        execution.input = json!({}); // no task_id field
        let original = make_failure_outcome();

        let recovered = try_recover_from_task(&host, &execution, &original);
        assert!(
            recovered.is_none(),
            "recovery should return None when input has no task_id"
        );
    }

    #[test]
    fn recover_filters_undeclared_fields_from_synthetic_result() {
        let task = make_task("Applied fixes to the codebase", Some("approve"));
        let host = FakeEngineHost::with_task(task);
        let mut execution = make_execution_context("T20260328-031712");
        // Output schema only declares execution_summary — status and pr_status
        // should be stripped from the synthetic result.
        execution.activity.output_schema_json = json!({
            "properties": {
                "execution_summary": { "type": "string" }
            },
            "type": "object"
        });
        let original = make_failure_outcome();

        let recovered = try_recover_from_task(&host, &execution, &original);
        assert!(recovered.is_some(), "recovery should succeed");

        let outcome = recovered.unwrap();
        assert_eq!(outcome.state, JobRunState::Success);

        let envelope: AgentResponseEnvelope =
            serde_json::from_value(outcome.response_json.unwrap()).unwrap();
        let result = envelope.result.unwrap();
        assert_eq!(result["execution_summary"], "Applied fixes to the codebase");
        // Undeclared fields must not appear in the result
        assert!(
            result.get("status").is_none(),
            "status should be filtered out when not in output schema"
        );
        assert!(
            result.get("pr_status").is_none(),
            "pr_status should be filtered out when not in output schema"
        );
    }

    #[test]
    fn recover_returns_none_when_all_meaningful_fields_filtered() {
        // Task has pr_status but the output schema only declares "status" —
        // after filtering, no meaningful fields remain, so recovery should fail.
        let task = make_task("", Some("approve"));
        let host = FakeEngineHost::with_task(task);
        let mut execution = make_execution_context("T20260328-031712");
        execution.activity.output_schema_json = json!({
            "properties": {
                "status": { "type": "string" }
            },
            "type": "object"
        });
        let original = make_failure_outcome();

        let recovered = try_recover_from_task(&host, &execution, &original);
        assert!(
            recovered.is_none(),
            "recovery should return None when meaningful fields are filtered out"
        );
    }
}

fn prepare_exec_args(
    invocation: &orbit_agent::AgentResponse,
) -> Result<(Vec<String>, Option<NamedTempFile>), OrbitError> {
    let mut args = invocation.args.clone();
    let mut stdout_schema_file = None;

    if let Some(schema) = invocation.stdout_schema_json.as_ref() {
        let mut file = NamedTempFile::new().map_err(|error| {
            OrbitError::Execution(format!(
                "failed to create temporary agent output schema file: {error}"
            ))
        })?;
        serde_json::to_writer(file.as_file_mut(), schema).map_err(|error| {
            OrbitError::Execution(format!(
                "failed to write temporary agent output schema file: {error}"
            ))
        })?;
        file.as_file_mut().flush().map_err(|error| {
            OrbitError::Execution(format!(
                "failed to flush temporary agent output schema file: {error}"
            ))
        })?;

        args.push("--output-schema".to_string());
        args.push(file.path().to_string_lossy().into_owned());
        stdout_schema_file = Some(file);
    }

    Ok((args, stdout_schema_file))
}

fn format_timeout_error_message(exec_result: &orbit_types::ExecutionResult) -> String {
    let stderr = exec_result.stderr.trim();
    if stderr.is_empty() {
        return "agent timed out before producing JSON stdout".to_string();
    }
    format!("agent timed out before producing JSON stdout; stderr: {stderr}")
}
