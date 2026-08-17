use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use orbit_common::OrbitError;

static SIGNAL_HANDLER_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static SIGNAL_PIPE_WRITE_FD: AtomicI32 = AtomicI32::new(-1);

pub(super) struct SignalHandlerGuard {
    previous_sigint: libc::sigaction,
    previous_sigterm: libc::sigaction,
    read_fd: i32,
    write_fd: i32,
    _lock: MutexGuard<'static, ()>,
}

impl SignalHandlerGuard {
    pub(super) fn install() -> Result<Self, OrbitError> {
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

    pub(super) fn take_signal(&self) -> Option<i32> {
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

impl Drop for SignalHandlerGuard {
    fn drop(&mut self) {
        SIGNAL_PIPE_WRITE_FD.store(-1, Ordering::SeqCst);
        restore_signal_handler(libc::SIGINT, &self.previous_sigint);
        restore_signal_handler(libc::SIGTERM, &self.previous_sigterm);
        close_fd(self.read_fd);
        close_fd(self.write_fd);
    }
}

pub(super) fn signal_message(signal: i32) -> String {
    format!("process interrupted by signal {}", signal_name(signal))
}

fn signal_name(signal: i32) -> &'static str {
    match signal {
        libc::SIGINT => "SIGINT",
        libc::SIGTERM => "SIGTERM",
        libc::SIGKILL => "SIGKILL",
        _ => "UNKNOWN",
    }
}

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

fn restore_signal_handler(signal: libc::c_int, previous: &libc::sigaction) {
    // Safety: restores the exact handler previously returned by sigaction.
    unsafe {
        libc::sigaction(signal, previous, std::ptr::null_mut());
    }
}

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

fn close_fd(fd: i32) {
    if fd >= 0 {
        // Safety: closing an owned file descriptor is safe; errors are ignored
        // during cleanup because the process is already tearing the guard down.
        unsafe {
            libc::close(fd);
        }
    }
}
