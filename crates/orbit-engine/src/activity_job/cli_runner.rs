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
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use orbit_agent::{Agent, AgentConfig, AgentOperation, AgentRequest};
use orbit_common::types::activity_job::{AgentLoopSpec, V2AuditEventKind};
use orbit_common::utility::redaction::PatternRedactor;
use serde_json::Value;

use super::audit_writer::V2AuditWriter;
use super::dispatcher::{DispatchError, DispatchOutcome, V2RuntimeHost};

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
) -> Result<DispatchOutcome, DispatchError> {
    let _ = run_id;

    let provider = spec.provider.as_str().to_string();
    let cli_executor = host.resolve_cli_executor(&provider)?;
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

    let config = AgentConfig::from_cli_config(
        cli_executor.command.clone(),
        spec.model.as_deref(),
        &HashMap::new(),
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

    let mut subprocess_args = Vec::with_capacity(cli_executor.args.len() + invocation.args.len());
    subprocess_args.extend(cli_executor.args.iter().cloned());
    subprocess_args.extend(invocation.args.iter().cloned());

    let mut argv = Vec::with_capacity(subprocess_args.len() + 1);
    argv.push(invocation.program.clone());
    argv.extend(subprocess_args.iter().cloned());

    let redaction = PatternRedactor::with_argv_secrets();
    let argv_redacted: Vec<String> = argv.iter().map(|a| redaction.apply_str(a)).collect();

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
    })
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

type SpawnOutput = (Vec<u8>, Vec<u8>, Option<i32>, Duration, bool);

fn spawn_with_timeout(
    program: &str,
    args: &[String],
    stdin_bytes: &[u8],
    timeout: Duration,
) -> Result<SpawnOutput, String> {
    let started = Instant::now();
    let mut child: Child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("spawn {program}: {err}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        let bytes = stdin_bytes.to_vec();
        thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }

    let stdout_buf = Arc::new(Mutex::new(Vec::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::new()));

    let stdout_reader = child.stdout.take().map(|mut handle| {
        let buf = Arc::clone(&stdout_buf);
        thread::spawn(move || {
            let mut local = Vec::new();
            let _ = std::io::copy(&mut handle, &mut local);
            *buf.lock().expect("stdout buf poisoned") = local;
        })
    });
    let stderr_reader = child.stderr.take().map(|mut handle| {
        let buf = Arc::clone(&stderr_buf);
        thread::spawn(move || {
            let mut local = Vec::new();
            let _ = std::io::copy(&mut handle, &mut local);
            *buf.lock().expect("stderr buf poisoned") = local;
        })
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
