use std::process::Command;
use std::sync::Arc;

use orbit_types::v2::{ActivityV2Spec, AgentLoopSpec, Backend, DeterministicSpec, ShellSpec};
use serde_json::Value;
use thiserror::Error;

use super::agent_loop_driver::drive_agent_loop;
use super::audit_writer::V2AuditWriter;
use super::cli_runner::run_cli_backend;

/// Orbit-core-owned responsibilities the v2 dispatcher delegates back across
/// the engine→core boundary: deterministic action execution (which needs the
/// runtime's tool registry + ToolContext) and provider credential sourcing
/// (which needs env/config access).
///
/// Agent-loop construction itself is NOT on this trait — it lives in
/// `orbit_engine::v2::agent_loop_driver::drive_agent_loop`, so implementors
/// never have to name orbit-agent types. The dispatcher calls
/// `host.api_key_for(provider)?` then `drive_agent_loop(spec, &api_key, ...)`
/// directly.
pub trait V2RuntimeHost: Send + Sync {
    /// Dispatch a deterministic action by name. The host looks up `action`
    /// in its registry and returns the action's structured output.
    fn run_deterministic(
        &self,
        action: &str,
        config: &Value,
        input: &Value,
    ) -> Result<Value, DispatchError>;

    /// Source the API key for a given provider (e.g. `"anthropic"`). Returns
    /// the raw key as a `String` so nothing orbit-agent-shaped bleeds across
    /// the boundary. Implementors typically read from env or config.
    fn api_key_for(&self, provider: &str) -> Result<String, DispatchError>;

    /// Resolve the CLI binary command for a given v2 provider name (§3.1
    /// backend: cli path). Workspace / env overrides live in the host so the
    /// engine's CLI runner stays environment-agnostic. Returning an error is
    /// the structured failure path when a provider has no CLI mapping (e.g.
    /// `openai_compat` which is HTTP-only).
    fn resolve_cli_command(&self, provider: &str) -> Result<String, DispatchError>;
}

/// Input bundle for a single v2 activity dispatch.
pub struct V2DispatchInput<'a> {
    pub activity_name: &'a str,
    pub spec: &'a ActivityV2Spec,
    pub input: Value,
    pub audit: Arc<V2AuditWriter>,
    pub run_id: &'a str,
    /// Runtime host for agent_loop + deterministic paths. Callers that only
    /// dispatch shell activities may pass `None`; shell is self-contained
    /// via `std::process::Command`.
    pub host: Option<&'a dyn V2RuntimeHost>,
}

/// Outcome of a v2 dispatch attempt. Kept separate from v1's AttemptOutcome
/// to avoid coupling v2 callers to the v1 engine context.
#[derive(Debug, Clone)]
pub struct DispatchOutcome {
    pub success: bool,
    pub output: Value,
    pub message: Option<String>,
}

#[derive(Debug, Error, Clone)]
pub enum DispatchError {
    #[error("runtime host required for activity type `{0}` but none provided")]
    HostRequired(&'static str),

    #[error("deterministic action not registered: {0}")]
    DeterministicActionNotRegistered(String),

    #[error("deterministic action `{action}` failed: {message}")]
    DeterministicActionFailed { action: String, message: String },

    #[error("shell program `{0}` not in allowed_programs")]
    ShellProgramNotAllowed(String),

    #[error("shell spawn failed: {0}")]
    ShellSpawnFailed(String),

    #[error("shell exited with code {code}; expected one of {expected:?}")]
    ShellExitedUnexpected { code: i32, expected: Vec<i32> },

    #[error("agent_loop run failed: {0}")]
    AgentLoopFailed(String),

    /// §3.1 no-silent-fallback: `backend: http` requested a provider whose
    /// HTTP transport is not wired. Must surface as a structured error rather
    /// than silently dispatching to CLI.
    #[error(
        "provider `{provider}` has no HTTP transport wired at this phase — set backend: cli or choose a provider whose HTTP path is implemented"
    )]
    UnwiredHttpTransport { provider: String },

    /// `backend: auto` was observed past the load-time resolver — every
    /// dispatch site must see a concrete backend. Indicates a caller that
    /// forgot to run `resolve_*_backends` before dispatching.
    #[error("backend `auto` leaked past load-time resolution (step id `{step_id}`)")]
    UnresolvedAutoBackend { step_id: String },

    /// CLI subprocess invocation failed at the host layer (e.g. failed to
    /// spawn, or provider key unknown). Wraps the host's error text verbatim.
    #[error("cli invocation failed: {0}")]
    CliInvocationFailed(String),

    /// Tool-allowlist denial (§6). Non-retryable — the retry wrapper must not
    /// re-attempt a denied call. Phase 2 formerly translated this to
    /// `Ok(terminated)`; Phase 3 surfaces it structurally so the DAG executor
    /// can classify it.
    #[error("tool `{tool_name}` denied at iteration {iteration}")]
    ToolDenied { tool_name: String, iteration: u32 },

    /// Job validation rejected the spec at load time.
    #[error("job validation failed: {0}")]
    JobValidation(String),

    /// Generic job-executor error — distinct from per-activity failures.
    #[error("job executor: {0}")]
    JobExecution(String),

    #[error("audit write failed: {0}")]
    AuditFailed(String),
}

impl DispatchError {
    /// Whether this error should bypass the retry wrapper. Tool denials,
    /// unknown deterministic actions, shell allowlist violations, and
    /// validation errors are non-retryable (§4.3: "Non-retryable errors —
    /// schema violations, allowlist denials, cancellation — skip retry").
    pub fn is_non_retryable(&self) -> bool {
        matches!(
            self,
            DispatchError::ToolDenied { .. }
                | DispatchError::DeterministicActionNotRegistered(_)
                | DispatchError::ShellProgramNotAllowed(_)
                | DispatchError::JobValidation(_)
                | DispatchError::HostRequired(_)
                | DispatchError::UnwiredHttpTransport { .. }
                | DispatchError::UnresolvedAutoBackend { .. }
        )
    }
}

/// Dispatch a v2 activity by type. Emits §7 activity.started/finished
/// events around the per-type runner and nests the runner's events beneath.
pub fn dispatch_v2_activity(input: V2DispatchInput<'_>) -> Result<DispatchOutcome, DispatchError> {
    let activity_type = match input.spec {
        ActivityV2Spec::AgentLoop(_) => "agent_loop",
        ActivityV2Spec::Deterministic(_) => "deterministic",
        ActivityV2Spec::Shell(_) => "shell",
    };

    let activity_event_id = input
        .audit
        .emit(orbit_types::v2::V2AuditEventKind::ActivityStarted {
            activity_name: input.activity_name.to_string(),
            activity_type: activity_type.to_string(),
        })
        .map_err(|err| DispatchError::AuditFailed(format!("{err:?}")))?;
    let _ = input.audit.push_parent(activity_event_id);

    let result = match input.spec {
        ActivityV2Spec::AgentLoop(spec) => match input.host {
            Some(host) => run_agent_loop_activity(
                host,
                input.activity_name,
                spec,
                input.run_id,
                input.audit.clone(),
                &input.input,
            ),
            None => Err(DispatchError::HostRequired("agent_loop")),
        },
        ActivityV2Spec::Deterministic(spec) => match input.host {
            Some(host) => run_deterministic(host, spec, &input.input),
            None => Err(DispatchError::HostRequired("deterministic")),
        },
        ActivityV2Spec::Shell(spec) => run_shell(spec),
    };

    let _ = input.audit.pop_parent();
    let outcome_str = match &result {
        Ok(o) if o.success => "success",
        Ok(_) => "failed",
        Err(_) => "error",
    };
    let _ = input
        .audit
        .emit(orbit_types::v2::V2AuditEventKind::ActivityFinished {
            activity_name: input.activity_name.to_string(),
            outcome: outcome_str.to_string(),
        });

    result
}

fn run_deterministic(
    host: &dyn V2RuntimeHost,
    spec: &DeterministicSpec,
    input: &Value,
) -> Result<DispatchOutcome, DispatchError> {
    let output = host.run_deterministic(&spec.action, &spec.config, input)?;
    Ok(DispatchOutcome {
        success: true,
        output,
        message: None,
    })
}

fn run_agent_loop_activity(
    host: &dyn V2RuntimeHost,
    activity_name: &str,
    spec: &AgentLoopSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    input: &Value,
) -> Result<DispatchOutcome, DispatchError> {
    match spec.backend {
        Backend::Auto => Err(DispatchError::UnresolvedAutoBackend {
            step_id: activity_name.to_string(),
        }),
        Backend::Http => {
            if !spec.provider.has_http_transport() {
                return Err(DispatchError::UnwiredHttpTransport {
                    provider: spec.provider.as_str().to_string(),
                });
            }
            run_agent_loop_via_driver(host, spec, run_id, audit, input)
        }
        Backend::Cli => run_cli_backend(host, spec, run_id, audit, input),
    }
}

fn run_agent_loop_via_driver(
    host: &dyn V2RuntimeHost,
    spec: &AgentLoopSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    input: &Value,
) -> Result<DispatchOutcome, DispatchError> {
    // Sourcing only: orbit-core pulls the provider credential from wherever
    // makes sense (env var, config, secrets manager). We treat a sourcing
    // failure as `None` so `drive_agent_loop` can still honor the offline
    // replay path (ORBIT_V2_REPLAY) without credentials. When the driver
    // actually needs a key and none is present, it errors structurally.
    let api_key = host.api_key_for("anthropic").ok();
    let outcome = drive_agent_loop(spec, api_key.as_deref(), run_id, audit, input)?;
    Ok(DispatchOutcome {
        success: true,
        output: serde_json::json!({
            "final_message": outcome.final_message,
            "terminate_reason": format!("{:?}", outcome.terminate_reason),
            "usage": {
                "input_tokens": outcome.usage.input_tokens,
                "output_tokens": outcome.usage.output_tokens,
            },
        }),
        message: None,
    })
}

fn run_shell(spec: &ShellSpec) -> Result<DispatchOutcome, DispatchError> {
    if !spec.allowed_programs.contains(&spec.program) {
        return Err(DispatchError::ShellProgramNotAllowed(spec.program.clone()));
    }
    let output = Command::new(&spec.program)
        .args(&spec.args)
        .output()
        .map_err(|err| DispatchError::ShellSpawnFailed(format!("{err}")))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let expected = if spec.expected_exit_codes.is_empty() {
        vec![0]
    } else {
        spec.expected_exit_codes.clone()
    };
    let success = expected.contains(&exit_code);

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    Ok(DispatchOutcome {
        success,
        output: serde_json::json!({
            "program": spec.program,
            "args": spec.args,
            "exit_code": exit_code,
            "stdout": stdout,
            "stderr": stderr,
        }),
        message: (!success).then(|| format!("exit {exit_code} not in {expected:?}")),
    })
}
