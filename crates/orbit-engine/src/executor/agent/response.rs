use orbit_agent::{AgentResponseStatus, parse_and_validate_response};
use orbit_types::{AgentResponseEnvelope, InvocationTrace, JobRunState, OrbitError};
use serde_json::Value;

use crate::context::{
    AGENT_COMMIT_FAILED, AGENT_INVOCATION_FAILED, AGENT_PROTOCOL_VIOLATION,
    AGENT_PROVIDER_OVERLOAD, AGENT_RATE_LIMIT, AGENT_TIMEOUT, AGENT_TRANSPORT_FAILURE,
    AgentProtocolHost, AttemptOutcome, EnvironmentHost, ExecutionContext,
};

pub(super) fn process_agent_response<H: EnvironmentHost + AgentProtocolHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
    exec_result: &orbit_types::ExecutionResult,
    envelope: AgentResponseEnvelope,
    state: AgentResponseStatus,
    invocation_trace: InvocationTrace,
) -> AttemptOutcome {
    let run_state = match state {
        AgentResponseStatus::Success => JobRunState::Success,
        AgentResponseStatus::Failed => JobRunState::Failed,
        AgentResponseStatus::Timeout => JobRunState::Timeout,
    };
    let error_code = envelope.error.as_ref().map(|error| error.code.clone());
    let error_message = envelope.error.as_ref().map(|error| error.message.clone());

    if let Some(outcome) = validate_agent_success(
        host,
        execution,
        exec_result,
        &envelope,
        run_state,
        invocation_trace.clone(),
    ) {
        return outcome;
    }

    AttemptOutcome {
        state: run_state,
        exit_code: exec_result.exit_code,
        duration_ms: Some(exec_result.duration_ms),
        invocation_trace,
        response_json: serde_json::to_value(envelope).ok(),
        error_code,
        error_message,
        protocol_violation: false,
        retry_count: 0,
    }
}

pub(super) fn invocation_failed_outcome(err: OrbitError) -> AttemptOutcome {
    let message = err.to_string();
    let error_code = classify_invocation_error(&message);
    AttemptOutcome::failed(&error_code, message)
}

pub(super) fn parse_agent_output(
    exec_result: &orbit_types::ExecutionResult,
    stdout_format: Option<&str>,
) -> Result<(AgentResponseEnvelope, AgentResponseStatus, InvocationTrace), OrbitError> {
    match stdout_format.unwrap_or("envelope") {
        "envelope" => parse_and_validate_response(exec_result),
        "json" => synthesize_json_response(exec_result),
        "text" => synthesize_text_response(exec_result),
        other => Err(OrbitError::Execution(format!(
            "unsupported agent executor stdout_format '{other}'"
        ))),
    }
}

pub(super) fn format_timeout_error_message(exec_result: &orbit_types::ExecutionResult) -> String {
    let stderr = exec_result.stderr.trim();
    if stderr.is_empty() {
        return "agent timed out before producing JSON stdout".to_string();
    }
    format!("agent timed out before producing JSON stdout; stderr: {stderr}")
}

fn validate_agent_success<H: EnvironmentHost + AgentProtocolHost + ?Sized>(
    host: &H,
    _execution: &ExecutionContext,
    exec_result: &orbit_types::ExecutionResult,
    envelope: &AgentResponseEnvelope,
    run_state: JobRunState,
    invocation_trace: InvocationTrace,
) -> Option<AttemptOutcome> {
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
            invocation_trace,
            response_json: serde_json::to_value(envelope).ok(),
            error_code: Some(error_code),
            error_message: Some(err.to_string()),
            protocol_violation,
            retry_count: 0,
        });
    }

    None
}

fn contains_status_code(message: &str, code: &str) -> bool {
    let bytes = message.as_bytes();
    let code_bytes = code.as_bytes();
    let code_len = code_bytes.len();
    let msg_len = bytes.len();

    if msg_len < code_len {
        return false;
    }

    let mut index = 0;
    while index <= msg_len - code_len {
        if bytes[index..index + code_len] == *code_bytes {
            let before_ok = index == 0 || !bytes[index - 1].is_ascii_digit();
            let after_ok = index + code_len == msg_len || !bytes[index + code_len].is_ascii_digit();
            if before_ok && after_ok {
                return true;
            }
        }
        index += 1;
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

fn synthesize_json_response(
    exec_result: &orbit_types::ExecutionResult,
) -> Result<(AgentResponseEnvelope, AgentResponseStatus, InvocationTrace), OrbitError> {
    let trace = InvocationTrace {
        duration_ms: exec_result.duration_ms,
        ..InvocationTrace::default()
    };
    if orbit_agent::is_timeout(exec_result) {
        return Ok(timeout_response(exec_result));
    }
    let stdout = exec_result.stdout.trim();
    let result = if stdout.is_empty() {
        None
    } else {
        Some(serde_json::from_str::<Value>(stdout).map_err(|error| {
            OrbitError::AgentProtocolViolation(format!(
                "stdout is not valid JSON for stdout_format=json: {error}"
            ))
        })?)
    };
    if exec_result.exit_code.unwrap_or(1) == 0 {
        return Ok((
            AgentResponseEnvelope {
                schema_version: 1,
                status: "success".to_string(),
                result,
                error: None,
                duration_ms: Some(exec_result.duration_ms),
            },
            AgentResponseStatus::Success,
            trace,
        ));
    }
    Ok(failed_response(exec_result, result))
}

fn synthesize_text_response(
    exec_result: &orbit_types::ExecutionResult,
) -> Result<(AgentResponseEnvelope, AgentResponseStatus, InvocationTrace), OrbitError> {
    let trace = InvocationTrace {
        duration_ms: exec_result.duration_ms,
        ..InvocationTrace::default()
    };
    if orbit_agent::is_timeout(exec_result) {
        return Ok(timeout_response(exec_result));
    }
    let result =
        (!exec_result.stdout.trim().is_empty()).then(|| Value::String(exec_result.stdout.clone()));
    if exec_result.exit_code.unwrap_or(1) == 0 {
        return Ok((
            AgentResponseEnvelope {
                schema_version: 1,
                status: "success".to_string(),
                result,
                error: None,
                duration_ms: Some(exec_result.duration_ms),
            },
            AgentResponseStatus::Success,
            trace,
        ));
    }
    Ok(failed_response(exec_result, result))
}

fn timeout_response(
    exec_result: &orbit_types::ExecutionResult,
) -> (AgentResponseEnvelope, AgentResponseStatus, InvocationTrace) {
    (
        AgentResponseEnvelope {
            schema_version: 1,
            status: "timeout".to_string(),
            result: None,
            error: Some(orbit_types::AgentRunError {
                code: AGENT_TIMEOUT.to_string(),
                message: "agent timed out".to_string(),
                details: Value::Null,
            }),
            duration_ms: Some(exec_result.duration_ms),
        },
        AgentResponseStatus::Timeout,
        InvocationTrace {
            duration_ms: exec_result.duration_ms,
            ..InvocationTrace::default()
        },
    )
}

fn failed_response(
    exec_result: &orbit_types::ExecutionResult,
    result: Option<Value>,
) -> (AgentResponseEnvelope, AgentResponseStatus, InvocationTrace) {
    (
        AgentResponseEnvelope {
            schema_version: 1,
            status: "failed".to_string(),
            result,
            error: Some(orbit_types::AgentRunError {
                code: AGENT_INVOCATION_FAILED.to_string(),
                message: synthetic_error_message(exec_result),
                details: Value::Null,
            }),
            duration_ms: Some(exec_result.duration_ms),
        },
        AgentResponseStatus::Failed,
        InvocationTrace {
            duration_ms: exec_result.duration_ms,
            ..InvocationTrace::default()
        },
    )
}

fn synthetic_error_message(exec_result: &orbit_types::ExecutionResult) -> String {
    let stderr = exec_result.stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    let stdout = exec_result.stdout.trim();
    if !stdout.is_empty() {
        return stdout.to_string();
    }
    "agent execution failed".to_string()
}
