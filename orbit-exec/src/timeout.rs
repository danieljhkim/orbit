use std::io::{Read, Write};
use std::process::Child;
#[cfg(unix)]
use std::sync::atomic::{AtomicI32, Ordering};
#[cfg(unix)]
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use orbit_types::OrbitError;
use orbit_types::redact_sensitive_env_text;
use wait_timeout::ChildExt;

const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(unix)]
const TERMINATION_GRACE_PERIOD: Duration = Duration::from_secs(5);

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
    debug: bool,
) -> Result<WaitResult, OrbitError> {
    // Drain stdout/stderr in background threads so the child never blocks on a
    // full pipe buffer (which would prevent it from exiting).
    //
    // In debug mode, stdout is tee'd to stderr so the user sees agent output
    // live while we still accumulate it for JSON parsing.  Stderr is inherited
    // directly by the child process (set in process::spawn), so no thread is
    // needed for it.
    let stdout_thread = child.stdout.take().map(|mut out| {
        thread::spawn(move || {
            if debug {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    match out.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            // Redact sensitive env values before printing to
                            // stderr so that tokens/secrets are never shown in
                            // debug output.
                            let raw = String::from_utf8_lossy(&chunk[..n]);
                            let redacted = redact_sensitive_env_text(&raw);
                            let _ = std::io::stderr().write_all(redacted.as_bytes());
                            buf.extend_from_slice(&chunk[..n]);
                        }
                    }
                }
                buf
            } else {
                let mut buf = Vec::new();
                let _ = out.read_to_end(&mut buf);
                buf
            }
        })
    });
    let stderr_thread = child.stderr.take().map(|mut err| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = err.read_to_end(&mut buf);
            buf
        })
    });

    #[cfg(unix)]
    let signal_guard = SignalHandlerGuard::install()?;

    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
    let (timed_out, interrupted_signal, exit_success, exit_code) = loop {
        let wait_slice = deadline
            .map(|end| {
                end.saturating_duration_since(Instant::now())
                    .min(WAIT_POLL_INTERVAL)
            })
            .unwrap_or(WAIT_POLL_INTERVAL);

        match child
            .wait_timeout(wait_slice)
            .map_err(|e| OrbitError::Execution(format!("wait timeout error: {e}")))?
        {
            Some(status) => {
                #[cfg(unix)]
                if let Some(signal) = signal_guard.take_signal() {
                    kill_process_group(child.id());
                    break (false, Some(signal), false, Some(128 + signal));
                }

                // Child exited successfully within the timeout. Kill its process
                // group so any orphan subprocesses still holding the pipes open
                // are reaped before we join the reader threads below.
                kill_process_group(child.id());
                break (false, None, status.success(), status.code());
            }
            None => {}
        }

        #[cfg(unix)]
        if let Some(signal) = signal_guard.take_signal() {
            terminate_process_group(&mut child, signal)?;
            break (false, Some(signal), false, Some(128 + signal));
        }

        if deadline.is_some_and(|end| Instant::now() >= end) {
            terminate_process_group(&mut child, termination_signal())?;
            break (true, None, false, None);
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
    } else if let Some(signal) = interrupted_signal {
        if !stderr.is_empty() {
            stderr.push(b'\n');
        }
        stderr.extend_from_slice(signal_message(signal).as_bytes());
    }

    Ok(WaitResult {
        exit_success,
        exit_code,
        stdout,
        stderr,
    })
}

#[cfg(unix)]
fn terminate_process_group(child: &mut Child, signal: i32) -> Result<(), OrbitError> {
    send_signal_to_process_group(child.id(), signal);
    match child
        .wait_timeout(TERMINATION_GRACE_PERIOD)
        .map_err(|e| OrbitError::Execution(format!("wait timeout error: {e}")))?
    {
        Some(_) => {
            kill_process_group(child.id());
        }
        None => {
            kill_process_group(child.id());
            child.kill().ok();
            child.wait().ok();
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn terminate_process_group(child: &mut Child, _signal: i32) -> Result<(), OrbitError> {
    child.kill().ok();
    child.wait().ok();
    Ok(())
}

fn signal_message(signal: i32) -> String {
    format!("process interrupted by signal {}", signal_name(signal))
}

#[cfg(unix)]
fn signal_name(signal: i32) -> &'static str {
    match signal {
        libc::SIGINT => "SIGINT",
        libc::SIGTERM => "SIGTERM",
        libc::SIGKILL => "SIGKILL",
        _ => "UNKNOWN",
    }
}

#[cfg(not(unix))]
fn signal_name(_signal: i32) -> &'static str {
    "UNKNOWN"
}

#[cfg(unix)]
fn termination_signal() -> i32 {
    libc::SIGTERM
}

#[cfg(not(unix))]
fn termination_signal() -> i32 {
    15
}

/// Send SIGKILL to every process in the child's process group.
///
/// Because we spawn children with `process_group(0)` the child's PGID equals
/// its PID, so `killpg` covers the child itself plus any subprocesses it
/// started that did not create their own group.
#[cfg(unix)]
fn kill_process_group(pid: u32) {
    send_signal_to_process_group(pid, libc::SIGKILL);
}

#[cfg(unix)]
fn send_signal_to_process_group(pid: u32, signal: i32) {
    // Safety: `killpg` is async-signal-safe and only sends a signal; no
    // undefined behaviour from the call itself.
    unsafe {
        libc::killpg(pid as libc::pid_t, signal);
    }
}

#[cfg(not(unix))]
fn kill_process_group(_pid: u32) {}

#[cfg(unix)]
static SIGNAL_HANDLER_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
#[cfg(unix)]
static SIGNAL_PIPE_WRITE_FD: AtomicI32 = AtomicI32::new(-1);

#[cfg(unix)]
struct SignalHandlerGuard {
    previous_sigint: libc::sigaction,
    previous_sigterm: libc::sigaction,
    read_fd: i32,
    write_fd: i32,
    _lock: MutexGuard<'static, ()>,
}

#[cfg(unix)]
impl SignalHandlerGuard {
    fn install() -> Result<Self, OrbitError> {
        let lock = SIGNAL_HANDLER_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .map_err(|_| OrbitError::Execution("signal handler lock poisoned".to_string()))?;

        let (read_fd, write_fd) = create_signal_pipe()?;
        SIGNAL_PIPE_WRITE_FD.store(write_fd, Ordering::SeqCst);
        let previous_sigint = install_signal_handler(libc::SIGINT)?;
        let previous_sigterm = match install_signal_handler(libc::SIGTERM) {
            Ok(previous) => previous,
            Err(err) => {
                SIGNAL_PIPE_WRITE_FD.store(-1, Ordering::SeqCst);
                close_fd(read_fd);
                close_fd(write_fd);
                restore_signal_handler(libc::SIGINT, &previous_sigint);
                return Err(err);
            }
        };

        Ok(Self {
            previous_sigint,
            previous_sigterm,
            read_fd,
            write_fd,
            _lock: lock,
        })
    }

    fn take_signal(&self) -> Option<i32> {
        let mut signal = [0_u8; 1];
        // Safety: the read end is non-blocking and owned by this guard.
        let result = unsafe { libc::read(self.read_fd, signal.as_mut_ptr().cast(), signal.len()) };
        if result > 0 {
            return Some(signal[0] as i32);
        }
        if result == 0 {
            return None;
        }

        match std::io::Error::last_os_error().raw_os_error() {
            Some(code) if code == libc::EAGAIN || code == libc::EWOULDBLOCK => None,
            _ => None,
        }
    }
}

#[cfg(unix)]
impl Drop for SignalHandlerGuard {
    fn drop(&mut self) {
        SIGNAL_PIPE_WRITE_FD.store(-1, Ordering::SeqCst);
        restore_signal_handler(libc::SIGINT, &self.previous_sigint);
        restore_signal_handler(libc::SIGTERM, &self.previous_sigterm);
        close_fd(self.read_fd);
        close_fd(self.write_fd);
    }
}

#[cfg(unix)]
unsafe extern "C" fn termination_signal_handler(signal: libc::c_int) {
    let fd = SIGNAL_PIPE_WRITE_FD.load(Ordering::Relaxed);
    if fd < 0 {
        return;
    }

    let byte = signal as u8;
    // Safety: `write` is async-signal-safe and writes a single byte to the
    // non-blocking pipe owned by the active guard.
    unsafe {
        libc::write(fd, (&byte as *const u8).cast(), 1);
    }
}

#[cfg(unix)]
fn install_signal_handler(signal: libc::c_int) -> Result<libc::sigaction, OrbitError> {
    // Safety: sigaction initializes/installs a process signal handler. The
    // handler only performs an atomic store, so it is safe for SIGINT/SIGTERM.
    unsafe {
        let mut new_action: libc::sigaction = std::mem::zeroed();
        new_action.sa_sigaction = termination_signal_handler as *const () as usize;
        new_action.sa_flags = 0;
        libc::sigemptyset(&mut new_action.sa_mask);

        let mut old_action: libc::sigaction = std::mem::zeroed();
        if libc::sigaction(signal, &new_action, &mut old_action) != 0 {
            return Err(OrbitError::Execution(format!(
                "failed to install signal handler for {}: {}",
                signal_name(signal),
                std::io::Error::last_os_error()
            )));
        }

        Ok(old_action)
    }
}

#[cfg(unix)]
fn restore_signal_handler(signal: libc::c_int, previous: &libc::sigaction) {
    // Safety: restores the exact handler previously returned by sigaction.
    unsafe {
        libc::sigaction(signal, previous, std::ptr::null_mut());
    }
}

#[cfg(unix)]
fn create_signal_pipe() -> Result<(i32, i32), OrbitError> {
    let mut fds = [0_i32; 2];
    // Safety: `pipe` initializes the two file descriptors when it returns 0.
    let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if rc != 0 {
        return Err(OrbitError::Execution(format!(
            "failed to create signal pipe: {}",
            std::io::Error::last_os_error()
        )));
    }

    if let Err(err) = set_nonblocking(fds[0]) {
        close_fd(fds[0]);
        close_fd(fds[1]);
        return Err(err);
    }

    Ok((fds[0], fds[1]))
}

#[cfg(unix)]
fn set_nonblocking(fd: i32) -> Result<(), OrbitError> {
    // Safety: `fcntl` queries and updates flags for this valid file descriptor.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return Err(OrbitError::Execution(format!(
                "failed to inspect signal pipe flags: {}",
                std::io::Error::last_os_error()
            )));
        }
        if libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) != 0 {
            return Err(OrbitError::Execution(format!(
                "failed to set signal pipe non-blocking mode: {}",
                std::io::Error::last_os_error()
            )));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn close_fd(fd: i32) {
    if fd >= 0 {
        // Safety: closing an owned file descriptor is safe; errors are ignored
        // during cleanup because the process is already tearing the guard down.
        unsafe {
            libc::close(fd);
        }
    }
}
