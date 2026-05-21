use orbit_common::types::{ExecutorDef, InvocationTrace, JobRunState, OrbitError};
use orbit_exec::{ExecRequest, NoSandbox, StdinMode, run_process};

use orbit_exec::EnvironmentMode;

use super::ActivityExecutor;
use crate::context::{
    AGENT_INVOCATION_FAILED, AGENT_TIMEOUT, AgentProtocolHost, AttemptOutcome, EnvironmentHost,
    ExecutionContext, ExecutorHost, apply_env_set, execution_working_directory_with_task,
    inject_state_env,
};

fn inject_activity_tools(mode: EnvironmentMode, tools: &[String]) -> EnvironmentMode {
    inject_csv_env(mode, "ORBIT_ACTIVITY_TOOLS", tools)
}

fn inject_proc_allowed_programs(mode: EnvironmentMode, programs: &[String]) -> EnvironmentMode {
    inject_csv_env(mode, "ORBIT_PROC_ALLOWED_PROGRAMS", programs)
}

fn inject_agent_identity(
    mode: EnvironmentMode,
    agent_label: &str,
    execution: &ExecutionContext,
) -> EnvironmentMode {
    let agent = normalize_agent_label(agent_label);
    if agent.is_empty() {
        return mode;
    }

    inject_environment(mode, |pairs| {
        pairs.push(("ORBIT_AGENT_NAME".to_string(), agent.clone()));
        if let Some(model) = execution
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            pairs.push(("ORBIT_AGENT_MODEL".to_string(), model.to_string()));
        }
    })
}

fn inject_csv_env(mode: EnvironmentMode, key: &str, values: &[String]) -> EnvironmentMode {
    if values.is_empty() {
        return mode;
    }

    let joined = values.join(",");
    inject_environment(mode, |pairs| pairs.push((key.to_string(), joined.clone())))
}

fn inject_environment<F>(mode: EnvironmentMode, inject: F) -> EnvironmentMode
where
    F: FnOnce(&mut Vec<(String, String)>),
{
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

fn normalize_agent_label(agent_cli: &str) -> String {
    std::path::Path::new(agent_cli)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(agent_cli)
        .to_ascii_lowercase()
}

// pub(crate) widened for tests/ layout under ORB-00225; test reaches via exposed surface.
pub(crate) fn append_runtime_model_args(
    args: &mut Vec<String>,
    model_flag: Option<&str>,
    model: Option<&str>,
) {
    let (Some(model_flag), Some(model)) = (model_flag, model) else {
        return;
    };

    if model_flag.trim().is_empty() || model.trim().is_empty() {
        return;
    }

    args.push(model_flag.to_string());
    args.push(model.to_string());
}

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
        let mut args = self.bound_executor.args.clone();
        append_runtime_model_args(
            &mut args,
            self.bound_executor.model_flag.as_deref(),
            execution.model.as_deref(),
        );

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
                        execution,
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

        map_exec_result_to_outcome(&exec_result)
    }
}

fn is_timeout(exec_result: &orbit_common::types::ExecutionResult) -> bool {
    !exec_result.success && exec_result.stderr.contains("process timed out")
}

fn synthetic_error_message(exec_result: &orbit_common::types::ExecutionResult) -> String {
    let stderr = exec_result.stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }
    format!(
        "agent execution failed with exit code {:?}",
        exec_result.exit_code
    )
}

fn base_outcome(exec_result: &orbit_common::types::ExecutionResult) -> AttemptOutcome {
    let trace = InvocationTrace {
        duration_ms: exec_result.duration_ms,
        ..InvocationTrace::default()
    };
    AttemptOutcome {
        state: JobRunState::Failed,
        exit_code: exec_result.exit_code,
        duration_ms: Some(exec_result.duration_ms),
        invocation_trace: trace,
        response_json: None,
        error_code: None,
        error_message: None,
        protocol_violation: false,
        retry_count: 0,
    }
}

// pub(crate) widened for tests/ layout under ORB-00225; test reaches via exposed surface.
pub(crate) fn map_exec_result_to_outcome(
    exec_result: &orbit_common::types::ExecutionResult,
) -> AttemptOutcome {
    let mut outcome = base_outcome(exec_result);
    if !exec_result.success && exec_result.stderr.contains("process interrupted by signal") {
        outcome.state = JobRunState::Cancelled;
        outcome.error_message = Some(exec_result.stderr.trim().to_string());
        return outcome;
    }
    if is_timeout(exec_result) {
        outcome.state = JobRunState::Timeout;
        outcome.error_code = Some(AGENT_TIMEOUT.to_string());
        outcome.error_message = Some(synthetic_error_message(exec_result));
        return outcome;
    }
    if exec_result.success {
        outcome.state = JobRunState::Success;
        return outcome;
    }
    outcome.error_code = Some(AGENT_INVOCATION_FAILED.to_string());
    outcome.error_message = Some(synthetic_error_message(exec_result));
    outcome
}

fn invocation_failed_outcome(err: OrbitError) -> AttemptOutcome {
    let message = err.to_string();
    AttemptOutcome::failed(AGENT_INVOCATION_FAILED, message)
}
