use std::io::Write;

use orbit_agent::{Agent, AgentRequest, AgentResponseStatus, parse_and_validate_response};
use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{AgentResponseEnvelope, JobRunState, OrbitError};
use tempfile::NamedTempFile;

use super::ActivityExecutor;
use crate::context::{
    AGENT_COMMIT_FAILED, AGENT_INVOCATION_FAILED, AGENT_OUTPUT_MISSING, AGENT_PROTOCOL_VIOLATION,
    AGENT_PROVIDER_OVERLOAD, AGENT_RATE_LIMIT, AGENT_TIMEOUT, AGENT_TRANSPORT_FAILURE,
    AgentProtocolHost, AttemptOutcome, EngineHost, EnvironmentHost, ExecutionContext,
    execution_working_directory,
};

pub struct AgentExecutor;

impl ActivityExecutor for AgentExecutor {
    fn spec_type(&self) -> &str {
        "agent_invoke"
    }

    fn execute(&self, host: &dyn EngineHost, execution: &ExecutionContext) -> AttemptOutcome {
        execute(host, execution)
    }
}

pub fn execute<H: EnvironmentHost + AgentProtocolHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
) -> AttemptOutcome {
    let invocation = match build_agent_invocation(host, execution) {
        Ok(invocation) => invocation,
        Err(outcome) => return outcome,
    };
    let exec_result = match execute_agent_process(host, execution, invocation) {
        Ok(result) => result,
        Err(outcome) => return outcome,
    };

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

    let invocation = agent
        .invoke(match &execution.job {
            Some(job) => AgentRequest::job(
                job.job_id.clone(),
                execution.activity.id.clone(),
                stdin_payload,
            ),
            None => AgentRequest::activity(execution.activity.id.clone(), stdin_payload),
        })
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
) -> Result<orbit_types::ExecutionResult, AttemptOutcome> {
    let (args, _stdout_schema_file) =
        prepare_exec_args(&invocation).map_err(invocation_failed_outcome)?;

    let resolved_model = resolve_model_for_env(host, execution);
    let environment_mode = inject_agent_identity(
        inject_activity_tools(
            host.execution_environment_mode(&execution.env_extra),
            &execution.activity.tools,
        ),
        execution,
        resolved_model.as_deref(),
    );

    run_process(
        &ExecRequest {
            program: invocation.program,
            args,
            current_dir: execution_working_directory(execution),
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
        AGENT_INVOCATION_FAILED, AGENT_PROVIDER_OVERLOAD, AGENT_RATE_LIMIT, AGENT_TRANSPORT_FAILURE,
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
