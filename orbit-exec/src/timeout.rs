use std::io::Read;
use std::process::Child;
use std::thread;
use std::time::Duration;

use orbit_types::OrbitError;
use wait_timeout::ChildExt;

/// Output collected from a spawned process.
pub(crate) struct WaitResult {
    pub(crate) exit_success: bool,
    pub(crate) exit_code: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    /// Stderr text; includes "process timed out" appended when timed out.
    pub(crate) stderr: Vec<u8>,
}

pub(crate) fn wait_with_optional_timeout(
    mut child: Child,
    timeout_ms: Option<u64>,
) -> Result<WaitResult, OrbitError> {
    // Drain stdout/stderr in background threads so the child never blocks on a
    // full pipe buffer (which would prevent it from exiting).
    let stdout_thread = child.stdout.take().map(|mut out| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out.read_to_end(&mut buf);
            buf
        })
    });
    let stderr_thread = child.stderr.take().map(|mut err| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = err.read_to_end(&mut buf);
            buf
        })
    });

    let (timed_out, exit_success, exit_code) = match timeout_ms {
        Some(ms) => {
            let timeout = Duration::from_millis(ms);
            match child
                .wait_timeout(timeout)
                .map_err(|e| OrbitError::Execution(format!("wait timeout error: {e}")))?
            {
                Some(status) => {
                    // Child exited successfully within the timeout.  Kill its
                    // process group so that orphan subprocesses (which may still
                    // hold the stdout/stderr pipe write ends open) are terminated.
                    // This unblocks the reader threads above.
                    kill_process_group(child.id());
                    (false, status.success(), status.code())
                }
                None => {
                    // Timeout elapsed.  Kill the process group (child + orphans).
                    kill_process_group(child.id());
                    child.kill().ok();
                    child.wait().ok(); // reap zombie
                    (true, false, None)
                }
            }
        }
        None => {
            // No timeout: block until the child exits, then kill any orphans.
            let status = child
                .wait()
                .map_err(|e| OrbitError::Execution(format!("failed waiting for process: {e}")))?;
            kill_process_group(child.id());
            (false, status.success(), status.code())
        }
    };

    // Join reader threads.  They complete quickly once the process group is
    // killed (all pipe write ends are closed → EOF).
    let stdout = stdout_thread
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();
    let mut stderr = stderr_thread
        .map(|h| h.join().unwrap_or_default())
        .unwrap_or_default();

    if timed_out {
        if !stderr.is_empty() {
            stderr.push(b'\n');
        }
        stderr.extend_from_slice(b"process timed out");
    }

    Ok(WaitResult {
        exit_success,
        exit_code,
        stdout,
        stderr,
    })
}

/// Send SIGKILL to every process in the child's process group.
///
/// Because we spawn children with `process_group(0)` the child's PGID equals
/// its PID, so `killpg` covers the child itself plus any subprocesses it
/// started that did not create their own group.
#[cfg(unix)]
fn kill_process_group(pid: u32) {
    // Safety: `killpg` is async-signal-safe and only sends a signal; no
    // undefined behaviour from the call itself.
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_process_group(_pid: u32) {}
