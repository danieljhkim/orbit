use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::super::dispatcher::ResolvedSandbox;
use super::spawn::{SpawnedChild, spawn_child_with_optional_sandbox};

/// Default wall-clock timeout when `AgentLoopSpec::wall_clock_timeout_seconds`
/// is zero. Matches §7.6 guidance: CLI subprocesses must have a mandatory
/// wall-clock guard.
pub(super) const DEFAULT_WALL_CLOCK_TIMEOUT_SECONDS: u64 = 300;

pub(super) type SpawnOutput = (Vec<u8>, Vec<u8>, Option<i32>, Duration, bool);

pub(super) struct SpawnTraceContext<'a> {
    pub(super) provider: &'a str,
    pub(super) job_run_id: &'a str,
    pub(super) task_id: Option<&'a str>,
    pub(super) cwd: Option<&'a str>,
}

pub(super) struct SpawnWithTimeoutRequest<'a> {
    pub(super) program: &'a str,
    pub(super) args: &'a [String],
    pub(super) stdin_bytes: &'a [u8],
    pub(super) env: &'a [(String, String)],
    pub(super) cwd: Option<&'a Path>,
    pub(super) timeout: Duration,
    pub(super) sandbox: Option<&'a ResolvedSandbox>,
    pub(super) trace: SpawnTraceContext<'a>,
}

struct OutputReaderContext {
    provider: String,
    stream: &'static str,
    job_run_id: String,
    task_id: Option<String>,
    cwd: Option<String>,
    dispatch: tracing::Dispatch,
}

pub(super) fn spawn_with_timeout(
    request: SpawnWithTimeoutRequest<'_>,
) -> Result<SpawnOutput, String> {
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
    use std::path::Path;
    use std::time::Duration;

    use tempfile::tempdir;

    use super::super::test_support::{
        assert_event, capture_events, capture_redacted_tracing_output, sh_args,
    };
    use super::*;

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
}
