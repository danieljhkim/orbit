#![allow(missing_docs)]

use std::path::Path;
use std::time::Duration;

use tempfile::tempdir;

use super::super::supervisor::{SpawnTraceContext, SpawnWithTimeoutRequest, spawn_with_timeout};
use super::test_support::{assert_event, capture_events, capture_redacted_tracing_output, sh_args};

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

#[cfg(unix)]
#[test]
fn spawn_with_timeout_kills_grandchild_holding_output_pipes() {
    let pid_dir = tempdir().expect("pid tempdir");
    let pid_file = pid_dir.path().join("grandchild.pid");
    let script = format!(
        "(sleep 30) & child=$!; printf '%s\\n' \"$child\" > {}; printf '%s\\n' 'before timeout'; sleep 30",
        shell_quote(pid_file.to_string_lossy().as_ref())
    );
    let args = sh_args(&script);

    let started = std::time::Instant::now();
    let (stdout, stderr, exit_code, duration, timed_out) = spawn_with_timeout(spawn_test_request(
        "/bin/sh",
        &args,
        None,
        Duration::from_millis(150),
        SpawnTraceContext {
            provider: "codex",
            job_run_id: "job-timeout-tree",
            task_id: Some("TTREE"),
            cwd: None,
        },
    ))
    .expect("spawn succeeds");

    assert!(timed_out);
    assert_eq!(exit_code, None);
    assert_eq!(stdout, b"before timeout\n");
    assert!(stderr.is_empty());
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "timeout path should return promptly; reported duration={duration:?}"
    );

    let grandchild_pid = read_pid(&pid_file);
    assert!(
        wait_until(Duration::from_secs(2), || !process_is_live(grandchild_pid)),
        "grandchild process {grandchild_pid} should be gone after timeout"
    );
}

#[cfg(unix)]
fn read_pid(path: &Path) -> u32 {
    std::fs::read_to_string(path)
        .expect("read pid file")
        .trim()
        .parse()
        .expect("parse pid")
}

#[cfg(unix)]
fn wait_until<F>(timeout: Duration, mut condition: F) -> bool
where
    F: FnMut() -> bool,
{
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    condition()
}

#[cfg(unix)]
fn process_is_live(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        return false;
    }
    let output = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output();
    let Ok(output) = output else {
        return true;
    };
    if !output.status.success() {
        return false;
    }
    let status = String::from_utf8_lossy(&output.stdout);
    !status.trim_start().starts_with('Z')
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
