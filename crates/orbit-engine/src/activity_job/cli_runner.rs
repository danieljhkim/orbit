//! `backend: cli` dispatch for v2 `agent_loop` activities (§3.1).
//!
//! Flow:
//!   1. Host resolves the provider name to a concrete CLI executor command and
//!      static args (env-overridable command for smokes).
//!   2. Build an `AgentRuntime` via `orbit-agent::Agent` — this retains the
//!      `*_cli.rs` runtimes unchanged (T20260419-0104 constraint). The
//!      `AgentInvocationSpec` it returns supplies provider runtime args and stdin.
//!   3. Emit §6 `tool_allowlist.harness_delegated` envelope event naming the
//!      declared allowlist + provider. Orbit does **not** enforce the
//!      allowlist — the harness owns enforcement.
//!   4. Emit §7.6 `cli.invocation.started` with redacted argv + stdin blob.
//!   5. Spawn the subprocess with a wall-clock timeout guard; capture
//!      stdout/stderr bytes and the exit status (or timeout flag).
//!   6. Emit §7.6 `cli.invocation.finished` with stdout/stderr blob refs,
//!      exit code, duration, and the timeout flag.
//!
//! Argv redaction is applied here via
//! `orbit_common::utility::redaction::PatternRedactor::with_argv_secrets()`, which
//! bundles the HTTP header/JSON patterns with a raw `sk-[A-Za-z0-9_\-]+`
//! pattern covering provider keys that may appear in argv when a user
//! mis-configures a provider.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use orbit_agent::{
    Agent, AgentConfig, AgentOperation, AgentRequest, parse_and_validate_response,
    peek_response_status,
};
use orbit_common::types::activity_job::{AgentLoopSpec, V2AuditEventKind};
use orbit_common::types::{ExecutionResult, ExecutorSandboxKind, InvocationTrace, OrbitError};
use orbit_common::utility::redaction::PatternRedactor;
use orbit_exec::{
    MacosSandboxSpawnRequest, claude_state_dir_from_env, compile_macos_sandbox_profile,
    sandbox_exec_available, spawn_under_macos_sandbox,
};
use serde_json::Value;
use tempfile::NamedTempFile;

use super::audit_writer::V2AuditWriter;
use super::dispatcher::{
    DispatchError, DispatchInvocationTrace, DispatchOutcome, ResolvedSandbox, V2RuntimeHost,
};
use super::workspace::resolve_subprocess_cwd;

/// Default wall-clock timeout when `AgentLoopSpec::wall_clock_timeout_seconds`
/// is zero. Matches §7.6 guidance: CLI subprocesses must have a mandatory
/// wall-clock guard.
const DEFAULT_WALL_CLOCK_TIMEOUT_SECONDS: u64 = 300;

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
    let tool_ctx = host.tool_context_for_activity(Some(run_id), fs_profile, None);
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

    let envelope_json = cli_agent_envelope_json(spec, run_id, input, task_ctx.as_ref())?;

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
    // parent is `sandbox-exec -f <profile.sb> <program> <args...>`; under
    // bare exec it's `<program> <args...>`. The redactor still scrubs the
    // child's program name + args so secrets in argv stay redacted.
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

    let child_env = vec![
        ("ORBIT_RUN_ID".to_string(), run_id.to_string()),
        ("ORBIT_MANAGED_RUN_CONTEXT".to_string(), "1".to_string()),
    ];
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
            "stdout_text": stdout_text,
        }),
        message,
        invocation: trace.map(|trace| DispatchInvocationTrace {
            provider,
            model,
            trace,
        }),
    })
}

fn cli_agent_envelope_json(
    spec: &AgentLoopSpec,
    run_id: &str,
    input: &Value,
    task_ctx: Option<&Value>,
) -> Result<Vec<u8>, DispatchError> {
    let mut envelope = serde_json::Map::new();
    envelope.insert("schemaVersion".to_string(), Value::from(1));
    envelope.insert(
        "instruction".to_string(),
        Value::String(spec.instruction.clone()),
    );
    envelope.insert(
        "prompt".to_string(),
        Value::String(user_prompt_from_input(input)?),
    );
    envelope.insert("input".to_string(), input.clone());
    envelope.insert("run_id".to_string(), Value::String(run_id.to_string()));
    envelope.insert(
        "tools".to_string(),
        serde_json::to_value(&spec.tools)
            .map_err(|err| DispatchError::CliInvocationFailed(format!("serialize tools: {err}")))?,
    );
    envelope.insert(
        "model".to_string(),
        serde_json::to_value(&spec.model)
            .map_err(|err| DispatchError::CliInvocationFailed(format!("serialize model: {err}")))?,
    );

    if let Some(task) = task_ctx {
        envelope.insert("task".to_string(), task.clone());
    }

    serde_json::to_vec(&Value::Object(envelope))
        .map_err(|err| DispatchError::CliInvocationFailed(format!("serialize envelope: {err}")))
}

fn parse_cli_invocation_trace(
    stdout: &[u8],
    stderr: &[u8],
    exit_code: Option<i32>,
    duration_ms: u64,
    success: bool,
) -> Option<InvocationTrace> {
    let exec_result = ExecutionResult {
        success,
        stdout: String::from_utf8_lossy(stdout).into_owned(),
        stderr: String::from_utf8_lossy(stderr).into_owned(),
        exit_code,
        duration_ms,
        output: None,
    };

    parse_and_validate_response(&exec_result)
        .map(|(_, _, trace)| trace)
        .ok()
}

fn user_prompt_from_input(input: &Value) -> Result<String, DispatchError> {
    match input {
        Value::Object(map) => match map.get("prompt") {
            Some(Value::String(text)) => Ok(text.clone()),
            Some(other) => serde_json::to_string(other).map_err(|err| {
                DispatchError::CliInvocationFailed(format!("serialize prompt: {err}"))
            }),
            None => serde_json::to_string(input).map_err(|err| {
                DispatchError::CliInvocationFailed(format!("serialize prompt: {err}"))
            }),
        },
        Value::String(text) => Ok(text.clone()),
        Value::Null => Ok(String::new()),
        other => serde_json::to_string(other)
            .map_err(|err| DispatchError::CliInvocationFailed(format!("serialize prompt: {err}"))),
    }
}

pub(super) fn task_id_from_input(input: &Value) -> Option<&str> {
    fn non_empty(value: &str) -> Option<&str> {
        if value.is_empty() { None } else { Some(value) }
    }

    input
        .get("task_id")
        .and_then(Value::as_str)
        .and_then(non_empty)
        .or_else(|| {
            input
                .get("task")
                .and_then(|task| task.get("id"))
                .and_then(Value::as_str)
                .and_then(non_empty)
        })
        .or_else(|| {
            input
                .get("task_ids")
                .and_then(Value::as_array)
                .and_then(|items| items.iter().find_map(Value::as_str))
                .and_then(non_empty)
        })
}

type SpawnOutput = (Vec<u8>, Vec<u8>, Option<i32>, Duration, bool);

/// Build the argv we audit-log. When wrapped, the parent process the kernel
/// sees is `sandbox-exec`, so we prepend `sandbox-exec -f <profile_path>` to
/// the child program. The profile path is the literal `<profile.sb>` because
/// the real path is a tempfile created at spawn time and only meaningful to
/// the kernel — the placeholder keeps the audit record stable across runs.
fn audit_argv_for_dispatch(
    program: &str,
    args: &[String],
    sandbox: Option<&ResolvedSandbox>,
) -> Vec<String> {
    match sandbox {
        Some(sb) if sb.kind == ExecutorSandboxKind::MacosSandboxExec => {
            let mut out = Vec::with_capacity(args.len() + 4);
            out.push("sandbox-exec".to_string());
            out.push("-f".to_string());
            out.push("<profile.sb>".to_string());
            out.push(program.to_string());
            out.extend(args.iter().cloned());
            out
        }
        _ => {
            let mut out = Vec::with_capacity(args.len() + 1);
            out.push(program.to_string());
            out.extend(args.iter().cloned());
            out
        }
    }
}

/// Pin codex's `--sandbox` to `danger-full-access` and drop gemini's `-s` /
/// `--sandbox` flag so the inner CLI sandbox does not double-encode the
/// outer orbit-exec sandbox. Claude has no native sandbox flag — nothing to
/// neutralize.
fn neutralize_inner_sandbox(
    provider: &str,
    provider_config: &mut HashMap<String, String>,
    static_args: &mut Vec<String>,
) {
    match provider {
        "codex" => {
            provider_config.insert("sandbox".to_string(), "danger-full-access".to_string());
        }
        "gemini" => {
            *static_args = filter_gemini_inner_sandbox_args(static_args);
        }
        _ => {}
    }
}

/// Sandbox-orthogonal arg fixups the dispatcher applies before spawn. Today
/// this only normalizes Claude's `--debug-file` path so the log lands at a
/// sandbox-allowed location regardless of how the executor YAML spelled it.
fn apply_provider_static_arg_fixups(provider: &str, static_args: &mut [String]) {
    if provider == "claude" {
        rewrite_claude_debug_file_path(static_args);
    }
}

/// Replace the value following any `--debug-file` token in `static_args`
/// with `<claude_state_dir>/<basename>`. Falls back to leaving the args
/// untouched when the state dir is unresolvable (e.g. `HOME` and
/// `CLAUDE_CONFIG_DIR` both unset) — the original relative path still
/// works in non-sandboxed runs, and the sandbox failure mode is what the
/// caller is opting into.
fn rewrite_claude_debug_file_path(static_args: &mut [String]) {
    let Some(state_dir) = claude_state_dir_from_env() else {
        return;
    };
    rewrite_debug_file_value(static_args, &state_dir);
}

fn rewrite_debug_file_value(static_args: &mut [String], state_dir: &std::path::Path) {
    let mut idx = 0;
    while idx + 1 < static_args.len() {
        if static_args[idx] == "--debug-file" {
            let basename = std::path::Path::new(&static_args[idx + 1])
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "claude-debug.log".to_string());
            static_args[idx + 1] = state_dir.join(basename).display().to_string();
            idx += 2;
        } else {
            idx += 1;
        }
    }
}

/// Strip gemini's sandbox flags from a static-args vector. `-s` and
/// `--sandbox` are toggle flags (no value); `--sandbox-image` would take a
/// value but gemini's sandbox-image is not currently used by orbit and is
/// out of scope.
fn filter_gemini_inner_sandbox_args(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|a| a.as_str() != "-s" && a.as_str() != "--sandbox")
        .cloned()
        .collect()
}

#[derive(Debug)]
struct SpawnedChild {
    child: Child,
    /// Sandbox profile tempfile, if any. Held until the supervisor returns
    /// so the kernel can keep reading the SBPL profile while the child runs.
    _profile_temp: Option<NamedTempFile>,
}

fn spawn_child_with_optional_sandbox(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&Path>,
    sandbox: Option<&ResolvedSandbox>,
) -> Result<SpawnedChild, OrbitError> {
    match sandbox {
        Some(sb) if sb.kind == ExecutorSandboxKind::MacosSandboxExec => {
            spawn_macos_sandboxed(program, args, env, cwd, sb)
        }
        Some(_) | None => spawn_bare(program, args, env, cwd),
    }
}

fn spawn_bare(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&Path>,
) -> Result<SpawnedChild, OrbitError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = cwd {
        command.current_dir(path);
    }
    let child = command
        .spawn()
        .map_err(|err| OrbitError::Execution(format!("failed to spawn `{program}`: {err}")))?;
    Ok(SpawnedChild {
        child,
        _profile_temp: None,
    })
}

fn spawn_macos_sandboxed(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&Path>,
    sandbox: &ResolvedSandbox,
) -> Result<SpawnedChild, OrbitError> {
    spawn_macos_sandboxed_with(program, args, env, cwd, sandbox, sandbox_exec_available())
}

/// Test-friendly variant of [`spawn_macos_sandboxed`]: callers pass an
/// explicit availability flag instead of probing `PATH`. Production routes
/// through the public wrapper which always reads the live `PATH`; tests
/// can assert the fail-closed and fallback branches without mutating
/// process-global state.
fn spawn_macos_sandboxed_with(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&Path>,
    sandbox: &ResolvedSandbox,
    sandbox_exec_present: bool,
) -> Result<SpawnedChild, OrbitError> {
    if !sandbox_exec_present {
        if sandbox.allow_fallback {
            tracing::warn!(
                target: "orbit.engine.cli_runner",
                program = program,
                "sandbox-exec not available on PATH; falling back to bare exec because executor declares allow_fallback"
            );
            return spawn_bare(program, args, env, cwd);
        }
        return Err(OrbitError::Execution(
            "sandbox-exec not available on PATH; declare allow_fallback: true to permit bare exec"
                .to_string(),
        ));
    }

    // SBPL compilation happens at spawn time so the orbit-exec dependency
    // stays scoped to this crate. The host returns only a descriptor
    // (`fs_profile` + `kind` + `allow_fallback`) so orbit-core has no
    // direct edge to orbit-exec.
    let profile_text = compile_macos_sandbox_profile(&sandbox.fs_profile)?;
    let (child, profile_temp) = spawn_under_macos_sandbox(MacosSandboxSpawnRequest {
        profile_text: &profile_text,
        program,
        args,
        env,
        cwd,
        stdin: Stdio::piped(),
        stdout: Stdio::piped(),
        stderr: Stdio::piped(),
    })?;
    Ok(SpawnedChild {
        child,
        _profile_temp: Some(profile_temp),
    })
}

struct SpawnTraceContext<'a> {
    provider: &'a str,
    job_run_id: &'a str,
    task_id: Option<&'a str>,
    cwd: Option<&'a str>,
}

struct SpawnWithTimeoutRequest<'a> {
    program: &'a str,
    args: &'a [String],
    stdin_bytes: &'a [u8],
    env: &'a [(String, String)],
    cwd: Option<&'a Path>,
    timeout: Duration,
    sandbox: Option<&'a ResolvedSandbox>,
    trace: SpawnTraceContext<'a>,
}

struct OutputReaderContext {
    provider: String,
    stream: &'static str,
    job_run_id: String,
    task_id: Option<String>,
    cwd: Option<String>,
    dispatch: tracing::Dispatch,
}

fn spawn_with_timeout(request: SpawnWithTimeoutRequest<'_>) -> Result<SpawnOutput, String> {
    let SpawnWithTimeoutRequest {
        program,
        args,
        stdin_bytes,
        env,
        cwd,
        timeout,
        sandbox,
        trace,
    } = request;

    let started = Instant::now();
    let SpawnedChild {
        mut child,
        // The temp profile must outlive the child — drop it after wait.
        _profile_temp,
    } = spawn_child_with_optional_sandbox(program, args, env, cwd, sandbox)
        .map_err(|err| format!("spawn {program}: {err}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        let bytes = stdin_bytes.to_vec();
        thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }

    let stdout_buf = Arc::new(Mutex::new(Vec::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::new()));
    let dispatch = tracing::dispatcher::get_default(Clone::clone);

    let stdout_reader = child.stdout.take().map(|handle| {
        spawn_output_reader(
            handle,
            Arc::clone(&stdout_buf),
            OutputReaderContext {
                provider: trace.provider.to_string(),
                stream: "stdout",
                job_run_id: trace.job_run_id.to_string(),
                task_id: trace.task_id.map(ToString::to_string),
                cwd: trace.cwd.map(ToString::to_string),
                dispatch: dispatch.clone(),
            },
        )
    });
    let stderr_reader = child.stderr.take().map(|handle| {
        spawn_output_reader(
            handle,
            Arc::clone(&stderr_buf),
            OutputReaderContext {
                provider: trace.provider.to_string(),
                stream: "stderr",
                job_run_id: trace.job_run_id.to_string(),
                task_id: trace.task_id.map(ToString::to_string),
                cwd: trace.cwd.map(ToString::to_string),
                dispatch,
            },
        )
    });

    let mut timed_out = false;
    let deadline = started + timeout;
    let exit_status;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                exit_status = Some(status);
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    timed_out = true;
                    let _ = child.wait();
                    exit_status = None;
                    break;
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(err) => return Err(format!("wait {program}: {err}")),
        }
    }

    if let Some(h) = stdout_reader {
        let _ = h.join();
    }
    if let Some(h) = stderr_reader {
        let _ = h.join();
    }

    let stdout = Arc::try_unwrap(stdout_buf)
        .map(|m| m.into_inner().unwrap_or_default())
        .unwrap_or_default();
    let stderr = Arc::try_unwrap(stderr_buf)
        .map(|m| m.into_inner().unwrap_or_default())
        .unwrap_or_default();
    let exit_code = exit_status.as_ref().and_then(|s| s.code());
    let duration = started.elapsed();
    Ok((stdout, stderr, exit_code, duration, timed_out))
}

fn spawn_output_reader<R>(
    handle: R,
    buf: Arc<Mutex<Vec<u8>>>,
    context: OutputReaderContext,
) -> thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    let OutputReaderContext {
        provider,
        stream,
        job_run_id,
        task_id,
        cwd,
        dispatch,
    } = context;

    thread::spawn(move || {
        tracing::dispatcher::with_default(&dispatch, || {
            let mut reader = BufReader::new(handle);
            let mut raw_line = Vec::new();
            loop {
                raw_line.clear();
                match reader.read_until(b'\n', &mut raw_line) {
                    Ok(0) => break,
                    Ok(_) => {
                        buf.lock()
                            .expect("subprocess output buf poisoned")
                            .extend_from_slice(&raw_line);
                        emit_output_line(
                            &provider,
                            stream,
                            &job_run_id,
                            task_id.as_deref(),
                            cwd.as_deref(),
                            &raw_line,
                        );
                    }
                    Err(_) => break,
                }
            }
        });
    })
}

fn emit_output_line(
    provider: &str,
    stream: &str,
    job_run_id: &str,
    task_id: Option<&str>,
    cwd: Option<&str>,
    raw_line: &[u8],
) {
    let line = line_text(raw_line);
    if let Some(cwd) = cwd {
        tracing::info!(
            provider = provider,
            stream = stream,
            job_run_id = job_run_id,
            task_id = task_id,
            cwd = cwd,
            line = line.as_str()
        );
    } else {
        tracing::info!(
            provider = provider,
            stream = stream,
            job_run_id = job_run_id,
            task_id = task_id,
            line = line.as_str()
        );
    }
}

fn line_text(raw_line: &[u8]) -> String {
    let line = raw_line.strip_suffix(b"\n").unwrap_or(raw_line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    String::from_utf8_lossy(line).into_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::HashMap;
    use std::fmt;
    use std::fs;
    use std::io::{self, Write};
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    use orbit_agent::loop_engine::audit::{AuditSink, LoopAuditEvent};
    use orbit_common::types::activity_job::{Backend, OnDenial, Provider};
    use orbit_common::utility::logging::RedactingFields;
    use orbit_tools::{FsAuditLogger, ToolContext};
    use tempfile::tempdir;
    use tracing::field::{Field, Visit};
    use tracing::{Event, Metadata, Subscriber, span};
    use tracing_subscriber::{Registry, fmt as tracing_fmt, fmt::MakeWriter, layer::SubscriberExt};

    use super::*;
    use crate::activity_job::dispatcher::ResolvedCliExecutor;

    fn spawn_test_request<'a>(
        program: &'a str,
        args: &'a [String],
        cwd: Option<&'a Path>,
        timeout: Duration,
        trace: SpawnTraceContext<'a>,
    ) -> SpawnWithTimeoutRequest<'a> {
        SpawnWithTimeoutRequest {
            program,
            args,
            stdin_bytes: b"",
            env: &[],
            cwd,
            timeout,
            sandbox: None,
            trace,
        }
    }

    #[test]
    fn user_prompt_from_object_input_without_prompt_serializes_full_input() {
        let input = serde_json::json!({
            "failed_step_id": "push",
            "activity_name": "git_push",
            "error_message": "network timeout",
            "attempt": 2,
            "max_attempts": 2,
        });

        let prompt = user_prompt_from_input(&input).expect("prompt serializes");
        let parsed: serde_json::Value = serde_json::from_str(&prompt).expect("prompt is json");

        assert_eq!(parsed, input);
    }

    #[test]
    fn user_prompt_from_object_input_prefers_explicit_prompt() {
        let prompt = user_prompt_from_input(&serde_json::json!({
            "prompt": "do only this",
            "failed_step_id": "push",
        }))
        .expect("prompt resolves");

        assert_eq!(prompt, "do only this");
    }

    #[test]
    fn spawn_with_timeout_emits_structured_stdout_and_stderr_events() {
        let args = sh_args("printf '%s\\n' out-one out-two; printf '%s\\n' err-one >&2");
        let (result, events) = capture_events(|| {
            spawn_with_timeout(spawn_test_request(
                "/bin/sh",
                &args,
                None,
                Duration::from_secs(5),
                SpawnTraceContext {
                    provider: "codex",
                    job_run_id: "job-123",
                    task_id: Some("T123"),
                    cwd: None,
                },
            ))
        });
        let (stdout, stderr, exit_code, _duration, timed_out) = result.expect("spawn succeeds");

        assert_eq!(stdout, b"out-one\nout-two\n");
        assert_eq!(stderr, b"err-one\n");
        assert_eq!(exit_code, Some(0));
        assert!(!timed_out);
        assert_eq!(events.len(), 3);

        assert_event(&events, "stdout", "out-one");
        assert_event(&events, "stdout", "out-two");
        assert_event(&events, "stderr", "err-one");
        for event in &events {
            assert_eq!(event.field("provider"), Some("codex"));
            assert_eq!(event.field("job_run_id"), Some("job-123"));
            assert_eq!(event.field("task_id"), Some("T123"));
            assert!(event.fields.contains_key("stream"));
            assert!(event.fields.contains_key("line"));
            assert!(!event.fields.contains_key("cwd"));
        }

        let cwd = tempdir().expect("cwd tempdir");
        let cwd_path = cwd.path().canonicalize().expect("canonical cwd");
        let cwd_string = cwd_path.display().to_string();
        let (result, events) = capture_events(|| {
            spawn_with_timeout(spawn_test_request(
                "/bin/sh",
                &args,
                Some(&cwd_path),
                Duration::from_secs(5),
                SpawnTraceContext {
                    provider: "codex",
                    job_run_id: "job-456",
                    task_id: Some("T456"),
                    cwd: Some(cwd_string.as_str()),
                },
            ))
        });
        let (stdout, stderr, exit_code, _duration, timed_out) = result.expect("spawn succeeds");

        assert_eq!(stdout, b"out-one\nout-two\n");
        assert_eq!(stderr, b"err-one\n");
        assert_eq!(exit_code, Some(0));
        assert!(!timed_out);
        assert_eq!(events.len(), 3);
        for event in &events {
            assert_eq!(event.field("cwd"), Some(cwd_string.as_str()));
        }
    }

    #[test]
    fn spawn_with_timeout_redacts_tracing_line_without_redacting_raw_stdout() {
        let args = sh_args("printf '%s\\n' 'Authorization: Bearer abc123'");
        let (result, formatted_output) = capture_redacted_tracing_output(|| {
            spawn_with_timeout(spawn_test_request(
                "/bin/sh",
                &args,
                None,
                Duration::from_secs(5),
                SpawnTraceContext {
                    provider: "codex",
                    job_run_id: "job-redact",
                    task_id: Some("TRED"),
                    cwd: None,
                },
            ))
        });
        let (stdout, stderr, exit_code, _duration, timed_out) = result.expect("spawn succeeds");

        assert_eq!(stdout, b"Authorization: Bearer abc123\n");
        assert!(stderr.is_empty());
        assert_eq!(exit_code, Some(0));
        assert!(!timed_out);
        assert!(formatted_output.contains("[REDACTED_AUTH]"));
        assert!(
            !formatted_output.contains("abc123"),
            "formatted tracing output leaked secret: {formatted_output}"
        );
    }

    #[test]
    fn spawn_bare_runs_program_in_provided_cwd() {
        let temp = tempdir().expect("tempdir");
        let cwd = temp.path().canonicalize().expect("canonical tempdir");
        let SpawnedChild {
            child,
            _profile_temp,
        } = spawn_bare("/bin/sh", &sh_args("pwd"), &[], Some(&cwd)).expect("spawn succeeds");

        let output = child.wait_with_output().expect("wait succeeds");
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).expect("stdout utf8"),
            format!("{}\n", cwd.display())
        );
    }

    #[test]
    fn run_cli_backend_finished_audit_event_keeps_stdout_stderr_blob_refs() {
        let temp = tempdir().expect("tempdir");
        let script = temp.path().join("codex");
        write_executable(
            &script,
            "#!/bin/sh\nprintf '%s\\n' 'plain stdout'\nprintf '%s\\n' 'plain stderr' >&2\n",
        );

        let sink = Arc::new(RecordingSink::default());
        let sink_for_writer: Arc<dyn AuditSink> = sink.clone();
        let audit = Arc::new(V2AuditWriter::new(
            "job-audit",
            "codex:gpt-5.5",
            sink_for_writer,
        ));
        let host = TestHost::with_command(script.display().to_string());
        let spec = test_agent_loop_spec(Duration::from_secs(5));
        let input = serde_json::json!({
            "prompt": "do it",
            "task_id": "TAUDIT"
        });

        let outcome = run_cli_backend(&host, &spec, "job-audit", audit.clone(), &input, None)
            .expect("run succeeds");

        assert!(outcome.success);
        assert_eq!(outcome.output["stdout_text"], "plain stdout\n");
        let events = audit.events_snapshot().expect("events snapshot");
        let finished = events
            .iter()
            .find_map(|event| match &event.kind {
                V2AuditEventKind::CliInvocationFinished {
                    provider,
                    exit_code,
                    stdout_blob_ref,
                    stderr_blob_ref,
                    timed_out,
                    ..
                } => Some((
                    provider.as_str(),
                    *exit_code,
                    stdout_blob_ref.as_deref(),
                    stderr_blob_ref.as_deref(),
                    *timed_out,
                )),
                _ => None,
            })
            .expect("finished event");

        assert_eq!(finished.0, "codex");
        assert_eq!(finished.1, Some(0));
        assert_eq!(finished.2, Some("blob-2"));
        assert_eq!(finished.3, Some("blob-3"));
        assert!(!finished.4);
        assert_eq!(sink.blob("blob-2"), Some(b"plain stdout\n".to_vec()));
        assert_eq!(sink.blob("blob-3"), Some(b"plain stderr\n".to_vec()));
    }

    #[test]
    fn run_cli_backend_returns_error_when_declared_workspace_path_missing() {
        let temp = tempdir().expect("tempdir");
        let script = temp.path().join("codex");
        write_executable(
            &script,
            "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"status\":\"ok\"}'\n",
        );
        let missing = temp.path().join("missing-worktree");

        let sink = Arc::new(RecordingSink::default());
        let sink_for_writer: Arc<dyn AuditSink> = sink;
        let audit = Arc::new(V2AuditWriter::new(
            "job-missing-cwd",
            "codex:gpt-5.5",
            sink_for_writer,
        ));
        let host = TestHost {
            command: script.display().to_string(),
            executor_args: Vec::new(),
            provider_config: HashMap::new(),
            sandbox: None,
            task_context: Some(serde_json::json!({
                "workspace_path": missing.display().to_string()
            })),
        };
        let spec = test_agent_loop_spec(Duration::from_secs(5));
        let input = serde_json::json!({
            "prompt": "do it",
            "task_id": "TMISSING"
        });

        let err = run_cli_backend(&host, &spec, "job-missing-cwd", audit.clone(), &input, None)
            .expect_err("missing declared workspace should fail");
        match err {
            DispatchError::CliInvocationFailed(message) => {
                assert!(
                    message.contains(&missing.display().to_string()),
                    "error should name missing path: {message}"
                );
            }
            other => panic!("expected CliInvocationFailed, got {other:?}"),
        }

        let events = audit.events_snapshot().expect("events snapshot");
        assert!(
            !events
                .iter()
                .any(|event| matches!(&event.kind, V2AuditEventKind::CliInvocationStarted { .. })),
            "CliInvocationStarted should not be emitted before cwd validation succeeds"
        );
    }

    #[test]
    fn run_cli_backend_records_resolved_cwd_in_started_event() {
        let temp = tempdir().expect("tempdir");
        let script = temp.path().join("codex");
        write_executable(
            &script,
            "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"status\":\"ok\"}'\n",
        );
        let workspace_dir = tempdir().expect("workspace tempdir");
        let workspace = workspace_dir
            .path()
            .canonicalize()
            .expect("canonical workspace");
        let workspace_string = workspace.display().to_string();

        let sink = Arc::new(RecordingSink::default());
        let sink_for_writer: Arc<dyn AuditSink> = sink;
        let audit = Arc::new(V2AuditWriter::new(
            "job-cwd-audit",
            "codex:gpt-5.5",
            sink_for_writer,
        ));
        let host = TestHost {
            command: script.display().to_string(),
            executor_args: Vec::new(),
            provider_config: HashMap::new(),
            sandbox: None,
            task_context: Some(serde_json::json!({
                "workspace_path": workspace_string.clone()
            })),
        };
        let spec = test_agent_loop_spec(Duration::from_secs(5));

        let outcome = run_cli_backend(
            &host,
            &spec,
            "job-cwd-audit",
            audit.clone(),
            &serde_json::json!({ "prompt": "do it", "task_id": "TCWD" }),
            None,
        )
        .expect("run succeeds");
        assert!(outcome.success);

        let events = audit.events_snapshot().expect("events snapshot");
        let cwd = events
            .iter()
            .find_map(|event| match &event.kind {
                V2AuditEventKind::CliInvocationStarted { cwd, .. } => cwd.as_deref(),
                _ => None,
            })
            .expect("cli.invocation.started cwd");
        assert_eq!(cwd, workspace_string);
    }

    #[test]
    fn run_cli_backend_passes_provider_config_to_codex_runtime_args() {
        let temp = tempdir().expect("tempdir");
        let script = temp.path().join("codex");
        write_executable(
            &script,
            "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"status\":\"ok\"}'\n",
        );

        let sink = Arc::new(RecordingSink::default());
        let sink_for_writer: Arc<dyn AuditSink> = sink;
        let audit = Arc::new(V2AuditWriter::new(
            "job-config",
            "codex:gpt-5.5",
            sink_for_writer,
        ));
        let mut provider_config = HashMap::new();
        provider_config.insert("sandbox".to_string(), "danger-full-access".to_string());
        provider_config.insert("approval_policy".to_string(), "never".to_string());
        provider_config.insert(
            "writable_dirs_json".to_string(),
            r#"["/tmp/orbit-a","/tmp/orbit-b"]"#.to_string(),
        );
        let host = TestHost {
            command: script.display().to_string(),
            executor_args: Vec::new(),
            provider_config,
            sandbox: None,
            task_context: None,
        };
        let spec = test_agent_loop_spec(Duration::from_secs(5));

        let outcome = run_cli_backend(
            &host,
            &spec,
            "job-config",
            audit.clone(),
            &serde_json::json!({ "prompt": "do it" }),
            None,
        )
        .expect("run succeeds");

        assert!(outcome.success);
        let events = audit.events_snapshot().expect("events snapshot");
        let argv = events
            .iter()
            .find_map(|event| match &event.kind {
                V2AuditEventKind::CliInvocationStarted { argv_redacted, .. } => Some(argv_redacted),
                _ => None,
            })
            .expect("cli.invocation.started event");

        assert_eq!(
            argv,
            &vec![
                script.display().to_string(),
                "--config".to_string(),
                "approval_policy=\"never\"".to_string(),
                "--sandbox".to_string(),
                "danger-full-access".to_string(),
                "--add-dir".to_string(),
                "/tmp/orbit-a".to_string(),
                "--add-dir".to_string(),
                "/tmp/orbit-b".to_string(),
            ]
        );
    }

    #[test]
    fn cli_agent_envelope_carries_input_run_id_and_task_context() {
        let host = TestHost {
            command: "codex".to_string(),
            executor_args: Vec::new(),
            provider_config: HashMap::new(),
            sandbox: None,
            task_context: Some(serde_json::json!({
                "id": "TCTX",
                "workspace_path": "/tmp/orbit-worktree",
                "plan": "implement it"
            })),
        };
        let spec = test_agent_loop_spec(Duration::from_secs(5));
        let input = serde_json::json!({
            "prompt": "do it",
            "task_id": "TCTX",
            "workspace_path": "/tmp/orbit-worktree"
        });

        let raw =
            cli_agent_envelope_json(&spec, "jrun-context", &input, host.task_context.as_ref())
                .expect("build cli agent envelope");
        let envelope: Value = serde_json::from_slice(&raw).expect("parse envelope json");

        assert_eq!(envelope["schemaVersion"], 1);
        assert_eq!(envelope["prompt"], "do it");
        assert_eq!(envelope["run_id"], "jrun-context");
        assert_eq!(envelope["input"]["task_id"], "TCTX");
        assert_eq!(envelope["input"]["workspace_path"], "/tmp/orbit-worktree");
        assert_eq!(envelope["task"]["id"], "TCTX");
        assert_eq!(envelope["task"]["workspace_path"], "/tmp/orbit-worktree");
    }

    #[test]
    fn spawn_with_timeout_kills_timed_out_process_and_keeps_partial_output() {
        let args = sh_args("printf '%s\\n' 'before timeout'; sleep 1; printf '%s\\n' after");
        let (result, events) = capture_events(|| {
            spawn_with_timeout(spawn_test_request(
                "/bin/sh",
                &args,
                None,
                Duration::from_millis(75),
                SpawnTraceContext {
                    provider: "codex",
                    job_run_id: "job-timeout",
                    task_id: Some("TTIME"),
                    cwd: None,
                },
            ))
        });
        let (stdout, stderr, exit_code, _duration, timed_out) = result.expect("spawn succeeds");

        assert_eq!(stdout, b"before timeout\n");
        assert!(stderr.is_empty());
        assert_eq!(exit_code, None);
        assert!(timed_out);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].field("stream"), Some("stdout"));
        assert_eq!(events[0].field("line"), Some("before timeout"));
    }

    fn sandbox_for_test() -> ResolvedSandbox {
        ResolvedSandbox {
            kind: ExecutorSandboxKind::MacosSandboxExec,
            fs_profile: orbit_common::types::ResolvedFsProfile {
                name: "default".to_string(),
                read: vec!["/tmp".to_string()],
                modify: vec!["/tmp".to_string()],
            },
            allow_fallback: false,
        }
    }

    #[test]
    fn audit_argv_for_dispatch_prepends_sandbox_exec_when_sandbox_active() {
        let argv = audit_argv_for_dispatch(
            "/usr/bin/claude",
            &["-p".to_string(), "hello".to_string()],
            Some(&sandbox_for_test()),
        );
        assert_eq!(
            argv,
            vec![
                "sandbox-exec",
                "-f",
                "<profile.sb>",
                "/usr/bin/claude",
                "-p",
                "hello"
            ]
        );
    }

    #[test]
    fn audit_argv_for_dispatch_returns_bare_when_no_sandbox() {
        let argv = audit_argv_for_dispatch(
            "/usr/bin/claude",
            &["-p".to_string(), "hello".to_string()],
            None,
        );
        assert_eq!(argv, vec!["/usr/bin/claude", "-p", "hello"]);
    }

    #[test]
    fn neutralize_inner_sandbox_pins_codex_to_danger_full_access() {
        let mut config = HashMap::new();
        config.insert("sandbox".to_string(), "workspace-write".to_string());
        let mut args = vec!["exec".to_string(), "--json".to_string()];
        neutralize_inner_sandbox("codex", &mut config, &mut args);
        assert_eq!(
            config.get("sandbox").map(String::as_str),
            Some("danger-full-access"),
            "codex sandbox should be pinned to danger-full-access when outer sandbox is active"
        );
        // Static args are untouched for codex; the sandbox flag flows
        // through provider_config.
        assert_eq!(args, vec!["exec", "--json"]);
    }

    #[test]
    fn neutralize_inner_sandbox_drops_gemini_sandbox_flags() {
        let mut config = HashMap::new();
        let mut args = vec![
            "--approval-mode".to_string(),
            "yolo".to_string(),
            "--sandbox".to_string(),
            "-s".to_string(),
            "-o".to_string(),
            "json".to_string(),
        ];
        neutralize_inner_sandbox("gemini", &mut config, &mut args);
        assert!(
            !args.iter().any(|a| a == "--sandbox" || a == "-s"),
            "gemini sandbox flags should be removed: {args:?}"
        );
        assert!(args.iter().any(|a| a == "--approval-mode"));
        assert!(args.iter().any(|a| a == "json"));
    }

    #[test]
    fn rewrite_debug_file_value_replaces_relative_path() {
        let mut args = vec![
            "-p".to_string(),
            "--debug-file".to_string(),
            ".orbit/state/logs/claude-debug.log".to_string(),
            "--tools".to_string(),
            "Read".to_string(),
        ];
        rewrite_debug_file_value(&mut args, std::path::Path::new("/Users/test/.claude"));
        assert_eq!(
            args,
            vec![
                "-p".to_string(),
                "--debug-file".to_string(),
                "/Users/test/.claude/claude-debug.log".to_string(),
                "--tools".to_string(),
                "Read".to_string(),
            ],
            "claude --debug-file value should be rewritten to <state_dir>/<basename>"
        );
    }

    #[test]
    fn rewrite_debug_file_value_handles_bare_filename() {
        let mut args = vec!["--debug-file".to_string(), "claude-debug.log".to_string()];
        rewrite_debug_file_value(&mut args, std::path::Path::new("/Users/test/.claude"));
        assert_eq!(args[1], "/Users/test/.claude/claude-debug.log");
    }

    #[test]
    fn rewrite_debug_file_value_no_op_without_flag() {
        let mut args = vec!["-p".to_string(), "--tools".to_string(), "Read".to_string()];
        let original = args.clone();
        rewrite_debug_file_value(&mut args, std::path::Path::new("/Users/test/.claude"));
        assert_eq!(
            args, original,
            "args without --debug-file should be untouched"
        );
    }

    #[test]
    fn rewrite_debug_file_value_rewrites_every_occurrence() {
        let mut args = vec![
            "--debug-file".to_string(),
            "first.log".to_string(),
            "--other".to_string(),
            "x".to_string(),
            "--debug-file".to_string(),
            "nested/dir/second.log".to_string(),
        ];
        rewrite_debug_file_value(&mut args, std::path::Path::new("/Users/test/.claude"));
        assert_eq!(args[1], "/Users/test/.claude/first.log");
        assert_eq!(args[5], "/Users/test/.claude/second.log");
    }

    #[test]
    fn rewrite_debug_file_value_falls_back_when_value_has_no_basename() {
        let mut args = vec!["--debug-file".to_string(), "/".to_string()];
        rewrite_debug_file_value(&mut args, std::path::Path::new("/Users/test/.claude"));
        assert_eq!(args[1], "/Users/test/.claude/claude-debug.log");
    }

    #[test]
    fn rewrite_debug_file_value_ignores_dangling_flag() {
        let mut args = vec!["-p".to_string(), "--debug-file".to_string()];
        let original = args.clone();
        rewrite_debug_file_value(&mut args, std::path::Path::new("/Users/test/.claude"));
        assert_eq!(
            args, original,
            "trailing --debug-file with no value must not panic or rewrite"
        );
    }

    #[test]
    fn neutralize_inner_sandbox_leaves_claude_args_unchanged() {
        let mut config = HashMap::new();
        let mut args = vec![
            "-p".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
            "--tools".to_string(),
            "Read,Write,Edit,Bash".to_string(),
        ];
        let original = args.clone();
        neutralize_inner_sandbox("claude", &mut config, &mut args);
        assert_eq!(
            args, original,
            "claude args must be unchanged by neutralization"
        );
        assert!(
            config.is_empty(),
            "claude provider_config must remain untouched"
        );
    }

    #[test]
    fn spawn_macos_sandboxed_returns_error_when_sandbox_exec_missing_and_fallback_disabled() {
        let sandbox = sandbox_for_test();
        let err = spawn_macos_sandboxed_with("/bin/sh", &[], &[], None, &sandbox, false)
            .expect_err("expected fallback-disabled error");
        match err {
            OrbitError::Execution(msg) => {
                assert!(
                    msg.contains("sandbox-exec not available"),
                    "unexpected error message: {msg}"
                );
            }
            other => panic!("expected Execution error, got {other:?}"),
        }
    }

    #[test]
    fn spawn_macos_sandboxed_falls_back_to_bare_exec_when_allow_fallback_set() {
        let sandbox = ResolvedSandbox {
            allow_fallback: true,
            ..sandbox_for_test()
        };
        let mut spawned = spawn_macos_sandboxed_with(
            "/bin/sh",
            &["-c".to_string(), "exit 0".to_string()],
            &[],
            None,
            &sandbox,
            false,
        )
        .expect("fallback should succeed");
        // The fallback path returns a SpawnedChild with no profile tempfile
        // because the sandbox-exec wrapper was bypassed.
        assert!(spawned._profile_temp.is_none());
        let _ = spawned.child.wait();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn run_cli_backend_audit_argv_starts_with_sandbox_exec_for_each_provider() {
        if !sandbox_exec_can_apply_for_test() {
            return;
        }

        for provider_name in ["claude", "codex", "gemini"] {
            let temp = tempdir().expect("tempdir");
            let script = temp.path().join(provider_name);
            write_executable(
                &script,
                "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"status\":\"ok\"}'\n",
            );

            let sink = Arc::new(RecordingSink::default());
            let sink_for_writer: Arc<dyn AuditSink> = sink;
            let audit = Arc::new(V2AuditWriter::new(
                "job-sandbox-shape",
                format!("{provider_name}:m"),
                sink_for_writer,
            ));
            let host = TestHost {
                command: script.display().to_string(),
                executor_args: Vec::new(),
                provider_config: HashMap::new(),
                sandbox: Some(sandbox_for_test()),
                task_context: None,
            };
            let spec = test_agent_loop_spec_for(provider_name, Duration::from_secs(5));

            let outcome = run_cli_backend(
                &host,
                &spec,
                "job-sandbox-shape",
                audit.clone(),
                &serde_json::json!({"prompt": "hi"}),
                None,
            )
            .expect("run cli backend");
            assert!(
                outcome.success,
                "provider {provider_name} cli backend failed"
            );

            let events = audit.events_snapshot().expect("events snapshot");
            let argv = events
                .iter()
                .find_map(|event| match &event.kind {
                    V2AuditEventKind::CliInvocationStarted { argv_redacted, .. } => {
                        Some(argv_redacted.clone())
                    }
                    _ => None,
                })
                .expect("cli.invocation.started event");
            assert_eq!(
                &argv[..3],
                &[
                    "sandbox-exec".to_string(),
                    "-f".to_string(),
                    "<profile.sb>".to_string()
                ],
                "provider {provider_name} should log sandbox-exec prefix; argv={argv:?}"
            );
            assert_eq!(
                argv[3],
                script.display().to_string(),
                "provider {provider_name} should log program after sandbox prefix"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn run_cli_backend_pins_codex_sandbox_under_outer_wrapper() {
        if !sandbox_exec_can_apply_for_test() {
            return;
        }

        let temp = tempdir().expect("tempdir");
        let script = temp.path().join("codex");
        write_executable(
            &script,
            "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"status\":\"ok\"}'\n",
        );

        let sink = Arc::new(RecordingSink::default());
        let sink_for_writer: Arc<dyn AuditSink> = sink;
        let audit = Arc::new(V2AuditWriter::new(
            "job-codex-pin",
            "codex:gpt-5.5",
            sink_for_writer,
        ));
        let mut provider_config = HashMap::new();
        provider_config.insert("sandbox".to_string(), "workspace-write".to_string());
        let host = TestHost {
            command: script.display().to_string(),
            executor_args: Vec::new(),
            provider_config,
            sandbox: Some(sandbox_for_test()),
            task_context: None,
        };
        let spec = test_agent_loop_spec_for("codex", Duration::from_secs(5));

        let outcome = run_cli_backend(
            &host,
            &spec,
            "job-codex-pin",
            audit.clone(),
            &serde_json::json!({"prompt": "hi"}),
            None,
        )
        .expect("run cli backend");
        assert!(outcome.success);

        let events = audit.events_snapshot().expect("events snapshot");
        let argv = events
            .iter()
            .find_map(|event| match &event.kind {
                V2AuditEventKind::CliInvocationStarted { argv_redacted, .. } => {
                    Some(argv_redacted.clone())
                }
                _ => None,
            })
            .expect("cli.invocation.started event");
        let mut idx = None;
        for (i, value) in argv.iter().enumerate() {
            if value == "--sandbox" {
                idx = Some(i);
                break;
            }
        }
        let i = idx.expect("argv must include --sandbox");
        assert_eq!(
            argv.get(i + 1).map(String::as_str),
            Some("danger-full-access"),
            "codex --sandbox must be pinned to danger-full-access; argv={argv:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn run_cli_backend_drops_gemini_sandbox_flag_under_outer_wrapper() {
        if !sandbox_exec_can_apply_for_test() {
            return;
        }

        let temp = tempdir().expect("tempdir");
        let script = temp.path().join("gemini");
        write_executable(
            &script,
            "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"status\":\"ok\"}'\n",
        );

        let sink = Arc::new(RecordingSink::default());
        let sink_for_writer: Arc<dyn AuditSink> = sink;
        let audit = Arc::new(V2AuditWriter::new(
            "job-gemini-drop",
            "gemini:gemini-3.1-pro",
            sink_for_writer,
        ));
        let host = TestHost {
            command: script.display().to_string(),
            executor_args: vec![
                "--approval-mode".to_string(),
                "yolo".to_string(),
                "--sandbox".to_string(),
                "-o".to_string(),
                "json".to_string(),
            ],
            provider_config: HashMap::new(),
            sandbox: Some(sandbox_for_test()),
            task_context: None,
        };
        let spec = test_agent_loop_spec_for("gemini", Duration::from_secs(5));

        let outcome = run_cli_backend(
            &host,
            &spec,
            "job-gemini-drop",
            audit.clone(),
            &serde_json::json!({"prompt": "hi"}),
            None,
        )
        .expect("run cli backend");
        assert!(outcome.success);

        let events = audit.events_snapshot().expect("events snapshot");
        let argv = events
            .iter()
            .find_map(|event| match &event.kind {
                V2AuditEventKind::CliInvocationStarted { argv_redacted, .. } => {
                    Some(argv_redacted.clone())
                }
                _ => None,
            })
            .expect("cli.invocation.started event");
        // Skip `sandbox-exec -f <profile.sb> <program>` prefix so the
        // assertion targets the gemini-side argv.
        let suffix = &argv[4..];
        assert!(
            !suffix.iter().any(|a| a == "--sandbox" || a == "-s"),
            "gemini argv suffix must not contain --sandbox / -s: {suffix:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn run_cli_backend_leaves_claude_argv_suffix_unchanged_under_sandbox() {
        if !sandbox_exec_can_apply_for_test() {
            return;
        }

        let temp = tempdir().expect("tempdir");
        let script = temp.path().join("claude");
        write_executable(
            &script,
            "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"status\":\"ok\"}'\n",
        );

        let sink = Arc::new(RecordingSink::default());
        let sink_for_writer: Arc<dyn AuditSink> = sink;
        let audit = Arc::new(V2AuditWriter::new(
            "job-claude-passthrough",
            "claude:claude-opus-4-7",
            sink_for_writer,
        ));
        let claude_static_args = vec![
            "-p".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
            "--output-format".to_string(),
            "json".to_string(),
        ];
        let host = TestHost {
            command: script.display().to_string(),
            executor_args: claude_static_args.clone(),
            provider_config: HashMap::new(),
            sandbox: Some(sandbox_for_test()),
            task_context: None,
        };
        let spec = test_agent_loop_spec_for("claude", Duration::from_secs(5));

        let outcome = run_cli_backend(
            &host,
            &spec,
            "job-claude-passthrough",
            audit.clone(),
            &serde_json::json!({"prompt": "hi"}),
            None,
        )
        .expect("run cli backend");
        assert!(outcome.success);

        let events = audit.events_snapshot().expect("events snapshot");
        let argv = events
            .iter()
            .find_map(|event| match &event.kind {
                V2AuditEventKind::CliInvocationStarted { argv_redacted, .. } => {
                    Some(argv_redacted.clone())
                }
                _ => None,
            })
            .expect("cli.invocation.started event");
        let suffix = &argv[4..4 + claude_static_args.len()];
        assert_eq!(
            suffix,
            claude_static_args.as_slice(),
            "claude static args must pass through unchanged"
        );
    }

    /// Regression for T20260508-17: a CLI subprocess that exits 0 but emits an
    /// embedded Orbit response envelope reporting `status: "failed"` must NOT
    /// be classified as success. Pre-fix, dispatch returned `success: true`
    /// because the dispatcher only consulted exit code, leaving the planning
    /// pipeline to push an empty branch and open an empty PR.
    #[test]
    fn run_cli_backend_demotes_success_when_envelope_reports_failed_despite_exit_zero() {
        let temp = tempdir().expect("tempdir");
        // The agent config layer infers provider from the command basename, so
        // the script name must match a known provider. The demotion logic is
        // provider-agnostic — codex exercises the same code path as claude.
        let script = temp.path().join("codex");
        // Stdout shape mirrors the Claude CLI: a wrapping JSON whose `result`
        // is a stringified Orbit envelope with status="failed". Exit 0.
        write_executable(
            &script,
            "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"{\\\"schemaVersion\\\":1,\\\"status\\\":\\\"failed\\\",\\\"error\\\":{\\\"code\\\":\\\"E\\\",\\\"message\\\":\\\"m\\\",\\\"details\\\":null}}\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}'\n",
        );

        let sink = Arc::new(RecordingSink::default());
        let sink_for_writer: Arc<dyn AuditSink> = sink;
        let audit = Arc::new(V2AuditWriter::new(
            "job-success-demote",
            "claude:s",
            sink_for_writer,
        ));
        let host = TestHost::with_command(script.display().to_string());
        let spec = test_agent_loop_spec(Duration::from_secs(5));

        let outcome = run_cli_backend(
            &host,
            &spec,
            "job-success-demote",
            audit,
            &serde_json::json!({"prompt": "hi"}),
            None,
        )
        .expect("run cli backend");

        assert!(
            !outcome.success,
            "envelope status=failed must demote dispatch success even on exit 0"
        );
        let message = outcome.message.expect("expected demote message");
        assert!(
            message.contains("envelope status") && message.contains("failed"),
            "demote message should explain envelope status; got {message:?}"
        );
    }

    /// Sanity check that the demotion does not regress the happy path: an exit-0
    /// run with a `status: "success"` envelope must still be classified as
    /// success. Without this, the demotion logic could silently flip every
    /// claude run to failed.
    #[test]
    fn run_cli_backend_keeps_success_when_envelope_reports_success() {
        let temp = tempdir().expect("tempdir");
        let script = temp.path().join("codex");
        write_executable(
            &script,
            "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{}}'\n",
        );

        let sink = Arc::new(RecordingSink::default());
        let sink_for_writer: Arc<dyn AuditSink> = sink;
        let audit = Arc::new(V2AuditWriter::new(
            "job-success-keep",
            "claude:s",
            sink_for_writer,
        ));
        let host = TestHost::with_command(script.display().to_string());
        let spec = test_agent_loop_spec(Duration::from_secs(5));

        let outcome = run_cli_backend(
            &host,
            &spec,
            "job-success-keep",
            audit,
            &serde_json::json!({"prompt": "hi"}),
            None,
        )
        .expect("run cli backend");
        assert!(
            outcome.success,
            "envelope status=success must keep dispatch success on exit 0"
        );
    }

    #[test]
    fn task_id_from_input_reads_common_activity_shapes() {
        assert_eq!(
            task_id_from_input(&serde_json::json!({"task_id": "T1"})),
            Some("T1")
        );
        assert_eq!(
            task_id_from_input(&serde_json::json!({"task": {"id": "T2"}})),
            Some("T2")
        );
        assert_eq!(
            task_id_from_input(&serde_json::json!({"task_ids": ["T3", "T4"]})),
            Some("T3")
        );
        assert_eq!(task_id_from_input(&serde_json::json!({})), None);
    }

    fn sh_args(script: &str) -> Vec<String> {
        vec!["-c".to_string(), script.to_string()]
    }

    fn capture_events<F>(f: F) -> (Result<SpawnOutput, String>, Vec<CapturedEvent>)
    where
        F: FnOnce() -> Result<SpawnOutput, String>,
    {
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = CaptureSubscriber {
            events: Arc::clone(&events),
            next_span_id: AtomicU64::new(1),
        };
        let dispatch = tracing::Dispatch::new(subscriber);
        let result = tracing::dispatcher::with_default(&dispatch, f);
        let events = events.lock().expect("events lock").clone();
        (result, events)
    }

    fn capture_redacted_tracing_output<F>(f: F) -> (Result<SpawnOutput, String>, String)
    where
        F: FnOnce() -> Result<SpawnOutput, String>,
    {
        let writer = BufferMakeWriter::default();
        let buffer = writer.buffer();
        let subscriber = Registry::default().with(
            tracing_fmt::layer()
                .with_ansi(false)
                .with_writer(writer)
                .fmt_fields(RedactingFields::default()),
        );
        let dispatch = tracing::Dispatch::new(subscriber);
        let result = tracing::dispatcher::with_default(&dispatch, f);
        let output = String::from_utf8(buffer.lock().expect("buffer lock").clone())
            .expect("formatted output utf8");
        (result, output)
    }

    fn assert_event(events: &[CapturedEvent], stream: &str, line: &str) {
        assert!(
            events
                .iter()
                .any(|event| event.field("stream") == Some(stream)
                    && event.field("line") == Some(line)),
            "missing event stream={stream:?} line={line:?}; captured={events:?}"
        );
    }

    #[derive(Debug, Clone)]
    struct CapturedEvent {
        fields: BTreeMap<String, String>,
    }

    impl CapturedEvent {
        fn field(&self, name: &str) -> Option<&str> {
            self.fields.get(name).map(String::as_str)
        }
    }

    struct CaptureSubscriber {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
        next_span_id: AtomicU64,
    }

    impl Subscriber for CaptureSubscriber {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &span::Attributes<'_>) -> span::Id {
            span::Id::from_u64(self.next_span_id.fetch_add(1, Ordering::Relaxed))
        }

        fn record(&self, _span: &span::Id, _values: &span::Record<'_>) {}

        fn record_follows_from(&self, _span: &span::Id, _follows: &span::Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut visitor = FieldCapture::default();
            event.record(&mut visitor);
            self.events
                .lock()
                .expect("events lock")
                .push(CapturedEvent {
                    fields: visitor.fields,
                });
        }

        fn enter(&self, _span: &span::Id) {}

        fn exit(&self, _span: &span::Id) {}
    }

    #[derive(Default)]
    struct FieldCapture {
        fields: BTreeMap<String, String>,
    }

    impl Visit for FieldCapture {
        fn record_str(&mut self, field: &Field, value: &str) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }

        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.fields
                .insert(field.name().to_string(), format!("{value:?}"));
        }
    }

    #[derive(Clone, Default)]
    struct BufferMakeWriter {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl BufferMakeWriter {
        fn buffer(&self) -> Arc<Mutex<Vec<u8>>> {
            Arc::clone(&self.buffer)
        }
    }

    impl<'writer> MakeWriter<'writer> for BufferMakeWriter {
        type Writer = BufferWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            BufferWriter {
                buffer: Arc::clone(&self.buffer),
            }
        }
    }

    struct BufferWriter {
        buffer: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for BufferWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.buffer
                .lock()
                .expect("buffer lock")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<LoopAuditEvent>>,
        blobs: Mutex<Vec<(String, Vec<u8>)>>,
    }

    impl RecordingSink {
        fn blob(&self, reference: &str) -> Option<Vec<u8>> {
            self.blobs
                .lock()
                .expect("blobs lock")
                .iter()
                .find_map(|(id, bytes)| {
                    if id == reference {
                        Some(bytes.clone())
                    } else {
                        None
                    }
                })
        }
    }

    impl AuditSink for RecordingSink {
        fn emit(&self, event: &LoopAuditEvent) {
            self.events.lock().expect("events lock").push(event.clone());
        }

        fn write_blob(&self, content: &[u8]) -> String {
            let mut blobs = self.blobs.lock().expect("blobs lock");
            let reference = format!("blob-{}", blobs.len() + 1);
            blobs.push((reference.clone(), content.to_vec()));
            reference
        }
    }

    struct TestHost {
        command: String,
        executor_args: Vec<String>,
        provider_config: HashMap<String, String>,
        sandbox: Option<ResolvedSandbox>,
        task_context: Option<Value>,
    }

    impl TestHost {
        fn with_command(command: String) -> Self {
            Self {
                command,
                executor_args: Vec::new(),
                provider_config: HashMap::new(),
                sandbox: None,
                task_context: None,
            }
        }
    }

    impl V2RuntimeHost for TestHost {
        fn run_deterministic(
            &self,
            _action: &str,
            _config: &Value,
            _input: &Value,
            _tool_context: ToolContext,
        ) -> Result<Value, DispatchError> {
            unreachable!("not used by cli runner tests")
        }

        fn api_key_for(&self, _provider: &str) -> Result<String, DispatchError> {
            Ok(String::new())
        }

        fn resolve_cli_executor(
            &self,
            _provider: &str,
        ) -> Result<ResolvedCliExecutor, DispatchError> {
            Ok(ResolvedCliExecutor {
                command: self.command.clone(),
                args: self.executor_args.clone(),
            })
        }

        fn provider_cli_config(&self, _provider: &str) -> HashMap<String, String> {
            self.provider_config.clone()
        }

        fn resolve_executor_sandbox(
            &self,
            _provider: &str,
            _fs_profile: Option<&str>,
            _subprocess_cwd: Option<&Path>,
        ) -> Result<Option<ResolvedSandbox>, DispatchError> {
            Ok(self.sandbox.clone())
        }

        fn task_context_for_agent_input(
            &self,
            _input: &Value,
        ) -> Result<Option<Value>, DispatchError> {
            Ok(self.task_context.clone())
        }

        fn tool_context_for_activity(
            &self,
            _run_id: Option<&str>,
            _fs_profile: Option<&str>,
            _fs_audit: Option<Arc<dyn FsAuditLogger>>,
        ) -> ToolContext {
            ToolContext::default()
        }
    }

    fn test_agent_loop_spec(timeout: Duration) -> AgentLoopSpec {
        AgentLoopSpec {
            instruction: String::new(),
            tools: Vec::new(),
            on_denial: OnDenial::Terminate,
            model: None,
            max_iterations: 1,
            backend: Backend::Cli,
            provider: Provider::Codex,
            wall_clock_timeout_seconds: timeout.as_secs(),
            role: None,
        }
    }

    #[cfg(target_os = "macos")]
    fn test_agent_loop_spec_for(provider: &str, timeout: Duration) -> AgentLoopSpec {
        let provider = match provider {
            "claude" => Provider::Claude,
            "codex" => Provider::Codex,
            "gemini" => Provider::Gemini,
            other => panic!("unsupported provider for test: {other}"),
        };
        AgentLoopSpec {
            instruction: String::new(),
            tools: Vec::new(),
            on_denial: OnDenial::Terminate,
            model: None,
            max_iterations: 1,
            backend: Backend::Cli,
            provider,
            wall_clock_timeout_seconds: timeout.as_secs(),
            role: None,
        }
    }

    fn write_executable(path: &Path, contents: &str) {
        fs::write(path, contents).expect("write script");
        make_executable(path);
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path).expect("script metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("script permissions");
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}

    #[cfg(target_os = "macos")]
    fn sandbox_exec_can_apply_for_test() -> bool {
        Command::new("sandbox-exec")
            .args(["-p", "(version 1)\n(allow default)\n", "/usr/bin/true"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}
