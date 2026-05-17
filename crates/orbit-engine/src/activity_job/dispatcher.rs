use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use orbit_common::types::activity_job::V2AuditEventKind;
use orbit_common::types::activity_job::{
    ActivityV2Spec, AgentLoopSpec, AgentRole, Backend, DeterministicSpec, ShellSpec,
};

use crate::context::AgentRoleConfig;
use orbit_common::types::{
    ExecutorSandboxKind, InvocationTrace, LearningInjectionCaps, LearningReminder, OrbitError,
    ResolvedFsProfile, TokenUsage, ToolCallTrace,
};
use orbit_tools::{FsAuditLogger, FsCallEvent, FsCallEventKind, ToolContext};
use serde_json::Value;
use thiserror::Error;

use super::agent_loop_driver::drive_agent_loop;
use super::audit_writer::V2AuditWriter;
use super::cli_runner::run_cli_backend;
use super::groundhog::run_groundhog_activity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCliExecutor {
    pub command: String,
    pub args: Vec<String>,
}

/// Sandbox descriptor for a CLI invocation. The host resolves the executor's
/// `sandbox` declaration and the activity's `fsProfile` against the active
/// policy and workspace root; the engine compiles the OS-specific payload
/// just before spawn (keeping the orbit-exec dependency local to orbit-engine).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSandbox {
    /// OS sandbox primitive selected by the executor declaration.
    pub kind: ExecutorSandboxKind,
    /// Workspace-absolute resolved `read` / `modify` rules from the activity's
    /// `FsProfile`. The engine passes this to `orbit_exec::compile_*_profile`
    /// to produce a kernel-shaped payload.
    pub fs_profile: ResolvedFsProfile,
    /// Whether to fall back to bare exec if the OS primitive is unavailable.
    pub allow_fallback: bool,
}

/// Orbit-core-owned responsibilities the v2 dispatcher delegates back across
/// the engine→core boundary: deterministic action execution (which needs the
/// runtime's tool registry + ToolContext) and provider credential sourcing
/// (which needs env/config access).
///
/// Agent-loop construction itself is NOT on this trait — it lives in
/// `orbit_engine::activity_job::agent_loop_driver::drive_agent_loop`, so implementors
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
        tool_context: ToolContext,
    ) -> Result<Value, DispatchError>;

    /// Source the API key for a given provider (e.g. `"anthropic"`). Returns
    /// the raw key as a `String` so nothing orbit-agent-shaped bleeds across
    /// the boundary. Implementors typically read from env or config.
    fn api_key_for(&self, provider: &str) -> Result<String, DispatchError>;

    /// Resolve the CLI executor command and static args for a given v2
    /// provider name (§3.1 backend: cli path). Workspace / env overrides live
    /// in the host so the engine's CLI runner stays environment-agnostic.
    /// Returning an error is the structured failure path when a provider has no
    /// CLI mapping (e.g. `openai_compat` which is HTTP-only).
    fn resolve_cli_executor(&self, provider: &str) -> Result<ResolvedCliExecutor, DispatchError>;

    /// Return provider-specific CLI runtime config for `backend: cli`.
    ///
    /// Most providers ignore this today. Codex uses it for sandbox,
    /// approval-policy, and writable-directory arguments that must stay dynamic
    /// rather than living in the static executor definition.
    fn provider_cli_config(&self, _provider: &str) -> HashMap<String, String> {
        HashMap::new()
    }

    /// Resolve the OS sandbox payload for a CLI invocation. The host reads
    /// the executor's `sandbox` declaration, materializes the activity's
    /// `fs_profile` against the active policy, and compiles the result via
    /// `orbit-exec`. Returns `Ok(None)` when the executor has no sandbox
    /// declared (today's behavior). Returns a structured error on
    /// platform mismatch (e.g. `macos-sandbox-exec` on Linux) so the
    /// activity fails closed at dispatch time.
    ///
    /// `subprocess_cwd` is the resolved working directory the subprocess
    /// will run in. The host uses it to re-allow the active worktree path
    /// after the policy's `denyModify .orbit/**` rule when the cwd is a
    /// jrun worktree under `.orbit/state/worktrees/`. Without this, every
    /// non-codex provider (claude/gemini) cannot write inside its own
    /// worktree because the deny rule wins last-match. See T20260508-17.
    fn resolve_executor_sandbox(
        &self,
        _provider: &str,
        _fs_profile: Option<&str>,
        _subprocess_cwd: Option<&Path>,
    ) -> Result<Option<ResolvedSandbox>, DispatchError> {
        Ok(None)
    }

    /// Optional task snapshot to embed in a backend: cli agent envelope.
    ///
    /// The engine keeps this as untyped JSON so orbit-core can source task data
    /// without leaking store or task-query details into orbit-engine.
    fn task_context_for_agent_input(&self, _input: &Value) -> Result<Option<Value>, DispatchError> {
        Ok(None)
    }

    /// Return project-learning reminders relevant to the task represented by
    /// `input`. Implementors that do not own task storage can ignore this; the
    /// engine preserves the original prompt when the returned set is empty.
    fn learning_reminders_for_task(
        &self,
        _input: &Value,
        _caps: LearningInjectionCaps,
    ) -> Result<Vec<LearningReminder>, DispatchError> {
        Ok(Vec::new())
    }

    fn tool_context_for_activity(
        &self,
        run_id: Option<&str>,
        fs_profile: Option<&str>,
        fs_audit: Option<Arc<dyn FsAuditLogger>>,
    ) -> ToolContext;

    fn persist_invocation_trace(
        &self,
        _job_run_id: &str,
        _activity_id: &str,
        _provider: &str,
        _model: Option<&str>,
        _input: &Value,
        _trace: &InvocationTrace,
    ) -> Result<(), DispatchError> {
        Ok(())
    }

    /// Resolve the selected crew role from the active workspace's
    /// `config.toml`.
    /// Mirrors [`crate::context::EnvironmentHost::agent_role_config`]; the
    /// engine's job dispatcher receives only `&dyn V2RuntimeHost`, so this
    /// method is the seam dispatch consults at run time. Default returns
    /// `None`, which makes the resolver fall through to inline activity
    /// values (preserving pre-ADR-029 dispatch behaviour for tests and other
    /// hosts that have no role-config layer).
    fn agent_role_config(&self, _role: AgentRole) -> Option<AgentRoleConfig> {
        None
    }

    fn agent_role_config_for_input(
        &self,
        role: AgentRole,
        _input: &Value,
    ) -> Option<AgentRoleConfig> {
        self.agent_role_config(role)
    }
}

/// Input bundle for a single v2 activity dispatch.
pub struct V2DispatchInput<'a> {
    pub activity_name: &'a str,
    pub spec: &'a ActivityV2Spec,
    pub fs_profile: Option<&'a str>,
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
    pub invocation: Option<DispatchInvocationTrace>,
}

#[derive(Debug, Clone)]
pub struct DispatchInvocationTrace {
    pub provider: String,
    pub model: Option<String>,
    pub trace: InvocationTrace,
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

    #[error("groundhog run failed: {0}")]
    GroundhogFailed(String),

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
    dispatch_v2_activity_inner(input, true)
}

pub(crate) fn dispatch_v2_activity_without_run_id_injection(
    input: V2DispatchInput<'_>,
) -> Result<DispatchOutcome, DispatchError> {
    dispatch_v2_activity_inner(input, false)
}

fn dispatch_v2_activity_inner(
    input: V2DispatchInput<'_>,
    inject_run_id_into_input: bool,
) -> Result<DispatchOutcome, DispatchError> {
    let activity_input = if inject_run_id_into_input {
        inject_run_id(&input.input, input.run_id)
    } else {
        input.input.clone()
    };
    let activity_type = match input.spec {
        ActivityV2Spec::AgentLoop(_) => "agent_loop",
        ActivityV2Spec::Groundhog(_) => "groundhog",
        ActivityV2Spec::Deterministic(_) => "deterministic",
        ActivityV2Spec::Shell(_) => "shell",
    };

    let activity_event_id = input
        .audit
        .emit(
            orbit_common::types::activity_job::V2AuditEventKind::ActivityStarted {
                activity_name: input.activity_name.to_string(),
                activity_type: activity_type.to_string(),
            },
        )
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
                &activity_input,
                input.fs_profile,
            ),
            None => Err(DispatchError::HostRequired("agent_loop")),
        },
        ActivityV2Spec::Groundhog(spec) => match input.host {
            Some(host) => {
                if !spec.provider.has_http_transport() {
                    return Err(DispatchError::UnwiredHttpTransport {
                        provider: spec.provider.as_str().to_string(),
                    });
                }
                run_groundhog_activity(
                    host,
                    input.activity_name,
                    spec,
                    input.run_id,
                    input.audit.clone(),
                    &activity_input,
                    input.fs_profile,
                )
            }
            None => Err(DispatchError::HostRequired("groundhog")),
        },
        ActivityV2Spec::Deterministic(spec) => match input.host {
            Some(host) => run_deterministic(
                host,
                input.run_id,
                spec,
                input.fs_profile,
                input.audit.clone(),
                &activity_input,
            ),
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
    let _ = input.audit.emit(
        orbit_common::types::activity_job::V2AuditEventKind::ActivityFinished {
            activity_name: input.activity_name.to_string(),
            outcome: outcome_str.to_string(),
        },
    );

    result
}

fn inject_run_id(input: &Value, run_id: &str) -> Value {
    let Value::Object(map) = input else {
        return input.clone();
    };
    if map.contains_key("run_id") {
        return input.clone();
    }

    let mut augmented = map.clone();
    augmented.insert("run_id".to_string(), Value::String(run_id.to_string()));
    Value::Object(augmented)
}

fn run_deterministic(
    host: &dyn V2RuntimeHost,
    run_id: &str,
    spec: &DeterministicSpec,
    fs_profile: Option<&str>,
    audit: Arc<V2AuditWriter>,
    input: &Value,
) -> Result<DispatchOutcome, DispatchError> {
    let tool_context = host.tool_context_for_activity(
        Some(run_id),
        fs_profile,
        Some(v2_fs_audit_logger(audit.clone())),
    );
    let output = host.run_deterministic(&spec.action, &spec.config, input, tool_context)?;
    Ok(DispatchOutcome {
        success: true,
        output,
        message: None,
        invocation: None,
    })
}

fn run_agent_loop_activity(
    host: &dyn V2RuntimeHost,
    activity_name: &str,
    spec: &AgentLoopSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    input: &Value,
    fs_profile: Option<&str>,
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
            run_agent_loop_via_driver(host, spec, run_id, audit, input, fs_profile)
        }
        Backend::Cli => run_cli_backend(host, spec, run_id, audit, input, fs_profile),
    }
}

#[allow(dead_code)]
fn run_agent_loop_via_driver(
    host: &dyn V2RuntimeHost,
    spec: &AgentLoopSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    input: &Value,
    fs_profile: Option<&str>,
) -> Result<DispatchOutcome, DispatchError> {
    // Sourcing only: orbit-core pulls the provider credential from wherever
    // makes sense (env var, config, secrets manager). We treat a sourcing
    // failure as `None` so `drive_agent_loop` can still honor the offline
    // replay path (ORBIT_V2_REPLAY) without credentials. When the driver
    // actually needs a key and none is present, it errors structurally.
    let api_key = host.api_key_for("anthropic").ok();
    let started = Instant::now();
    let outcome = drive_agent_loop(
        spec,
        api_key.as_deref(),
        run_id,
        audit,
        input,
        host,
        fs_profile,
    )?;
    let trace = loop_outcome_trace(&outcome, started.elapsed().as_millis() as u64);
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "final_message".to_string(),
        Value::String(outcome.final_message.clone()),
    );
    metadata.insert(
        "terminate_reason".to_string(),
        Value::String(format!("{:?}", outcome.terminate_reason)),
    );
    metadata.insert(
        "usage".to_string(),
        serde_json::json!({
            "input_tokens": outcome.usage.input_tokens,
            "output_tokens": outcome.usage.output_tokens,
        }),
    );
    Ok(DispatchOutcome {
        success: true,
        output: agent_loop_output_from_final_message(&outcome.final_message, metadata),
        message: None,
        invocation: Some(DispatchInvocationTrace {
            provider: spec.provider.as_str().to_string(),
            model: spec.model.clone(),
            trace,
        }),
    })
}

pub(crate) fn loop_outcome_trace(
    outcome: &orbit_agent::loop_engine::LoopOutcome,
    duration_ms: u64,
) -> InvocationTrace {
    let mut seq = 0;
    let tool_calls = outcome
        .trace
        .iter()
        .flat_map(|iteration| iteration.tool_calls.iter())
        .map(|tool_name| {
            seq += 1;
            ToolCallTrace {
                seq,
                tool_name: tool_name.clone(),
                result_bytes: 0,
                result_payload: None,
            }
        })
        .collect();

    InvocationTrace {
        usage: TokenUsage {
            input: outcome.usage.input_tokens,
            cache_read: outcome.usage.cache_read_input_tokens,
            cache_create: outcome.usage.cache_creation_input_tokens,
            output: outcome.usage.output_tokens,
        },
        tool_calls,
        duration_ms,
    }
}

pub(crate) fn agent_loop_output_from_final_message(
    final_message: &str,
    metadata: serde_json::Map<String, Value>,
) -> Value {
    let mut output = parse_structured_final_message(final_message).unwrap_or_default();
    for (key, value) in metadata {
        output.entry(key).or_insert(value);
    }
    Value::Object(output)
}

fn parse_structured_final_message(final_message: &str) -> Option<serde_json::Map<String, Value>> {
    let parsed: Value = serde_json::from_str(final_message.trim()).ok()?;
    match parsed {
        Value::Object(map) => {
            if (map.contains_key("schemaVersion") || map.contains_key("status"))
                && let Some(Value::Object(result)) = map.get("result")
            {
                return Some(result.clone());
            }
            Some(map)
        }
        _ => None,
    }
}

struct V2FsAuditLogger {
    audit: Arc<V2AuditWriter>,
}

impl FsAuditLogger for V2FsAuditLogger {
    fn emit(&self, event: FsCallEvent) -> Result<(), OrbitError> {
        let kind = match event.kind {
            FsCallEventKind::Request => V2AuditEventKind::FsCallRequest {
                profile: event.profile,
                op: event.op,
                path: event.path,
                allowed: event.allowed,
                matched_rule: event.matched_rule,
            },
            FsCallEventKind::Result => V2AuditEventKind::FsCallResult {
                profile: event.profile,
                op: event.op,
                path: event.path,
                allowed: event.allowed,
                matched_rule: event.matched_rule,
            },
            FsCallEventKind::Denied => V2AuditEventKind::FsCallDenied {
                profile: event.profile,
                op: event.op,
                path: event.path,
                allowed: event.allowed,
                matched_rule: event.matched_rule,
            },
        };

        self.audit
            .emit(kind)
            .map(|_| ())
            .map_err(|error| OrbitError::Execution(format!("audit write failed: {error}")))
    }
}

pub(crate) fn v2_fs_audit_logger(audit: Arc<V2AuditWriter>) -> Arc<dyn FsAuditLogger> {
    Arc::new(V2FsAuditLogger { audit })
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
        invocation: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_loop_output_exposes_structured_final_message_fields() {
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "final_message".to_string(),
            Value::String("raw".to_string()),
        );

        let output = agent_loop_output_from_final_message(
            r#"{"cycle_notes":"dispatched one","dispatched_run_ids":["jrun-1"]}"#,
            metadata,
        );

        assert_eq!(output["cycle_notes"], json!("dispatched one"));
        assert_eq!(output["dispatched_run_ids"], json!(["jrun-1"]));
        assert_eq!(output["final_message"], json!("raw"));
    }

    #[test]
    fn agent_loop_output_unwraps_response_envelope_result() {
        let output = agent_loop_output_from_final_message(
            r#"{"schemaVersion":1,"status":"success","result":{"dispatched_run_ids":[]}}"#,
            serde_json::Map::new(),
        );

        assert_eq!(output["dispatched_run_ids"], json!([]));
        assert!(output.get("schemaVersion").is_none());
    }
}
