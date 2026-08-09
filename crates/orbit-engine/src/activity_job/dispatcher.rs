use std::sync::Arc;
use std::time::Instant;

use orbit_common::types::activity_job::V2AuditEventKind;
use orbit_common::types::activity_job::{
    ActivityV2Spec, AgentLoopSpec, Backend, DeterministicSpec,
};

use crate::context::RuntimeHost;
use orbit_common::types::{
    DeterministicAction, ExecutorSandboxKind, InvocationTrace, McpCapability, OrbitError,
    ResolvedFsProfile, TokenUsage, ToolCallTrace,
};
use orbit_tools::{FsAuditLogger, FsCallEvent, FsCallEventKind};
use serde_json::Value;
use thiserror::Error;

use super::agent_loop_driver::drive_agent_loop;
use super::audit_writer::V2AuditWriter;
use super::cli_runner::run_cli_backend;

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
    /// Whether the subprocess runs in an Orbit-owned disposable worktree.
    /// Linux may snapshot-expand non-subtree deny globs only in this case.
    pub managed_worktree: bool,
}

/// Input bundle for a single v2 activity dispatch.
pub struct V2DispatchInput<'a> {
    pub activity_name: &'a str,
    pub spec: &'a ActivityV2Spec,
    pub fs_profile: Option<&'a str>,
    pub input: Value,
    pub audit: Arc<V2AuditWriter>,
    pub run_id: &'a str,
    /// Runtime host for agent_loop + deterministic paths. A `None` host is only
    /// valid for callers that never dispatch a host-backed activity; host-backed
    /// specs return `DispatchError::HostRequired` when it is absent.
    pub host: Option<&'a dyn RuntimeHost>,
}

/// Outcome of a v2 dispatch attempt.
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

    /// [ORB-10385] A resolved catalog activity names a deterministic action
    /// the executing runtime does not implement — the job/activity assets and
    /// the installed binary are out of sync. Raised by
    /// [`crate::validate_job_deterministic_actions`] before any step runs, so
    /// the run never admits a task or creates a worktree it cannot finish.
    #[error(
        "activity `{activity}` references deterministic action `{action}`, which is not registered in the executing runtime — the loaded catalog asset and the installed orbit binary are out of sync; reinstall or rebuild orbit, or remove the activity from the job"
    )]
    DeterministicActionUnavailable { activity: String, action: String },

    #[error("deterministic action `{action}` failed: {message}")]
    DeterministicActionFailed { action: String, message: String },

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
    /// Treated as transient: the step retry wrapper may re-attempt it.
    #[error("cli invocation failed: {0}")]
    CliInvocationFailed(String),

    /// CLI subprocess invocation failed in a way retrying cannot fix —
    /// agent config rejected, executable missing, or permission denied
    /// (ORB-10006). Non-retryable: the step retry wrapper fails fast
    /// instead of burning attempts on a deterministic failure.
    #[error("cli invocation failed (permanent): {0}")]
    CliInvocationPermanent(String),

    /// A linked-worktree provider invocation changed the registered primary
    /// checkout. Ordinary retries must not compound or misattribute the delta.
    /// An explicitly configured recovery activity may inspect the diagnostic
    /// once before the executor's single post-recovery attempt (ORB-10306).
    #[error("worktree integrity violation `{code}`: {diagnostic}")]
    WorktreeIntegrity {
        code: &'static str,
        diagnostic: String,
    },

    /// Tool-allowlist denial (§6). Non-retryable — the retry wrapper must not
    /// re-attempt a denied call. Phase 2 formerly translated this to
    /// `Ok(terminated)`; Phase 3 surfaces it structurally so the DAG executor
    /// can classify it.
    #[error("tool `{tool_name}` denied at iteration {iteration}")]
    ToolDenied { tool_name: String, iteration: u32 },

    /// Job validation rejected the spec at load time.
    #[error("job validation failed: {0}")]
    JobValidation(String),

    /// A step's `retry:` block violates a config invariant (ORB-10006).
    /// Caught by `validate_job` before any step executes; the message names
    /// the offending values.
    #[error("step `{step_id}`: invalid retry config: {field} = {value} violates `{invariant}`")]
    RetryConfigInvalid {
        step_id: String,
        field: &'static str,
        value: u64,
        invariant: String,
    },

    /// Generic job-executor error — distinct from per-activity failures.
    #[error("job executor: {0}")]
    JobExecution(String),

    #[error("audit write failed: {0}")]
    AuditFailed(String),
}

impl DispatchError {
    /// Whether this error should bypass the retry wrapper. Tool denials,
    /// unknown deterministic actions, validation errors, and permanent CLI
    /// invocation failures are non-retryable (§4.3: "Non-retryable errors —
    /// schema violations, allowlist denials, cancellation — skip retry").
    pub fn is_non_retryable(&self) -> bool {
        matches!(
            self,
            DispatchError::ToolDenied { .. }
                | DispatchError::DeterministicActionNotRegistered(_)
                | DispatchError::DeterministicActionUnavailable { .. }
                | DispatchError::JobValidation(_)
                | DispatchError::RetryConfigInvalid { .. }
                | DispatchError::HostRequired(_)
                | DispatchError::UnwiredHttpTransport { .. }
                | DispatchError::UnresolvedAutoBackend { .. }
                | DispatchError::CliInvocationPermanent(_)
                | DispatchError::WorktreeIntegrity { .. }
        )
    }

    /// Whether an error that bypasses normal retry may still reach an
    /// explicitly configured recovery activity.
    ///
    /// Worktree integrity failures carry the structured checkout diagnostic a
    /// recovery agent needs to establish whether reconciliation is safe. All
    /// other non-retryable classes retain their fail-fast behavior.
    pub fn allows_recovery(&self) -> bool {
        matches!(self, DispatchError::WorktreeIntegrity { .. })
    }
}

/// Translate a [`DispatchError`] into the workspace-public [`OrbitError`]
/// surface at crate boundaries.
///
/// Validation failures keep their dedicated [`OrbitError::JobValidation`]
/// variant — including [`DispatchError::DeterministicActionUnavailable`],
/// which is raised by the same pre-execution validation pass [ORB-10385].
/// Everything else collapses into [`OrbitError::InvalidInput`] with the
/// dispatch error's rendered message. Callers translate with
/// `.map_err(dispatch_error_to_orbit)?` per
/// `docs/design-patterns/error_translation.md` [ORB-10013].
pub fn dispatch_error_to_orbit(error: DispatchError) -> OrbitError {
    match error {
        DispatchError::JobValidation(message) => OrbitError::JobValidation(message),
        unavailable @ DispatchError::DeterministicActionUnavailable { .. } => {
            OrbitError::JobValidation(unavailable.to_string())
        }
        other => OrbitError::InvalidInput(format!("{other}")),
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
    let spec = input.spec;
    let activity_type = match spec {
        ActivityV2Spec::AgentLoop(_) => "agent_loop",
        ActivityV2Spec::Deterministic(_) => "deterministic",
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
    input.audit.push_parent_lossy(activity_event_id);

    let result = match spec {
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
    };

    input.audit.pop_parent_lossy();
    let outcome_str = match &result {
        Ok(o) if o.success => "success",
        Ok(_) => "failed",
        Err(_) => "error",
    };
    input.audit.emit_lossy(
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
    host: &dyn RuntimeHost,
    run_id: &str,
    spec: &DeterministicSpec,
    fs_profile: Option<&str>,
    audit: Arc<V2AuditWriter>,
    input: &Value,
) -> Result<DispatchOutcome, DispatchError> {
    let mut tool_context = host.tool_context_for_activity(
        Some(run_id),
        fs_profile,
        Some(v2_fs_audit_logger(audit.clone())),
        None,
    );
    tool_context
        .session_context
        .effective_capabilities
        .insert(McpCapability::Runner);
    let output = match DeterministicAction::parse(&spec.action) {
        Some(DeterministicAction::Engine(action)) => {
            let state_context = crate::executor::automation::StateExecutionContext {
                run_id: input
                    .get("run_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
                ..crate::executor::automation::StateExecutionContext::default()
            };
            crate::executor::automation::execute_engine_action(
                host,
                action,
                input,
                Some(&state_context),
            )
            .map_err(|error| DispatchError::DeterministicActionFailed {
                action: spec.action.clone(),
                message: error.to_string(),
            })?
        }
        Some(DeterministicAction::Core(_)) | None => {
            host.run_deterministic(&spec.action, &spec.config, input, tool_context)?
        }
    };
    Ok(DispatchOutcome {
        success: true,
        output,
        message: None,
        invocation: None,
    })
}

fn run_agent_loop_activity(
    host: &dyn RuntimeHost,
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
        Backend::Cli => run_cli_backend(host, spec, run_id, audit, input, fs_profile)
            .map(|outcome| label_failure_with_step(activity_name, outcome)),
    }
}

/// [ORB-10449] Prefix a failing CLI agent-loop message with the step that
/// produced it.
///
/// A run surfaces only its terminal message, and the executor's fallback
/// (`step `<id>` completed with success=false`) is used only when the step
/// reports no message at all. So a step that *does* report one was previously
/// anonymous in the run record — an operator saw the symptom without the
/// origin. Naming the step here keeps that fix in one place for every CLI
/// agent-loop failure mode (timeout, nonzero exit, protocol violation,
/// invalid envelope).
fn label_failure_with_step(activity_name: &str, mut outcome: DispatchOutcome) -> DispatchOutcome {
    if !outcome.success
        && let Some(message) = outcome.message.take()
    {
        outcome.message = Some(format!("step `{activity_name}`: {message}"));
    }
    outcome
}

fn run_agent_loop_via_driver(
    host: &dyn RuntimeHost,
    spec: &AgentLoopSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    input: &Value,
    fs_profile: Option<&str>,
) -> Result<DispatchOutcome, DispatchError> {
    // Sourcing only: orbit-core pulls the provider credential from wherever
    // makes sense (env var, config, secrets manager). We treat a sourcing
    // failure as `None` so a `replay`-enabled `drive_agent_loop` can still
    // honor ORBIT_V2_REPLAY without credentials. Default builds ignore replay
    // variables; when the driver needs a key and none is present, it errors
    // structurally.
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
            "cache_read_input_tokens": outcome.usage.cache_read_input_tokens,
            "cache_creation_input_tokens": outcome.usage.cache_creation_input_tokens,
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
            // OpenAI-compatible and generic agent-loop usage reports a single
            // cache-creation counter. The 1h/5m TTL split isn't surfaced here,
            // so all reported writes retain the standard (5m) rate.
            cache_create_1h: 0,
            output: outcome.usage.output_tokens,
        },
        tool_calls,
        duration_ms,
        provider_model: None,
        provider_cost_usd: None,
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
