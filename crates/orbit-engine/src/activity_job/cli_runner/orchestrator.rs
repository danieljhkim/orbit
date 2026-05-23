//! Orchestration for `backend: cli` agent subprocess dispatch.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use orbit_agent::{Agent, AgentConfig, AgentOperation, AgentRequest, peek_response_status};
use orbit_common::types::activity_job::{AgentLoopSpec, V2AuditEventKind};
use orbit_common::types::{LearningInjectionCaps, LearningInjectionState, prepend_reminder_block};
use orbit_common::utility::learning_session::{
    learning_session_state_path, write_learning_session_state,
};
use orbit_common::utility::redaction::{PatternRedactor, redact_sensitive_env_text};
use serde_json::Value;

use super::super::audit_writer::V2AuditWriter;
use super::super::dispatcher::{
    DispatchError, DispatchInvocationTrace, DispatchOutcome, V2RuntimeHost,
};
use super::super::workspace::resolve_subprocess_cwd;
use super::argv::{
    apply_provider_static_arg_fixups, audit_argv_for_dispatch, neutralize_inner_sandbox,
};
use super::envelope::{cli_agent_envelope_json, parse_cli_invocation_trace, task_id_from_input};
use super::supervisor::{
    DEFAULT_WALL_CLOCK_TIMEOUT_SECONDS, SpawnTraceContext, SpawnWithTimeoutRequest,
    spawn_with_timeout,
};

const STDOUT_TEXT_PREVIEW_LIMIT_BYTES: usize = 64 * 1024;

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
    let _ = audit.emit(V2AuditEventKind::ToolAllowlistHarnessDelegated {
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

    let config = AgentConfig::from_cli_config(
        cli_executor.command.clone(),
        spec.model.as_deref(),
        &provider_config,
    )
    .map_err(|err| DispatchError::CliInvocationFailed(format!("agent config: {err}")))?;
    let agent = Agent::new(&config)
        .map_err(|err| DispatchError::CliInvocationFailed(format!("agent build: {err}")))?;

    let agent_req = AgentRequest {
        operation: AgentOperation::Activity {
            activity_id: "v2_cli_backend".to_string(),
        },
        envelope_json,
        verbose: false,
    };

    let (invocation, _trace) = agent
        .invoke(agent_req)
        .map_err(|err| DispatchError::CliInvocationFailed(format!("agent invoke: {err}")))?;
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

    let model_redacted = agent.model_name().map(|m| redaction.apply_str(m));
    let _ = audit.emit(V2AuditEventKind::CliInvocationStarted {
        provider: provider.clone(),
        argv_redacted: argv_redacted.clone(),
        stdin_blob_ref: Some(stdin_blob_ref.clone()),
        model: model_redacted,
        cwd: subprocess_cwd_string.clone(),
        wall_clock_timeout_ms: wall_clock_timeout.as_millis() as u64,
    });

    let mut child_env = vec![
        ("ORBIT_RUN_ID".to_string(), run_id.to_string()),
        ("ORBIT_MANAGED_RUN_CONTEXT".to_string(), "1".to_string()),
    ];
    if let Some(agent_name) = tool_ctx.agent_name.as_deref() {
        child_env.push(("ORBIT_AGENT_NAME".to_string(), agent_name.to_string()));
    }
    if let Some(model_name) = tool_ctx.model_name.as_deref() {
        child_env.push(("ORBIT_AGENT_MODEL".to_string(), model_name.to_string()));
    }
    if let Some(session_id) = learning_context.session_id {
        child_env.push(("ORBIT_SESSION_ID".to_string(), session_id));
    }
    let (stdout, stderr, exit_code, duration, timed_out) =
        spawn_with_timeout(SpawnWithTimeoutRequest {
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
        })
        .map_err(|err| DispatchError::CliInvocationFailed(err.to_string()))?;

    let stdout_blob_ref = audit.write_blob(&stdout);
    let stderr_blob_ref = audit.write_blob(&stderr);

    let _ = audit.emit(V2AuditEventKind::CliInvocationFinished {
        provider: provider.clone(),
        exit_code,
        duration_ms: duration.as_millis() as u64,
        stdout_blob_ref: Some(stdout_blob_ref.clone()),
        stderr_blob_ref: Some(stderr_blob_ref.clone()),
        harness_version: None,
        timed_out,
    });

    // Provisional success based on exit code; the embedded envelope status
    // (read below) can demote this to false. Some provider CLIs (notably
    // claude) can exit 0 with an outer `result.subtype = "success"` envelope
    // even when their inner Orbit response payload reports `status = "failed"`,
    // and pre-T20260508-17 the dispatcher recorded that as success. The
    // structured envelope is now authoritative when present.
    let exit_success = !timed_out && matches!(exit_code, Some(0));
    let stdout_text = String::from_utf8_lossy(&stdout).into_owned();
    let envelope_status = peek_response_status(&stdout_text);
    let stdout_preview = stdout_text_preview(&stdout_text, &redaction);
    let envelope_indicates_failure =
        matches!(envelope_status.as_deref(), Some("failed") | Some("timeout"));
    let success = exit_success && !envelope_indicates_failure;
    let trace = parse_cli_invocation_trace(
        &stdout,
        &stderr,
        exit_code,
        duration.as_millis() as u64,
        success,
    );
    let message = if timed_out {
        Some(format!(
            "cli subprocess exceeded {}s wall-clock timeout",
            timeout_seconds
        ))
    } else if exit_success && envelope_indicates_failure {
        Some(format!(
            "cli subprocess reported envelope status={:?} despite exit 0",
            envelope_status.as_deref().unwrap_or("unknown")
        ))
    } else if !success {
        Some(format!("cli subprocess exited with code {:?}", exit_code))
    } else {
        None
    };

    Ok(DispatchOutcome {
        success,
        output: serde_json::json!({
            "provider": provider,
            "argv_redacted": argv_redacted,
            "stdin_blob_ref": stdin_blob_ref,
            "stdout_blob_ref": stdout_blob_ref,
            "stderr_blob_ref": stderr_blob_ref,
            "exit_code": exit_code,
            "duration_ms": duration.as_millis() as u64,
            "timed_out": timed_out,
            "stdout_text": stdout_preview.text,
            "stdout_text_truncated": stdout_preview.truncated,
            "stdout_text_original_bytes": stdout.len(),
            "stdout_text_preview_bytes": stdout_preview.preview_bytes,
            "stdout_text_preview_limit_bytes": STDOUT_TEXT_PREVIEW_LIMIT_BYTES,
        }),
        message,
        invocation: trace.map(|trace| DispatchInvocationTrace {
            provider,
            model,
            trace,
        }),
    })
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
    if let Some(root) = workspace_root {
        let path = learning_session_state_path(root, &session_id);
        write_learning_session_state(&path, &state).map_err(|err| {
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

fn stdout_text_preview(raw: &str, redactor: &PatternRedactor) -> StdoutTextPreview {
    let redacted = redactor.apply_str(&redact_sensitive_env_text(raw));
    let truncated = redacted.len() > STDOUT_TEXT_PREVIEW_LIMIT_BYTES;
    let text = if truncated {
        let boundary = redacted
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|idx| *idx <= STDOUT_TEXT_PREVIEW_LIMIT_BYTES)
            .last()
            .unwrap_or(0);
        redacted[..boundary].to_string()
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
