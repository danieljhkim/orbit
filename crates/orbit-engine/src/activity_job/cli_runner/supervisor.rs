// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::io::{Read, Write};
use std::path::Path;
use std::process::Child;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use super::super::dispatcher::ResolvedSandbox;
use super::spawn::{SpawnedChild, spawn_child_with_optional_sandbox};
use orbit_common::utility::output_capture::{BoundedOutputCapture, capture_limit_from_env};

/// Default wall-clock timeout when `AgentLoopSpec::wall_clock_timeout_seconds`
/// is zero. Matches §7.6 guidance: CLI subprocesses must have a mandatory
/// wall-clock guard.
pub(super) const DEFAULT_WALL_CLOCK_TIMEOUT_SECONDS: u64 = 300;

pub(super) type SpawnOutput = (Vec<u8>, Vec<u8>, Option<i32>, Duration, bool);

const OUTPUT_READER_JOIN_TIMEOUT: Duration = Duration::from_millis(500);
const CLI_RUNNER_OUTPUT_CAPTURE_LIMIT_ENV: &str = "ORBIT_CLI_RUNNER_OUTPUT_CAPTURE_LIMIT_BYTES";
const DEFAULT_CLI_RUNNER_OUTPUT_CAPTURE_LIMIT_BYTES: usize = 1024 * 1024;
const OUTPUT_LINE_EVENT_LIMIT_BYTES: usize = 64 * 1024;

type SharedOutputCapture = Arc<Mutex<BoundedOutputCapture>>;

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
    #[cfg(test)]
    pub(super) output_capture_limit: Option<usize>,
}

struct OutputReaderContext {
    provider: String,
    stream: &'static str,
    job_run_id: String,
    task_id: Option<String>,
    cwd: Option<String>,
    dispatch: tracing::Dispatch,
    limit_tx: mpsc::Sender<&'static str>,
}

struct OutputReaderHandle {
    finished: mpsc::Receiver<()>,
    join: thread::JoinHandle<()>,
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
        #[cfg(test)]
        output_capture_limit,
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

    #[cfg(test)]
    let output_limit = output_capture_limit.unwrap_or_else(default_output_capture_limit);
    #[cfg(not(test))]
    let output_limit = default_output_capture_limit();
    let stdout_buf = Arc::new(Mutex::new(BoundedOutputCapture::new(output_limit)));
    let stderr_buf = Arc::new(Mutex::new(BoundedOutputCapture::new(output_limit)));
    let (output_limit_tx, output_limit_rx) = mpsc::channel();
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
                limit_tx: output_limit_tx.clone(),
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
                limit_tx: output_limit_tx,
            },
        )
    });

    let mut timed_out = false;
    let deadline = started + timeout;
    let exit_status;
    loop {
        if output_limit_rx.try_recv().is_ok() {
            kill_child_process_tree(&mut child);
            exit_status = None;
            break;
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                cleanup_child_process_group(child.id());
                exit_status = Some(status);
                break;
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    kill_child_process_tree(&mut child);
                    timed_out = true;
                    exit_status = None;
                    break;
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(err) => return Err(format!("wait {program}: {err}")),
        }
    }

    let reader_join_deadline = timed_out.then(|| Instant::now() + OUTPUT_READER_JOIN_TIMEOUT);
    if let Some(h) = stdout_reader {
        join_output_reader(h, reader_join_deadline);
    }
    if let Some(h) = stderr_reader {
        join_output_reader(h, reader_join_deadline);
    }

    let stdout = stdout_buf
        .lock()
        .map(|buf| buf.as_bytes().to_vec())
        .unwrap_or_default();
    let stderr = stderr_buf
        .lock()
        .map(|buf| buf.as_bytes().to_vec())
        .unwrap_or_default();
    let exit_code = exit_status.as_ref().and_then(|s| s.code());
    let duration = started.elapsed();
    Ok((stdout, stderr, exit_code, duration, timed_out))
}

fn default_output_capture_limit() -> usize {
    capture_limit_from_env(
        CLI_RUNNER_OUTPUT_CAPTURE_LIMIT_ENV,
        DEFAULT_CLI_RUNNER_OUTPUT_CAPTURE_LIMIT_BYTES,
    )
}

fn spawn_output_reader<R>(
    handle: R,
    buf: SharedOutputCapture,
    context: OutputReaderContext,
) -> OutputReaderHandle
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
        limit_tx,
    } = context;

    let (finished_tx, finished) = mpsc::channel();
    let join = thread::spawn(move || {
        tracing::dispatcher::with_default(&dispatch, || {
            let mut reader = handle;
            let mut chunk = [0u8; 4096];
            let mut line_buf = Vec::new();
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let raw = &chunk[..n];
                        let exceeded = buf
                            .lock()
                            .expect("subprocess output buf poisoned")
                            .push(raw);
                        emit_output_chunk(
                            &provider,
                            stream,
                            &job_run_id,
                            task_id.as_deref(),
                            cwd.as_deref(),
                            raw,
                            &mut line_buf,
                        );
                        if exceeded {
                            let _ = limit_tx.send(stream);
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            if !line_buf.is_empty() {
                emit_output_line(
                    &provider,
                    stream,
                    &job_run_id,
                    task_id.as_deref(),
                    cwd.as_deref(),
                    &line_buf,
                );
            }
        });
        let _ = finished_tx.send(());
    });
    OutputReaderHandle { finished, join }
}

fn join_output_reader(reader: OutputReaderHandle, deadline: Option<Instant>) {
    let OutputReaderHandle { finished, join } = reader;
    match deadline {
        Some(deadline) => {
            let timeout = deadline.saturating_duration_since(Instant::now());
            match finished.recv_timeout(timeout) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = join.join();
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        None => {
            let _ = join.join();
        }
    }
}

fn kill_child_process_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let _ = signal_child_process_group(child.id(), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn cleanup_child_process_group(child_id: u32) {
    let _ = signal_child_process_group(child_id, libc::SIGKILL);
}

#[cfg(not(unix))]
fn cleanup_child_process_group(_child_id: u32) {}

#[cfg(unix)]
fn signal_child_process_group(child_id: u32, signal: libc::c_int) -> std::io::Result<()> {
    if child_id == 0 || child_id > i32::MAX as u32 {
        return Ok(());
    }
    let rc = unsafe { libc::killpg(child_id as libc::pid_t, signal) };
    if rc == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

fn emit_output_chunk(
    provider: &str,
    stream: &str,
    job_run_id: &str,
    task_id: Option<&str>,
    cwd: Option<&str>,
    raw: &[u8],
    line_buf: &mut Vec<u8>,
) {
    for segment in raw.split_inclusive(|byte| *byte == b'\n') {
        line_buf.extend_from_slice(segment);
        if segment.ends_with(b"\n") || line_buf.len() >= OUTPUT_LINE_EVENT_LIMIT_BYTES {
            emit_output_line(provider, stream, job_run_id, task_id, cwd, line_buf);
            line_buf.clear();
        }
    }
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
