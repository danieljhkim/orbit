//! Orchestration for `backend: cli` agent subprocess dispatch.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use orbit_agent::{
    Agent, AgentConfig, AgentOperation, AgentRequest, peek_response_status,
    response_envelope_protocol_check,
};
use orbit_common::types::activity_job::{AgentLoopSpec, V2AuditEventKind};
use orbit_common::types::{LearningInjectionCaps, LearningInjectionState, prepend_reminder_block};
use orbit_common::utility::redaction::{PatternRedactor, redact_sensitive_env_text};
use serde_json::Value;

use crate::context::{ProvenanceEnv, provenance_env};

use super::super::audit_writer::V2AuditWriter;
use super::super::dispatcher::{
    DispatchError, DispatchInvocationTrace, DispatchOutcome, V2RuntimeHost,
};
use super::super::workspace::{
    WorktreeBoundaryGuard, resolve_subprocess_cwd, validate_declared_worktree_pair,
};
use super::argv::{
    apply_provider_static_arg_fixups, audit_argv_for_dispatch, neutralize_inner_sandbox,
};
use super::envelope::{
    cli_agent_envelope_json, parse_cli_invocation_trace, parse_cli_response_result,
    task_id_from_input,
};
use super::supervisor::{
    DEFAULT_WALL_CLOCK_TIMEOUT_SECONDS, SpawnTraceContext, SpawnWithTimeoutRequest,
    spawn_with_timeout,
};

const STDOUT_TEXT_PREVIEW_LIMIT_BYTES: usize = 64 * 1024;
const RESPONSE_DIAGNOSTIC_LIMIT_CHARS: usize = 1024;

pub fn run_cli_backend(
    host: &dyn V2RuntimeHost,
    spec: &AgentLoopSpec,
    run_id: &str,
    audit: Arc<V2AuditWriter>,
    input: &Value,
    fs_profile: Option<&str>,
) -> Result<DispatchOutcome, DispatchError> {
    let provider = spec.provider.as_str().to_string();
    let mut cli_executor = host.resolve_cli_executor(&provider)?;
    let timeout_seconds = if spec.wall_clock_timeout_seconds == 0 {
        DEFAULT_WALL_CLOCK_TIMEOUT_SECONDS
    } else {
        spec.wall_clock_timeout_seconds
    };
    let wall_clock_timeout = Duration::from_secs(timeout_seconds);

    // §6 allowlist-advisory event — emitted once per invocation before the
    // subprocess starts so a reviewer can see the enforcement gap at a glance.
    audit.emit_lossy(V2AuditEventKind::ToolAllowlistHarnessDelegated {
        provider: provider.clone(),
        tools: spec.tools.clone(),
    });

    let task_ctx = host.task_context_for_agent_input(input)?;
    let mut tool_ctx = host.tool_context_for_activity(
        Some(run_id),
        fs_profile,
        None,
        spec.proc_allowed_programs.as_deref(),
    );
    tool_ctx.agent_name = Some(provider.clone());
    tool_ctx.model_name = spec.model.as_deref().map(str::to_string);
    // A shipment pipeline renders the assigned checkout twice: once as the
    // child cwd and once inside the agent contract. Validate that pair against
    // the registered primary before sandbox construction or provider spawn.
    let declared_worktree_pair = validate_declared_worktree_pair(
        input,
        task_ctx.as_ref(),
        run_id,
        &provider,
        tool_ctx.workspace_root.as_deref(),
    )?;
    // Resolve the subprocess cwd before sandbox compilation so the host can
    // re-allow the active worktree subpath after the policy deny rules. The
    // sandbox's `denyModify .orbit/**` rule otherwise blocks every non-codex
    // provider from writing inside its own jrun worktree. See T20260508-17.
    let subprocess_cwd =
        resolve_subprocess_cwd(input, task_ctx.as_ref(), tool_ctx.workspace_root.as_deref())?;
    let subprocess_cwd_string = subprocess_cwd
        .as_ref()
        .map(|path| path.display().to_string());
    let sandbox =
        host.resolve_executor_sandbox(&provider, fs_profile, subprocess_cwd.as_deref())?;

    let learning_context = cli_learning_context(host, input, tool_ctx.workspace_root.as_deref())?;
    let envelope_json = cli_agent_envelope_json(
        spec,
        run_id,
        input,
        task_ctx.as_ref(),
        learning_context.prompt.as_deref(),
    )?;

    let mut provider_config = host.provider_cli_config(&provider);

    // Provider-specific static-arg fixups that are independent of whether the
    // outer sandbox is active. Today this only rewrites Claude's `--debug-file`
    // value to an absolute path under the writable claude state dir, so the
    // log lands somewhere `denyModify: .orbit/**` does not block. See
    // T20260505-22.
    apply_provider_static_arg_fixups(&provider, &mut cli_executor.args);

    // Inner-sandbox neutralization. When orbit-exec wraps the CLI we are the
    // single source of truth for filesystem enforcement; the agent's own
    // sandbox flag would either double-encode the same constraint or
    // contradict it. We neutralize per-provider rather than layering:
    //   - codex: pin `--sandbox danger-full-access` so codex behaves
    //     transparently inside our outer sandbox.
    //   - gemini: drop `-s` / `--sandbox` from the executor's static args.
    //   - claude: nothing to do; claude has no OS-level sandbox flag.
    if sandbox.is_some() {
        neutralize_inner_sandbox(&provider, &mut provider_config, &mut cli_executor.args);
    }

    // Config parse / agent construction failures are deterministic — the
    // same spec fails identically on every attempt, so they are classified
    // permanent and skip the step retry wrapper (ORB-10006).
    let config = AgentConfig::from_cli_config(
        cli_executor.command.clone(),
        spec.model.as_deref(),
        &provider_config,
    )
    .map_err(|err| DispatchError::CliInvocationPermanent(format!("agent config: {err}")))?;
    let agent = Agent::new(&config)
        .map_err(|err| DispatchError::CliInvocationPermanent(format!("agent build: {err}")))?;

    let agent_req = AgentRequest {
        operation: AgentOperation::Activity {
            activity_id: "v2_cli_backend".to_string(),
        },
        envelope_json,
        verbose: false,
    };

    // `invoke` only renders the argv/stdin for the subprocess (nothing has
    // executed yet) — failures here are deterministic request-shaping errors.
    let (invocation, _trace) = agent
        .invoke(agent_req)
        .map_err(|err| DispatchError::CliInvocationPermanent(format!("agent invoke: {err}")))?;
    let model = agent.model_name().map(str::to_string);

    let mut subprocess_args = Vec::with_capacity(cli_executor.args.len() + invocation.args.len());
    subprocess_args.extend(cli_executor.args.iter().cloned());
    subprocess_args.extend(invocation.args.iter().cloned());

    // The audit argv reflects what actually runs. Under sandbox-exec the
    // parent is `<trusted sandbox-exec> -f <profile.sb> <program> <args...>`;
    // under bare exec it's `<program> <args...>`. The redactor still scrubs
    // the child's program name + args so secrets in argv stay redacted.
    let redaction = PatternRedactor::with_argv_secrets();
    let audit_argv =
        audit_argv_for_dispatch(&invocation.program, &subprocess_args, sandbox.as_ref());
    let argv_redacted: Vec<String> = audit_argv.iter().map(|a| redaction.apply_str(a)).collect();

    let stdin_blob_ref = audit.write_blob(&invocation.stdin);

    // L-0095: Provider cwd is advisory; enforce the linked-worktree postcondition.
    // Snapshot both sides of a linked-worktree invocation immediately before
    // provider spawn. `tool_ctx.workspace_root` is the registered primary
    // checkout; `subprocess_cwd` is the canonical assigned worktree. Direct
    // invocations where those resolve to the same checkout remain unchanged.
    let mut worktree_boundary = WorktreeBoundaryGuard::capture(
        input,
        task_ctx.as_ref(),
        run_id,
        &provider,
        subprocess_cwd.as_deref(),
        tool_ctx.workspace_root.as_deref(),
        declared_worktree_pair.as_ref(),
    )?;

    let model_redacted = agent.model_name().map(|m| redaction.apply_str(m));
    audit.emit_lossy(V2AuditEventKind::CliInvocationStarted {
        provider: provider.clone(),
        argv_redacted: argv_redacted.clone(),
        stdin_blob_ref: Some(stdin_blob_ref.clone()),
        model: model_redacted,
        cwd: subprocess_cwd_string.clone(),
        wall_clock_timeout_ms: wall_clock_timeout.as_millis() as u64,
    });

    let task_id = task_id_from_input(input);
    // ADR-0182: external CLI agents get the same active-task hook binding as
    // direct-agent executions. The AGENT_* fields preserve ORB-10342's
    // commit-telemetry contract and omit unknown model/task values.
    let child_env = provenance_env(ProvenanceEnv {
        orbit_run_id: Some(run_id),
        orbit_managed_run_context: true,
        orbit_agent_name: tool_ctx.agent_name.as_deref(),
        orbit_agent_model: tool_ctx.model_name.as_deref(),
        orbit_session_id: learning_context.session_id.as_deref(),
        orbit_task_id: task_id,
        orbit_active_task: true,
        agent_run_id: Some(run_id),
        agent_model: model.as_deref(),
        agent_task_id: task_id,
    });
    let spawn_result = spawn_with_timeout(SpawnWithTimeoutRequest {
        program: &invocation.program,
        args: &subprocess_args,
        stdin_bytes: &invocation.stdin,
        env: &child_env,
        cwd: subprocess_cwd.as_deref(),
        timeout: wall_clock_timeout,
        sandbox: sandbox.as_ref(),
        trace: SpawnTraceContext {
            provider: &provider,
            job_run_id: run_id,
            task_id: task_id_from_input(input),
            cwd: subprocess_cwd_string.as_deref(),
        },
        output_capture_limit: None,
    });

    let (stdout, stderr, exit_code, duration, timed_out) = match spawn_result {
        Ok(result) => result,
        Err(err) => {
            if let Some(boundary) = worktree_boundary.take() {
                boundary.verify()?;
            }
            // Spawn-layer classification (ORB-10006): executable missing /
            // permission denied fail fast; resource exhaustion (EAGAIN,
            // ENOMEM, ...) and other transient host failures stay retryable
            // at the step layer.
            return Err(if err.permanent {
                DispatchError::CliInvocationPermanent(err.message)
            } else {
                DispatchError::CliInvocationFailed(err.message)
            });
        }
    };

    let stdout_blob_ref = audit.write_blob(stdout.bytes());
    let stderr_blob_ref = audit.write_blob(stderr.bytes());

    audit.emit_lossy(V2AuditEventKind::CliInvocationFinished {
        provider: provider.clone(),
        exit_code,
        duration_ms: duration.as_millis() as u64,
        stdout_blob_ref: Some(stdout_blob_ref.clone()),
        stderr_blob_ref: Some(stderr_blob_ref.clone()),
        harness_version: None,
        timed_out,
    });

    // Verify the write boundary after recording the terminal provider event
    // but before its success/failure classification can reach the DAG. The
    // integrity error deliberately takes precedence over exit zero, nonzero,
    // and timeout outcomes.
    if let Some(boundary) = worktree_boundary {
        boundary.verify()?;
    }

    // Provider output is not the system of record for artifact-backed
    // activities: task state, review threads, git state, and deterministic
    // downstream gates are. Parse response envelopes to project useful fields
    // and diagnostics, but only make them authoritative when the activity
    // explicitly declares that downstream templates require them.
    let exit_success = !timed_out && matches!(exit_code, Some(0));
    // A truncated capture retains the final complete JSONL events separately
    // from its diagnostic prefix. Protocol parsing must use that tail so a
    // verbose provider's final Orbit envelope remains authoritative.
    let stdout_text = String::from_utf8_lossy(stdout.protocol_bytes());
    let envelope_status = peek_response_status(stdout_text.as_ref());
    let stdout_preview = stdout_text_preview(stdout_text.as_ref(), &redaction, stdout.truncated());
    let parsed_result = exit_success.then(|| {
        parse_cli_response_result(
            stdout.protocol_bytes(),
            stderr.protocol_bytes(),
            exit_code,
            duration.as_millis() as u64,
            true,
        )
    });
    let response_envelope_valid = matches!(parsed_result.as_ref(), Some(Ok(_)));
    let response_envelope_error = parsed_result
        .as_ref()
        .and_then(|result| result.as_ref().err())
        .map(|error| response_diagnostic(error, &redaction));
    // [ORB-10449]: the step-completion protocol check. Content-blind
    // by construction — `response_envelope_protocol_check` reads the envelope
    // frame and never `result`/`error`, so this asks only "did the invocation
    // run its contract to the end", never "do we believe what it said". An
    // agent that declares `status: "failed"` passes; one that yielded mid-turn
    // emitted no envelope and does not.
    //
    // Only meaningful on an otherwise-clean exit: a timeout or nonzero exit
    // already fails the step with a more specific message.
    let completion_envelope_error = exit_success
        .then(|| response_envelope_protocol_check(stdout_text.as_ref()))
        .and_then(Result::err)
        .map(|error| completion_diagnostic(&error.to_string(), &redaction));
    let completion_protocol_violation =
        spec.require_completion_envelope && completion_envelope_error.is_some();
    // Two orthogonal contracts. `require_completion_envelope` gates step
    // completion (above); `require_response_envelope` additionally gates the
    // envelope's *content* for activities whose downstream templates consume it
    // (ADR-0224 / L-0087) — outside that opt-in, parsing stays advisory.
    let success = exit_success
        && !completion_protocol_violation
        && (!spec.require_response_envelope || response_envelope_valid);
    let trace = parse_cli_invocation_trace(
        stdout.protocol_bytes(),
        stderr.protocol_bytes(),
        exit_code,
        duration.as_millis() as u64,
        success,
    );
    let message = if timed_out {
        Some(format!(
            "cli subprocess exceeded {}s wall-clock timeout",
            timeout_seconds
        ))
    } else if !exit_success {
        Some(format!("cli subprocess exited with code {:?}", exit_code))
    } else if spec.require_response_envelope
        && matches!(envelope_status.as_deref(), Some("failed") | Some("timeout"))
    {
        Some(format!(
            "cli subprocess reported envelope status={:?} despite exit 0",
            envelope_status.as_deref().unwrap_or("unknown")
        ))
    } else if spec.require_response_envelope {
        response_envelope_error.clone()
    } else if completion_protocol_violation {
        // Ordered last on purpose: an activity that opted into the content
        // contract already produced a strictly more specific diagnostic above,
        // and the two conditions largely overlap. This branch is what the
        // remaining activities — the ones that only ever had the advisory
        // parse — now report instead of silently checkpointing success.
        completion_envelope_error.clone()
    } else {
        None
    };

    let StdoutTextPreview {
        text: stdout_text,
        truncated: stdout_text_truncated,
        preview_bytes: stdout_text_preview_bytes,
    } = stdout_preview;
    let mut output = parsed_result.and_then(Result::ok).unwrap_or_default();
    for (key, value) in [
        ("provider", Value::String(provider.clone())),
        ("argv_redacted", serde_json::json!(argv_redacted)),
        ("stdin_blob_ref", Value::String(stdin_blob_ref.clone())),
        ("stdout_blob_ref", Value::String(stdout_blob_ref.clone())),
        ("stderr_blob_ref", Value::String(stderr_blob_ref.clone())),
        ("exit_code", serde_json::json!(exit_code)),
        (
            "duration_ms",
            serde_json::json!(duration.as_millis() as u64),
        ),
        ("timed_out", Value::Bool(timed_out)),
        (
            "response_envelope_required",
            Value::Bool(spec.require_response_envelope),
        ),
        (
            "response_envelope_valid",
            Value::Bool(response_envelope_valid),
        ),
        (
            "response_envelope_status",
            envelope_status.map_or(Value::Null, Value::String),
        ),
        (
            "response_envelope_error",
            response_envelope_error.map_or(Value::Null, Value::String),
        ),
        (
            "completion_envelope_required",
            Value::Bool(spec.require_completion_envelope),
        ),
        (
            "completion_envelope_satisfied",
            Value::Bool(!exit_success || completion_envelope_error.is_none()),
        ),
        (
            "completion_envelope_error",
            completion_envelope_error.map_or(Value::Null, Value::String),
        ),
        ("stdout_text", Value::String(stdout_text)),
        (
            "stdout_text_truncated",
            Value::Bool(stdout.truncated() || stdout_text_truncated),
        ),
        (
            "stdout_text_original_bytes",
            serde_json::json!(stdout.observed_bytes()),
        ),
        (
            "stdout_text_preview_bytes",
            serde_json::json!(stdout_text_preview_bytes),
        ),
        (
            "stdout_text_preview_limit_bytes",
            serde_json::json!(STDOUT_TEXT_PREVIEW_LIMIT_BYTES),
        ),
        (
            "stdout_text_captured_bytes",
            serde_json::json!(stdout.bytes().len()),
        ),
        ("stdout_capture_truncated", Value::Bool(stdout.truncated())),
        (
            "stdout_capture_limit_bytes",
            serde_json::json!(stdout.capture_limit_bytes()),
        ),
        (
            "stderr_original_bytes",
            serde_json::json!(stderr.observed_bytes()),
        ),
        (
            "stderr_captured_bytes",
            serde_json::json!(stderr.bytes().len()),
        ),
        ("stderr_capture_truncated", Value::Bool(stderr.truncated())),
        (
            "stderr_capture_limit_bytes",
            serde_json::json!(stderr.capture_limit_bytes()),
        ),
    ] {
        output.entry(key.to_string()).or_insert(value);
    }

    Ok(DispatchOutcome {
        success,
        output: Value::Object(output),
        message,
        invocation: trace.map(|trace| DispatchInvocationTrace {
            provider,
            model,
            trace,
        }),
    })
}

fn response_diagnostic(error: &str, redactor: &PatternRedactor) -> String {
    format!(
        "cli response envelope invalid: {}",
        bounded_diagnostic(error, redactor)
    )
}

/// [ORB-10449] Name the protocol violation for what it is. The old surfaced
/// failure was whatever deterministic gate tripped several steps later, which
/// reads as a downstream defect; this says the agent stopped before finishing
/// its turn and points at the evidence.
fn completion_diagnostic(error: &str, redactor: &PatternRedactor) -> String {
    format!(
        "agent step did not complete: the provider exited 0 but stdout carried no valid \
         terminating Orbit response envelope ({}). The invocation ended without finishing its \
         contract — typically an agent that yielded mid-work — so this step's work is incomplete \
         and only what it persisted before stopping is durable.",
        bounded_diagnostic(error, redactor)
    )
}

fn bounded_diagnostic(error: &str, redactor: &PatternRedactor) -> String {
    let redacted = redactor.apply_str(&redact_sensitive_env_text(error));
    let bounded: String = redacted
        .chars()
        .take(RESPONSE_DIAGNOSTIC_LIMIT_CHARS)
        .collect();
    let suffix = if bounded.len() < redacted.len() {
        "…"
    } else {
        ""
    };
    format!("{bounded}{suffix}")
}

struct CliLearningContext {
    prompt: Option<String>,
    session_id: Option<String>,
}

fn cli_learning_context(
    host: &dyn V2RuntimeHost,
    input: &Value,
    workspace_root: Option<&std::path::Path>,
) -> Result<CliLearningContext, DispatchError> {
    let caps = LearningInjectionCaps::from_env();
    let reminders = host.learning_reminders_for_task(input, caps)?;
    if reminders.is_empty() {
        return Ok(CliLearningContext {
            prompt: None,
            session_id: None,
        });
    }

    let mut state = LearningInjectionState::new();
    let admitted = state.admit_reminders(&reminders, caps);
    if admitted.is_empty() {
        return Ok(CliLearningContext {
            prompt: None,
            session_id: None,
        });
    }
    let base_prompt = super::envelope::user_prompt_from_input(input)?;
    let prompt = prepend_reminder_block(&base_prompt, &admitted);
    let session_id = format!("S{:x}-cli", Utc::now().timestamp_micros());
    if workspace_root.is_some() {
        host.persist_session_learning_state(&session_id, &state)
            .map_err(|err| {
                DispatchError::CliInvocationFailed(format!("persist learning state: {err}"))
            })?;
    }
    Ok(CliLearningContext {
        prompt: Some(prompt),
        session_id: Some(session_id),
    })
}

struct StdoutTextPreview {
    text: String,
    truncated: bool,
    preview_bytes: usize,
}

fn stdout_text_preview(
    raw: &str,
    redactor: &PatternRedactor,
    prefer_tail: bool,
) -> StdoutTextPreview {
    let redacted = redactor.apply_str(&redact_sensitive_env_text(raw));
    let truncated = redacted.len() > STDOUT_TEXT_PREVIEW_LIMIT_BYTES;
    let text = if truncated {
        if prefer_tail {
            let requested_start = redacted.len() - STDOUT_TEXT_PREVIEW_LIMIT_BYTES;
            let boundary = redacted
                .char_indices()
                .map(|(idx, _)| idx)
                .find(|idx| *idx >= requested_start)
                .unwrap_or(redacted.len());
            let line_boundary = redacted[boundary..]
                .find('\n')
                .map_or(boundary, |idx| boundary + idx + 1);
            redacted[line_boundary..].to_string()
        } else {
            let boundary = redacted
                .char_indices()
                .map(|(idx, _)| idx)
                .take_while(|idx| *idx <= STDOUT_TEXT_PREVIEW_LIMIT_BYTES)
                .last()
                .unwrap_or(0);
            redacted[..boundary].to_string()
        }
    } else {
        redacted
    };
    let preview_bytes = text.len();

    StdoutTextPreview {
        text,
        truncated,
        preview_bytes,
    }
}
