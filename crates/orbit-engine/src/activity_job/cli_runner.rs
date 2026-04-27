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

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use orbit_agent::{Agent, AgentConfig, AgentOperation, AgentRequest, parse_and_validate_response};
use orbit_common::types::activity_job::{AgentLoopSpec, V2AuditEventKind};
use orbit_common::types::{ExecutionResult, InvocationTrace};
use orbit_common::utility::redaction::PatternRedactor;
use serde_json::Value;

use super::audit_writer::V2AuditWriter;
use super::dispatcher::{DispatchError, DispatchInvocationTrace, DispatchOutcome, V2RuntimeHost};

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

    let provider_config = host.provider_cli_config(&provider);
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
        &provider,
        run_id,
        task_id_from_input(input),
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

fn spawn_with_timeout(
    program: &str,
    args: &[String],
    stdin_bytes: &[u8],
    timeout: Duration,
    provider: &str,
    job_run_id: &str,
    task_id: Option<&str>,
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
    let dispatch = tracing::dispatcher::get_default(Clone::clone);

    let stdout_reader = child.stdout.take().map(|handle| {
        spawn_output_reader(
            handle,
            Arc::clone(&stdout_buf),
            provider.to_string(),
            "stdout",
            job_run_id.to_string(),
            task_id.map(ToString::to_string),
            dispatch.clone(),
        )
    });
    let stderr_reader = child.stderr.take().map(|handle| {
        spawn_output_reader(
            handle,
            Arc::clone(&stderr_buf),
            provider.to_string(),
            "stderr",
            job_run_id.to_string(),
            task_id.map(ToString::to_string),
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
                "codex",
                "job-123",
                Some("T123"),
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
                "codex",
                "job-redact",
                Some("TRED"),
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
        let host = TestHost {
            command: script.display().to_string(),
            provider_config: HashMap::new(),
        };
        let spec = test_agent_loop_spec(Duration::from_secs(5));
        let input = serde_json::json!({
            "prompt": "do it",
            "task_id": "TAUDIT"
        });

        let outcome = run_cli_backend(&host, &spec, "job-audit", audit.clone(), &input)
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
            provider_config,
        };
        let spec = test_agent_loop_spec(Duration::from_secs(5));

        let outcome = run_cli_backend(
            &host,
            &spec,
            "job-config",
            audit.clone(),
            &serde_json::json!({ "prompt": "do it" }),
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
                "codex",
                "job-timeout",
                Some("TTIME"),
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
        provider_config: HashMap<String, String>,
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
                args: Vec::new(),
            })
        }

        fn provider_cli_config(&self, _provider: &str) -> HashMap<String, String> {
            self.provider_config.clone()
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
