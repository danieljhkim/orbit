use std::path::Path;

use orbit_types::{
    AgentResponseEnvelope, ExecutionResult, OrbitError, SchedulerRunState, SchedulerTargetType,
};
use serde_json::{Deserializer, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdinAdapter {
    OrbitEnvelopeJson,
    PromptWithEmbeddedEnvelope,
}

#[derive(Debug, Clone)]
pub struct AgentInvocation {
    pub program: String,
    pub args: Vec<String>,
    pub stdin_adapter: StdinAdapter,
}

pub fn build_invocation(
    agent_cli: &str,
    target_type: SchedulerTargetType,
    target_id: &str,
) -> Result<AgentInvocation, OrbitError> {
    let provider = provider_key(agent_cli);

    let (args, stdin_adapter) = match provider.as_str() {
        // Keep provider-specific mappers explicit to avoid hidden command drift.
        // `mock-agent` implements Orbit's native scheduled flags and consumes JSON stdin directly.
        "mock-agent" => (
            default_scheduled_args(target_type, target_id),
            StdinAdapter::OrbitEnvelopeJson,
        ),
        // Modern `codex` CLI uses `exec` for non-interactive mode and reads prompt from stdin
        // when no prompt arg is supplied.
        "codex" => (
            vec!["exec".to_string()],
            StdinAdapter::PromptWithEmbeddedEnvelope,
        ),
        // Modern `claude` CLI uses `-p` for non-interactive print mode.
        "claude" => (
            vec![
                "-p".to_string(),
                "--output-format".to_string(),
                "text".to_string(),
            ],
            StdinAdapter::PromptWithEmbeddedEnvelope,
        ),
        _ => {
            return Err(OrbitError::UnsupportedAgentProvider(provider));
        }
    };

    Ok(AgentInvocation {
        program: agent_cli.to_string(),
        args,
        stdin_adapter,
    })
}

pub fn build_stdin_payload(invocation: &AgentInvocation, envelope_json: &[u8]) -> Vec<u8> {
    match invocation.stdin_adapter {
        StdinAdapter::OrbitEnvelopeJson => envelope_json.to_vec(),
        StdinAdapter::PromptWithEmbeddedEnvelope => {
            let envelope_text = String::from_utf8_lossy(envelope_json);
            format!(
                "You are Orbit's scheduled scheduler agent.\n\
Read the execution envelope JSON and perform the requested job.\n\
Return exactly one JSON object and nothing else.\n\
Required response schema:\n\
{{\"schemaVersion\":1,\"status\":\"success|failed|timeout\",\"result\":{{}},\"error\":null,\"durationMs\":123}}\n\
Rules:\n\
- Output valid JSON only.\n\
- No markdown fences.\n\
- If execution cannot complete, return status=\"failed\" with non-empty error.code and error.message.\n\
- Keep result as a JSON object.\n\
Execution envelope:\n\
{envelope_text}\n"
            )
            .into_bytes()
        }
    }
}

pub fn parse_and_validate_response(
    exec_result: &ExecutionResult,
) -> Result<(AgentResponseEnvelope, SchedulerRunState), OrbitError> {
    let stderr_hint = exec_result.stderr.trim();
    if exec_result.stdout.trim().is_empty() {
        if !stderr_hint.is_empty() {
            return Err(OrbitError::Execution(format!(
                "agent did not produce JSON stdout; stderr: {}",
                truncate(stderr_hint, 300)
            )));
        }
        return Err(OrbitError::AgentProtocolViolation(
            "agent stdout is empty".to_string(),
        ));
    }

    let value = match parse_single_json_document(&exec_result.stdout) {
        Ok(value) => value,
        Err(err) => {
            if is_invocation_failure(exec_result) {
                return Err(OrbitError::Execution(format!(
                    "agent did not produce valid JSON stdout; stderr: {}; stdout: {}",
                    truncate(stderr_hint, 300),
                    truncate(exec_result.stdout.trim(), 300),
                )));
            }
            return Err(err);
        }
    };
    let envelope: AgentResponseEnvelope = serde_json::from_value(value).map_err(|e| {
        OrbitError::AgentProtocolViolation(format!("invalid agent response envelope: {e}"))
    })?;

    if envelope.schema_version != 1 {
        return Err(OrbitError::AgentProtocolViolation(format!(
            "unsupported schemaVersion: {}",
            envelope.schema_version
        )));
    }

    let state = match envelope.status.as_str() {
        "success" => SchedulerRunState::Success,
        "failed" => {
            let Some(error) = &envelope.error else {
                return Err(OrbitError::AgentProtocolViolation(
                    "failed status requires error object".to_string(),
                ));
            };
            if error.code.trim().is_empty() {
                return Err(OrbitError::AgentProtocolViolation(
                    "failed status requires non-empty error.code".to_string(),
                ));
            }
            SchedulerRunState::Failed
        }
        "timeout" => SchedulerRunState::Timeout,
        other => {
            return Err(OrbitError::AgentProtocolViolation(format!(
                "unknown status: {other}"
            )));
        }
    };

    validate_exit_alignment(exec_result, &envelope)?;

    Ok((envelope, state))
}

fn parse_single_json_document(stdout: &str) -> Result<Value, OrbitError> {
    let mut stream = Deserializer::from_str(stdout).into_iter::<Value>();

    let Some(first) = stream.next() else {
        return Err(OrbitError::AgentProtocolViolation(
            "stdout does not contain a JSON document".to_string(),
        ));
    };

    let first = first.map_err(|e| {
        OrbitError::AgentProtocolViolation(format!("stdout is not valid JSON: {e}"))
    })?;

    if stream.next().is_some() {
        return Err(OrbitError::AgentProtocolViolation(
            "stdout contains multiple JSON documents".to_string(),
        ));
    }

    Ok(first)
}

fn validate_exit_alignment(
    exec_result: &ExecutionResult,
    envelope: &AgentResponseEnvelope,
) -> Result<(), OrbitError> {
    let timed_out = is_timeout(exec_result);

    if timed_out && envelope.status != "timeout" {
        return Err(OrbitError::AgentProtocolViolation(
            "timeout process must report status=timeout".to_string(),
        ));
    }

    if timed_out {
        return Ok(());
    }

    let exit_code = exec_result.exit_code.unwrap_or(1);
    if exit_code == 0 && envelope.status != "success" {
        return Err(OrbitError::AgentProtocolViolation(
            "exit_code=0 must report status=success".to_string(),
        ));
    }
    if exit_code != 0 && envelope.status == "success" {
        return Err(OrbitError::AgentProtocolViolation(
            "non-zero exit code cannot report status=success".to_string(),
        ));
    }

    Ok(())
}

pub fn is_timeout(exec_result: &ExecutionResult) -> bool {
    !exec_result.success && exec_result.stderr.contains("process timed out")
}

fn is_invocation_failure(exec_result: &ExecutionResult) -> bool {
    exec_result.exit_code.unwrap_or(1) != 0 && !exec_result.stderr.trim().is_empty()
}

fn default_scheduled_args(target_type: SchedulerTargetType, target_id: &str) -> Vec<String> {
    vec![
        "run".to_string(),
        "--target-type".to_string(),
        target_type.to_string(),
        "--target-id".to_string(),
        target_id.to_string(),
        "--mode".to_string(),
        "scheduled".to_string(),
        "--output".to_string(),
        "json".to_string(),
    ]
}

pub(crate) fn provider_key(agent_cli: &str) -> String {
    Path::new(agent_cli)
        .file_name()
        .and_then(|v| v.to_str())
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| agent_cli.to_ascii_lowercase())
}

fn truncate(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        return value.to_string();
    }
    let mut output = value[..max_len].to_string();
    output.push_str("...");
    output
}
