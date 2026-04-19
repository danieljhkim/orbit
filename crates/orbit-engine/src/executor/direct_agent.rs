use orbit_exec::{ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{
    AgentResponseEnvelope, AgentRunError, ExecutorDef, InvocationTrace, JobRunState, OrbitError,
};
use serde_json::Value;

use super::ActivityExecutor;
use crate::context::{
    AGENT_INVOCATION_FAILED, AGENT_PROTOCOL_VIOLATION, AGENT_TIMEOUT, AgentProtocolHost,
    AttemptOutcome, EnvironmentHost, ExecutionContext, ExecutorHost, apply_env_set,
    execution_working_directory_with_task, inject_state_env,
};

// Re-use the environment helpers defined in agent.rs.  They are crate-public
// (non-`pub(super)`) so we can reference them via the sibling module path.
use super::agent::{inject_activity_tools, inject_agent_identity, inject_proc_allowed_programs};

pub struct DirectAgentExecutor {
    bound_executor: ExecutorDef,
}

impl DirectAgentExecutor {
    pub fn from_executor_def(def: ExecutorDef) -> Self {
        Self {
            bound_executor: def,
        }
    }
}

impl ActivityExecutor for DirectAgentExecutor {
    fn spec_type(&self) -> &str {
        "direct_agent"
    }

    fn execute(&self, host: ExecutorHost<'_>, execution: &ExecutionContext) -> AttemptOutcome {
        let agent_host = host.agent();
        let working_dir = execution_working_directory_with_task(&agent_host, execution);

        // --- Build stdin envelope ---
        let stdin_payload = match agent_host.build_agent_stdin_envelope_payload(execution) {
            Ok(payload) => payload,
            Err(err) => return invocation_failed_outcome(err),
        };

        // --- Resolve command + args from the bound ExecutorDef ---
        let command = match self.bound_executor.command.as_ref() {
            Some(cmd) => cmd.clone(),
            None => {
                return invocation_failed_outcome(OrbitError::InvalidInput(
                    "direct_agent executor requires a 'command' field in the executor def"
                        .to_string(),
                ));
            }
        };
        let args = self.bound_executor.args.clone();

        // --- Resolve model (step.model → step.model_tier → executor def models) ---
        let model = resolve_executor_model(&self.bound_executor, execution);

        // --- Assemble environment ---
        let label = self.bound_executor.name.clone();
        let mut env_set = self.bound_executor.env.clone();
        env_set.extend(execution.env_set.clone());
        let environment_mode = apply_env_set(
            inject_state_env(
                inject_proc_allowed_programs(
                    inject_agent_identity(
                        inject_activity_tools(
                            agent_host.execution_environment_mode(&execution.env_extra),
                            &execution.activity.tools,
                        ),
                        &label,
                        model.as_deref(),
                    ),
                    &execution.activity.proc_allowed_programs,
                ),
                execution,
            ),
            &env_set,
        );

        // --- Build ExecRequest and run ---
        let exec_result = match run_process(
            &ExecRequest {
                program: command,
                args,
                current_dir: working_dir,
                timeout_ms: Some(execution.timeout_seconds.saturating_mul(1000)),
                stdin_mode: StdinMode::Bytes(stdin_payload),
                environment_mode,
                debug: execution.debug,
            },
            &NoSandbox,
        ) {
            Ok(result) => result,
            Err(err) => return invocation_failed_outcome(err),
        };

        // --- Check for process interruption ---
        if !exec_result.success
            && exec_result.stderr.contains("process interrupted by signal")
            && exec_result.stdout.trim().is_empty()
        {
            return AttemptOutcome {
                state: JobRunState::Cancelled,
                exit_code: exec_result.exit_code,
                duration_ms: Some(exec_result.duration_ms),
                invocation_trace: InvocationTrace {
                    duration_ms: exec_result.duration_ms,
                    ..InvocationTrace::default()
                },
                response_json: None,
                error_code: None,
                error_message: Some(exec_result.stderr.trim().to_string()),
                protocol_violation: false,
                retry_count: 0,
            };
        }

        // --- Check for timeout ---
        if is_timeout(&exec_result) && exec_result.stdout.trim().is_empty() {
            return AttemptOutcome {
                state: JobRunState::Timeout,
                exit_code: exec_result.exit_code,
                duration_ms: Some(exec_result.duration_ms),
                invocation_trace: InvocationTrace {
                    duration_ms: exec_result.duration_ms,
                    ..InvocationTrace::default()
                },
                response_json: None,
                error_code: Some(AGENT_TIMEOUT.to_string()),
                error_message: Some(format_timeout_error_message(&exec_result)),
                protocol_violation: false,
                retry_count: 0,
            };
        }

        // --- Parse response envelope from stdout ---
        match parse_response_envelope(&exec_result) {
            Ok(envelope) => map_envelope_to_outcome(&agent_host, execution, &exec_result, envelope),
            Err(OrbitError::AgentProtocolViolation(message)) => AttemptOutcome {
                state: JobRunState::Failed,
                exit_code: exec_result.exit_code,
                duration_ms: Some(exec_result.duration_ms),
                invocation_trace: InvocationTrace {
                    duration_ms: exec_result.duration_ms,
                    ..InvocationTrace::default()
                },
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
                invocation_trace: InvocationTrace {
                    duration_ms: exec_result.duration_ms,
                    ..InvocationTrace::default()
                },
                response_json: None,
                error_code: Some(AGENT_INVOCATION_FAILED.to_string()),
                error_message: Some(err.to_string()),
                protocol_violation: false,
                retry_count: 0,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Model resolution
// ---------------------------------------------------------------------------

fn resolve_executor_model(
    executor_def: &ExecutorDef,
    execution: &ExecutionContext,
) -> Option<String> {
    // 1. Explicit model on the step
    if let Some(model) = execution
        .model
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return Some(model.to_string());
    }

    // 2. Model tier mapped through executor def
    let tier = execution
        .model_tier
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())?;

    executor_def.model_for_tier(tier).map(|m| m.to_string())
}

// ---------------------------------------------------------------------------
// Timeout detection (standalone — does not depend on orbit-agent)
// ---------------------------------------------------------------------------

fn is_timeout(exec_result: &orbit_types::ExecutionResult) -> bool {
    !exec_result.success && exec_result.stderr.contains("process timed out")
}

fn format_timeout_error_message(exec_result: &orbit_types::ExecutionResult) -> String {
    let stderr = exec_result.stderr.trim();
    if stderr.is_empty() {
        return "agent timed out before producing JSON stdout".to_string();
    }
    format!("agent timed out before producing JSON stdout; stderr: {stderr}")
}

// ---------------------------------------------------------------------------
// Response parsing (standalone — does not depend on orbit-agent)
// ---------------------------------------------------------------------------

/// Parse an `AgentResponseEnvelope` from the process stdout.
///
/// Direct agents must emit a single protocol envelope. The only synthesized
/// fallback we keep is a plain failure when the child exits non-zero without
/// producing any stdout at all.
fn parse_response_envelope(
    exec_result: &orbit_types::ExecutionResult,
) -> Result<AgentResponseEnvelope, OrbitError> {
    let stdout = exec_result.stdout.trim();

    if stdout.is_empty() {
        if exec_result.exit_code == Some(0) {
            return Err(OrbitError::AgentProtocolViolation(
                "stdout does not contain an Orbit response envelope".to_string(),
            ));
        }
        return Ok(AgentResponseEnvelope {
            schema_version: 1,
            status: "failed".to_string(),
            result: None,
            error: Some(AgentRunError {
                code: AGENT_INVOCATION_FAILED.to_string(),
                message: synthetic_error_message(exec_result),
                details: Value::Null,
            }),
            duration_ms: Some(exec_result.duration_ms),
        });
    }

    // Try to parse as an envelope directly.
    let value: Value = serde_json::from_str(stdout).map_err(|err| {
        OrbitError::AgentProtocolViolation(format!("stdout is not valid JSON: {err}"))
    })?;

    // If it looks like an envelope (has schemaVersion + status), deserialize it.
    if value.get("schemaVersion").is_some() && value.get("status").is_some() {
        let envelope: AgentResponseEnvelope = serde_json::from_value(value).map_err(|err| {
            OrbitError::AgentProtocolViolation(format!(
                "stdout contains envelope-like JSON but failed to deserialize: {err}"
            ))
        })?;
        if envelope.schema_version != 1 {
            return Err(OrbitError::AgentProtocolViolation(format!(
                "unsupported schemaVersion: {}",
                envelope.schema_version
            )));
        }
        validate_exit_alignment(exec_result, &envelope)?;
        return Ok(envelope);
    }

    Err(OrbitError::AgentProtocolViolation(
        "stdout does not contain an Orbit response envelope".to_string(),
    ))
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

fn validate_exit_alignment(
    exec_result: &orbit_types::ExecutionResult,
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

// ---------------------------------------------------------------------------
// Envelope → AttemptOutcome mapping
// ---------------------------------------------------------------------------

fn map_envelope_to_outcome<H: EnvironmentHost + AgentProtocolHost + ?Sized>(
    host: &H,
    _execution: &ExecutionContext,
    exec_result: &orbit_types::ExecutionResult,
    envelope: AgentResponseEnvelope,
) -> AttemptOutcome {
    let trace = InvocationTrace {
        duration_ms: exec_result.duration_ms,
        ..InvocationTrace::default()
    };

    let run_state = match envelope.status.as_str() {
        "success" => JobRunState::Success,
        "timeout" => JobRunState::Timeout,
        _ => JobRunState::Failed,
    };

    // Side-effect success: exit 0, status success, no result payload.
    if run_state == JobRunState::Success
        && envelope.result.is_none()
        && exec_result.exit_code == Some(0)
        && !is_timeout(exec_result)
    {
        let mut outcome = AttemptOutcome::success(0, exec_result.duration_ms, Value::Null);
        outcome.invocation_trace = trace;
        return outcome;
    }

    // Commit request handling for successful runs.
    if run_state == JobRunState::Success {
        if let Some(result) = envelope.result.as_ref() {
            if let Err(err) = host.execute_commit_request_if_present(result) {
                let (error_code, protocol_violation) = match err {
                    OrbitError::AgentProtocolViolation(_) => {
                        (AGENT_PROTOCOL_VIOLATION.to_string(), true)
                    }
                    _ => (crate::context::AGENT_COMMIT_FAILED.to_string(), false),
                };
                return AttemptOutcome {
                    state: JobRunState::Failed,
                    exit_code: exec_result.exit_code,
                    duration_ms: Some(exec_result.duration_ms),
                    invocation_trace: trace,
                    response_json: serde_json::to_value(&envelope).ok(),
                    error_code: Some(error_code),
                    error_message: Some(err.to_string()),
                    protocol_violation,
                    retry_count: 0,
                };
            }
        }
    }

    let error_code = envelope.error.as_ref().map(|e| e.code.clone());
    let error_message = envelope.error.as_ref().map(|e| e.message.clone());

    AttemptOutcome {
        state: run_state,
        exit_code: exec_result.exit_code,
        duration_ms: Some(exec_result.duration_ms),
        invocation_trace: trace,
        response_json: serde_json::to_value(envelope).ok(),
        error_code,
        error_message,
        protocol_violation: false,
        retry_count: 0,
    }
}

fn invocation_failed_outcome(err: OrbitError) -> AttemptOutcome {
    let message = err.to_string();
    AttemptOutcome::failed(AGENT_INVOCATION_FAILED, message)
}
