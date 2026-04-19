use std::io::Write;

use orbit_agent::{Agent, AgentRequest, AgentResponseStatus};
use orbit_exec::{ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::{
    ExecutorDef, InvocationTrace, JobRunState, OrbitError, StdoutFormat, resolve_agent_model_pair,
};
use serde_json::Value;
use tempfile::NamedTempFile;

use crate::context::{
    AGENT_INVOCATION_FAILED, AGENT_PROTOCOL_VIOLATION, AGENT_TIMEOUT, AgentProtocolHost,
    AttemptOutcome, EnvironmentHost, ExecutionContext, ExecutorLookupHost, apply_env_set,
    inject_state_env,
};

use super::{
    environment::{
        inject_activity_tools, inject_agent_identity, inject_proc_allowed_programs,
        resolve_model_for_env,
    },
    response::{
        format_timeout_error_message, invocation_failed_outcome, parse_agent_output,
        process_agent_response,
    },
};

#[derive(Debug, Clone)]
pub(super) struct ResolvedAgentExecution {
    pub(super) label: String,
    pub(super) command: String,
    pub(super) args: Vec<String>,
    pub(super) env: std::collections::HashMap<String, String>,
    pub(super) stdout_format: Option<StdoutFormat>,
    pub(super) model: Option<String>,
}

pub(super) fn execute_with_cwd<H: EnvironmentHost + AgentProtocolHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
    working_dir: Option<String>,
    resolved: &ResolvedAgentExecution,
) -> AttemptOutcome {
    let invocation = match build_agent_invocation(host, execution, resolved) {
        Ok(invocation) => invocation,
        Err(outcome) => return *outcome,
    };
    let exec_result =
        match execute_agent_process(host, execution, invocation, working_dir, resolved) {
            Ok(result) => result,
            Err(outcome) => return *outcome,
        };

    if agent_process_was_interrupted(&exec_result) && exec_result.stdout.trim().is_empty() {
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

    if orbit_agent::is_timeout(&exec_result) && exec_result.stdout.trim().is_empty() {
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

    match parse_agent_output(&exec_result, resolved.stdout_format) {
        Ok((envelope, state, trace)) => {
            if state == AgentResponseStatus::Success
                && envelope.result.is_none()
                && exec_result.exit_code == Some(0)
                && !orbit_agent::is_timeout(&exec_result)
            {
                let mut outcome = AttemptOutcome::success(0, exec_result.duration_ms, Value::Null);
                outcome.invocation_trace = trace;
                return outcome;
            }
            process_agent_response(host, execution, &exec_result, envelope, state, trace)
        }
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

pub(super) fn resolve_agent_execution<H: EnvironmentHost + ExecutorLookupHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
    bound_executor: Option<&ExecutorDef>,
) -> Result<ResolvedAgentExecution, OrbitError> {
    let executor_def = if let Some(def) = bound_executor {
        Some(def.clone())
    } else if let Some(name) = execution
        .activity
        .executor
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        host.get_executor_def(name)?
    } else if !execution.agent_cli.trim().is_empty() {
        host.get_executor_def(&execution.agent_cli)?
    } else {
        None
    };

    let label = if !execution.agent_cli.trim().is_empty() {
        execution.agent_cli.trim().to_string()
    } else if let Some(name) = execution
        .activity
        .executor
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        name.to_string()
    } else if let Some(def) = executor_def.as_ref() {
        def.name.clone()
    } else {
        return Err(OrbitError::InvalidInput(
            "agent_invoke execution requires agent_cli or activity.executor".to_string(),
        ));
    };

    let command = executor_def
        .as_ref()
        .and_then(|def| def.command.clone())
        .unwrap_or_else(|| label.clone());
    let model = resolve_executor_model(executor_def.as_ref(), &command, execution);

    Ok(ResolvedAgentExecution {
        label,
        command,
        args: executor_def
            .as_ref()
            .map(|def| def.args.clone())
            .unwrap_or_default(),
        env: executor_def
            .as_ref()
            .map(|def| def.env.clone())
            .unwrap_or_default(),
        stdout_format: executor_def.and_then(|def| def.stdout_format),
        model,
    })
}

fn agent_process_was_interrupted(exec_result: &orbit_types::ExecutionResult) -> bool {
    !exec_result.success && exec_result.stderr.contains("process interrupted by signal")
}

fn build_agent_invocation<H: EnvironmentHost + AgentProtocolHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
    resolved: &ResolvedAgentExecution,
) -> Result<orbit_agent::AgentInvocationSpec, Box<AttemptOutcome>> {
    let config = host
        .agent_config_for(&resolved.command, resolved.model.as_deref())
        .map_err(|error| Box::new(invocation_failed_outcome(error)))?;
    let agent = Agent::new(&config).map_err(|error| Box::new(invocation_failed_outcome(error)))?;
    let stdin_payload: Vec<u8> = host
        .build_agent_stdin_envelope_payload(execution)
        .map_err(|error| Box::new(invocation_failed_outcome(error)))?;

    let (invocation, _) = agent
        .invoke(
            match &execution.job {
                Some(job) => AgentRequest::job(
                    job.job_id.clone(),
                    execution.activity.id.clone(),
                    stdin_payload,
                ),
                None => AgentRequest::activity(execution.activity.id.clone(), stdin_payload),
            }
            .with_verbose(execution.debug),
        )
        .map_err(|error| Box::new(invocation_failed_outcome(error)))?;

    let missing_env = host.missing_required_environment_vars(invocation.required_env_vars);
    if !missing_env.is_empty() {
        let vars = missing_env.join(", ");
        return Err(Box::new(AttemptOutcome::failed(
            AGENT_INVOCATION_FAILED,
            format!(
                "missing required environment variable(s) for provider '{}': {vars}. \
configure .orbit/config.toml [execution.env].pass and set these variables in the parent shell.",
                invocation.runtime_key
            ),
        )));
    }

    Ok(invocation)
}

fn execute_agent_process<H: EnvironmentHost + AgentProtocolHost + ?Sized>(
    host: &H,
    execution: &ExecutionContext,
    invocation: orbit_agent::AgentInvocationSpec,
    working_dir: Option<String>,
    resolved: &ResolvedAgentExecution,
) -> Result<orbit_types::ExecutionResult, Box<AttemptOutcome>> {
    let (args, _stdout_schema_file) = prepare_exec_args(&invocation, &resolved.args)
        .map_err(|error| Box::new(invocation_failed_outcome(error)))?;

    let resolved_model = resolve_model_for_env(host, resolved);
    let mut env_set = resolved.env.clone();
    env_set.extend(execution.env_set.clone());
    let environment_mode = apply_env_set(
        inject_state_env(
            inject_proc_allowed_programs(
                inject_agent_identity(
                    inject_activity_tools(
                        host.execution_environment_mode(&execution.env_extra),
                        &execution.activity.tools,
                    ),
                    &resolved.label,
                    resolved_model.as_deref(),
                ),
                &execution.activity.proc_allowed_programs,
            ),
            execution,
        ),
        &env_set,
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
    .map_err(|error| Box::new(invocation_failed_outcome(error)))
}

fn prepare_exec_args(
    invocation: &orbit_agent::AgentInvocationSpec,
    executor_args: &[String],
) -> Result<(Vec<String>, Option<NamedTempFile>), OrbitError> {
    let mut args = executor_args.to_vec();
    args.extend(invocation.args.clone());
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

fn resolve_executor_model(
    executor_def: Option<&ExecutorDef>,
    agent_cli: &str,
    execution: &ExecutionContext,
) -> Option<String> {
    if let Some(model) = execution
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(model.to_string());
    }

    let tier = execution
        .model_tier
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    if let Some(model) = executor_def.and_then(|def| def.model_for_tier(tier)) {
        return Some(model.to_string());
    }

    match tier {
        "strong" => resolve_agent_model_pair(agent_cli).map(|pair| pair.orchestrator),
        "weak" => resolve_agent_model_pair(agent_cli).map(|pair| pair.helper),
        _ => None,
    }
}
