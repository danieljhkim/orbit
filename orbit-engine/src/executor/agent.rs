use std::io::Write;

use orbit_agent::{Agent, AgentRequest, AgentResponseStatus, parse_and_validate_response};
use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{AgentResponseEnvelope, JobRunState, OrbitError};
use tempfile::NamedTempFile;

use super::ActivityExecutor;
use crate::context::{
    AGENT_COMMIT_FAILED, AGENT_INVOCATION_FAILED, AGENT_PROTOCOL_VIOLATION,
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
        };
    }

    match parse_and_validate_response(&exec_result) {
        Ok((envelope, state)) => {
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
        },
        Err(err) => AttemptOutcome {
            state: JobRunState::Failed,
            exit_code: exec_result.exit_code,
            duration_ms: Some(exec_result.duration_ms),
            response_json: None,
            error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
            error_message: Some(err.to_string()),
            protocol_violation: false,
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

    let environment_mode = inject_activity_tools(
        host.execution_environment_mode(&execution.env_extra),
        &execution.activity.tools,
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
        });
    }

    None
}

fn invocation_failed_outcome(err: OrbitError) -> AttemptOutcome {
    let message = err.to_string();
    let error_code = classify_invocation_error(&message);
    AttemptOutcome::failed(&error_code, message)
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
    } else if lower.contains("500")
        || lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
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
