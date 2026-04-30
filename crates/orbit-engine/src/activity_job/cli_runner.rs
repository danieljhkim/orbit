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
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use orbit_agent::{Agent, AgentConfig, AgentOperation, AgentRequest, parse_and_validate_response};
use orbit_common::types::activity_job::{AgentLoopSpec, V2AuditEventKind};
use orbit_common::types::{ExecutionResult, ExecutorSandboxKind, InvocationTrace, OrbitError};
use orbit_common::utility::redaction::PatternRedactor;
use orbit_exec::{
    compile_macos_sandbox_profile, sandbox_exec_available, spawn_under_macos_sandbox,
};
use serde_json::Value;
use tempfile::NamedTempFile;

use super::audit_writer::V2AuditWriter;
use super::dispatcher::{
    DispatchError, DispatchInvocationTrace, DispatchOutcome, ResolvedSandbox, V2RuntimeHost,
};

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
    let sandbox = host.resolve_executor_sandbox(&provider, fs_profile)?;
    let prompt = user_prompt_from_input(input)?;
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

    let envelope = serde_json::json!({
        "instruction": spec.instruction,
        "prompt": prompt,
        "tools": spec.tools,
        "model": spec.model,
    });
    let envelope_json = serde_json::to_vec(&envelope)
        .map_err(|err| DispatchError::CliInvocationFailed(format!("serialize envelope: {err}")))?;

    let mut provider_config = host.provider_cli_config(&provider);

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
        wall_clock_timeout_ms: wall_clock_timeout.as_millis() as u64,
    });

    let (stdout, stderr, exit_code, duration, timed_out) = spawn_with_timeout(
        &invocation.program,
        &subprocess_args,
        &invocation.stdin,
        wall_clock_timeout,
        SpawnTraceContext {
            provider: &provider,
            job_run_id: run_id,
            task_id: task_id_from_input(input),
        },
        sandbox.as_ref(),
    )
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

    let success = !timed_out && matches!(exit_code, Some(0));
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
    } else if !success {
        Some(format!("cli subprocess exited with code {:?}", exit_code))
    } else {
        None
    };

    let stdout_text = String::from_utf8_lossy(&stdout).into_owned();

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
            None => Ok(String::new()),
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
    sandbox: Option<&ResolvedSandbox>,
) -> Result<SpawnedChild, OrbitError> {
    match sandbox {
        Some(sb) if sb.kind == ExecutorSandboxKind::MacosSandboxExec => {
            spawn_macos_sandboxed(program, args, sb)
        }
        Some(_) | None => spawn_bare(program, args),
    }
}

fn spawn_bare(program: &str, args: &[String]) -> Result<SpawnedChild, OrbitError> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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
    sandbox: &ResolvedSandbox,
) -> Result<SpawnedChild, OrbitError> {
    spawn_macos_sandboxed_with(program, args, sandbox, sandbox_exec_available())
}

/// Test-friendly variant of [`spawn_macos_sandboxed`]: callers pass an
/// explicit availability flag instead of probing `PATH`. Production routes
/// through the public wrapper which always reads the live `PATH`; tests
/// can assert the fail-closed and fallback branches without mutating
/// process-global state.
fn spawn_macos_sandboxed_with(
    program: &str,
    args: &[String],
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
            return spawn_bare(program, args);
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
    let (child, profile_temp) = spawn_under_macos_sandbox(
        &profile_text,
        program,
        args,
        Stdio::piped(),
        Stdio::piped(),
        Stdio::piped(),
    )?;
    Ok(SpawnedChild {
        child,
        _profile_temp: Some(profile_temp),
    })
}

struct SpawnTraceContext<'a> {
    provider: &'a str,
    job_run_id: &'a str,
    task_id: Option<&'a str>,
}

fn spawn_with_timeout(
    program: &str,
    args: &[String],
    stdin_bytes: &[u8],
    timeout: Duration,
    trace: SpawnTraceContext<'_>,
    sandbox: Option<&ResolvedSandbox>,
) -> Result<SpawnOutput, String> {
    let started = Instant::now();
    let SpawnedChild {
        mut child,
        // The temp profile must outlive the child — drop it after wait.
        _profile_temp,
    } = spawn_child_with_optional_sandbox(program, args, sandbox)
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
            trace.provider.to_string(),
            "stdout",
            trace.job_run_id.to_string(),
            trace.task_id.map(ToString::to_string),
            dispatch.clone(),
        )
    });
    let stderr_reader = child.stderr.take().map(|handle| {
        spawn_output_reader(
            handle,
            Arc::clone(&stderr_buf),
            trace.provider.to_string(),
            "stderr",
            trace.job_run_id.to_string(),
            trace.task_id.map(ToString::to_string),
            dispatch,
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
    provider: String,
    stream: &'static str,
    job_run_id: String,
    task_id: Option<String>,
    dispatch: tracing::Dispatch,
) -> thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
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
    raw_line: &[u8],
) {
    let line = line_text(raw_line);
    tracing::info!(
        provider = provider,
        stream = stream,
        job_run_id = job_run_id,
        task_id = task_id,
        line = line.as_str()
    );
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

    #[test]
    fn spawn_with_timeout_emits_structured_stdout_and_stderr_events() {
        let args = sh_args("printf '%s\\n' out-one out-two; printf '%s\\n' err-one >&2");
        let (result, events) = capture_events(|| {
            spawn_with_timeout(
                "/bin/sh",
                &args,
                b"",
                Duration::from_secs(5),
                SpawnTraceContext {
                    provider: "codex",
                    job_run_id: "job-123",
                    task_id: Some("T123"),
                },
                None,
            )
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
        }
    }

    #[test]
    fn spawn_with_timeout_redacts_tracing_line_without_redacting_raw_stdout() {
        let args = sh_args("printf '%s\\n' 'Authorization: Bearer abc123'");
        let (result, formatted_output) = capture_redacted_tracing_output(|| {
            spawn_with_timeout(
                "/bin/sh",
                &args,
                b"",
                Duration::from_secs(5),
                SpawnTraceContext {
                    provider: "codex",
                    job_run_id: "job-redact",
                    task_id: Some("TRED"),
                },
                None,
            )
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
    fn spawn_with_timeout_kills_timed_out_process_and_keeps_partial_output() {
        let args = sh_args("printf '%s\\n' 'before timeout'; sleep 1; printf '%s\\n' after");
        let (result, events) = capture_events(|| {
            spawn_with_timeout(
                "/bin/sh",
                &args,
                b"",
                Duration::from_millis(75),
                SpawnTraceContext {
                    provider: "codex",
                    job_run_id: "job-timeout",
                    task_id: Some("TTIME"),
                },
                None,
            )
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
        let err = spawn_macos_sandboxed_with("/bin/sh", &[], &sandbox, false)
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
    }

    impl TestHost {
        fn with_command(command: String) -> Self {
            Self {
                command,
                executor_args: Vec::new(),
                provider_config: HashMap::new(),
                sandbox: None,
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
        ) -> Result<Option<ResolvedSandbox>, DispatchError> {
            Ok(self.sandbox.clone())
        }

        fn tool_context_for_activity(
            &self,
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
}
