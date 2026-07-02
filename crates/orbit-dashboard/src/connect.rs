//! `orbit web connect <ssh-host>` — client-side SSH-tunnel convenience.
//!
//! The dashboard binds loopback-only by design (see [`crate::check_bindable_host`],
//! ORB-00360): it has no authentication, so it must never be exposed to a
//! network directly. To view a workspace's dashboard from another machine the
//! supported path is an authenticated SSH tunnel — historically the manual
//! `ssh -L 7878:localhost:7878 <host> "orbit web serve --no-open"`.
//!
//! `connect` automates exactly that workflow and nothing more: it delegates
//! authentication to SSH, keeps the loopback bind guard intact, and adds no new
//! attack surface. It runs `orbit web serve` on the remote over a single `ssh`
//! invocation that also forwards a local port, waits for the remote server to
//! answer `/healthz`, opens a browser, and — on Ctrl-C — tears the tunnel down
//! so no orphaned remote `orbit web serve` process is left behind.
//!
//! Unlike [`crate::serve`], this command reads no local `.orbit/` state: the
//! workspace lives on the remote, so it needs no [`orbit_core::OrbitRuntime`].

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use clap::Args;
use orbit_core::OrbitError;

use crate::{DEFAULT_DASHBOARD_PORT, open_browser};

/// How long to wait for the remote dashboard to answer `/healthz` before
/// giving up. Generous because it covers SSH connect + remote process spawn.
const READINESS_TIMEOUT: Duration = Duration::from_secs(30);

/// Delay between readiness probes.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Per-probe TCP connect/read/write timeout for the `/healthz` check.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Grace period between SIGTERM and SIGKILL when tearing the tunnel down.
#[cfg(unix)]
const TEARDOWN_GRACE: Duration = Duration::from_secs(2);

/// Arguments for `orbit web connect`.
#[derive(Args, Clone)]
#[command(about = "Open a remote workspace's dashboard over an SSH tunnel")]
pub struct ConnectArgs {
    /// SSH destination — anything `ssh` accepts (`host`, `user@host`, or a
    /// `~/.ssh/config` alias).
    pub ssh_host: String,

    /// Local port for the tunnel. Defaults to 7878, falling back to an
    /// ephemeral port if 7878 is already in use.
    #[arg(long)]
    pub port: Option<u16>,

    /// Port the remote `orbit web serve` listens on (remote loopback).
    #[arg(long, default_value_t = DEFAULT_DASHBOARD_PORT)]
    pub remote_port: u16,

    /// Remote workspace path, passed through to `orbit web serve --root` on
    /// the remote host. Omit to use the remote's default workspace resolution.
    #[arg(long)]
    pub root: Option<String>,

    /// Do not open the dashboard URL in a browser once the tunnel is ready.
    #[arg(long)]
    pub no_open: bool,
}

/// Establish the tunnel, wait for readiness, open the browser, and block until
/// Ctrl-C / SIGTERM — then tear everything down cleanly.
pub fn connect(args: ConnectArgs) -> Result<(), OrbitError> {
    let local_port = select_local_port(args.port)?;
    let ssh_args = build_ssh_args(&args, local_port);

    // Force a pty (`-tt`) so that when we kill the local `ssh`, the remote
    // `orbit web serve` receives SIGHUP and exits — no orphan. `stdin` is null
    // so Ctrl-C is delivered to *us* (the foreground process) rather than being
    // forwarded down the pty to the remote.
    let child = Command::new("ssh")
        .args(&ssh_args)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| OrbitError::Io(format!("failed to launch ssh: {e}")))?;

    // From here on, every exit path (error, panic, normal) tears the tunnel
    // down via `SshTunnel`'s `Drop`.
    let mut tunnel = SshTunnel::new(child);

    let url = format!("http://localhost:{local_port}");
    wait_until_ready(&mut tunnel, local_port)?;

    #[allow(clippy::print_stdout)]
    {
        println!("Dashboard tunnel ready: {url}  (Ctrl-C to disconnect)");
    }

    if !args.no_open {
        open_browser(&url);
    }

    wait_for_shutdown(&mut tunnel);
    Ok(())
}

// Visibility note: the pure helpers below are `pub(crate)` so the sibling
// `tests/connect.rs` module can exercise them directly (the crate's test-layout
// convention). None are part of the crate's public API.

/// Choose the local port to bind the tunnel to.
///
/// - An explicit `--port` is honored or fails with a clear error if busy.
/// - Otherwise prefer the conventional [`DEFAULT_DASHBOARD_PORT`], falling back
///   to an OS-assigned ephemeral port when it is taken.
///
/// Note: this is inherently racy (TOCTOU) — the probed port can be claimed by
/// another process before `ssh` binds it. That is acceptable for a developer
/// convenience command; if it happens, `ssh -L` fails loudly on startup.
pub(crate) fn select_local_port(preferred: Option<u16>) -> Result<u16, OrbitError> {
    match preferred {
        Some(port) => {
            probe_bindable(port).map_err(|e| {
                OrbitError::InvalidInput(format!(
                    "requested local port {port} is not available: {e}"
                ))
            })?;
            Ok(port)
        }
        None => {
            if probe_bindable(DEFAULT_DASHBOARD_PORT).is_ok() {
                Ok(DEFAULT_DASHBOARD_PORT)
            } else {
                ephemeral_port()
            }
        }
    }
}

/// Return `Ok` if a loopback TCP listener can bind `port` (immediately released).
pub(crate) fn probe_bindable(port: u16) -> std::io::Result<()> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map(|_| ())
}

/// Ask the OS for a free ephemeral loopback port.
pub(crate) fn ephemeral_port() -> Result<u16, OrbitError> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|e| OrbitError::Io(format!("could not reserve a local port: {e}")))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| OrbitError::Io(format!("could not read reserved local port: {e}")))
}

/// Build the argument vector passed to `ssh` (everything after the program).
///
/// Pure and deterministic so it can be unit-tested without spawning anything.
pub(crate) fn build_ssh_args(cfg: &ConnectArgs, local_port: u16) -> Vec<String> {
    vec![
        // Force pty allocation even though our stdin is null, so the remote
        // command dies (SIGHUP) when the tunnel drops.
        "-tt".to_string(),
        // Fail fast if the local port cannot be forwarded rather than silently
        // running the remote command with no working tunnel.
        "-o".to_string(),
        "ExitOnForwardFailure=yes".to_string(),
        "-L".to_string(),
        format!("{local_port}:localhost:{}", cfg.remote_port),
        cfg.ssh_host.clone(),
        remote_serve_command(cfg),
    ]
}

/// The remote shell command line: `orbit web serve --no-open --port N [--root P]`.
///
/// `ssh` concatenates trailing args with spaces and re-parses them via the
/// remote shell, so any value that could contain spaces (`--root`) is
/// shell-quoted.
pub(crate) fn remote_serve_command(cfg: &ConnectArgs) -> String {
    let mut cmd = format!("orbit web serve --no-open --port {}", cfg.remote_port);
    if let Some(root) = &cfg.root {
        cmd.push_str(" --root ");
        cmd.push_str(&shell_quote(root));
    }
    cmd
}

/// POSIX single-quote a value for safe interpolation into the remote command.
pub(crate) fn shell_quote(value: &str) -> String {
    // Wrap in single quotes; a literal `'` becomes `'\''`.
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Poll `/healthz` until the remote dashboard answers, or fail if `ssh` exits
/// first (misconfigured host, `orbit` not on PATH, …) or the timeout elapses.
fn wait_until_ready(tunnel: &mut SshTunnel, local_port: u16) -> Result<(), OrbitError> {
    let deadline = Instant::now() + READINESS_TIMEOUT;
    loop {
        if let Some(status) = tunnel.try_wait()? {
            return Err(classify_ssh_exit(status));
        }
        if healthz_ok(local_port) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(OrbitError::Execution(format!(
                "timed out after {}s waiting for the remote dashboard at \
                 http://localhost:{local_port}/healthz to become ready",
                READINESS_TIMEOUT.as_secs()
            )));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Best-effort `GET /healthz` over the forwarded local port. Returns `true`
/// only on a `200` status line. Any connect/IO error (including `ssh` refusing
/// the forwarded connection because the remote server is not up yet) is `false`.
fn healthz_ok(local_port: u16) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, local_port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(PROBE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));
    if stream
        .write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut status_line = String::new();
    if BufReader::new(stream).read_line(&mut status_line).is_err() {
        return false;
    }
    status_line.starts_with("HTTP/1.") && status_line.contains(" 200 ")
}

/// Map an early `ssh` exit to an actionable error.
pub(crate) fn classify_ssh_exit(status: ExitStatus) -> OrbitError {
    match status.code() {
        // The remote shell returns 127 when it cannot find the command.
        Some(127) => OrbitError::Execution(
            "`orbit` was not found on the remote host's PATH (ssh exited 127). \
             Ensure orbit is installed and on PATH for non-interactive SSH \
             sessions (e.g. add it to ~/.profile / ~/.bashrc on the remote)."
                .to_string(),
        ),
        // ssh's own failure code (bad host, auth, network).
        Some(255) => OrbitError::Execution(
            "ssh could not connect (exit 255). Check the host, your SSH \
             config/keys, and network reachability."
                .to_string(),
        ),
        Some(code) => OrbitError::Execution(format!(
            "remote `orbit web serve` exited with status {code} before the \
             dashboard became ready"
        )),
        None => OrbitError::Execution(
            "ssh was terminated by a signal before the dashboard became ready".to_string(),
        ),
    }
}

/// Block until Ctrl-C / SIGTERM, or until the `ssh` child exits on its own
/// (e.g. the remote server dies). Teardown then happens via `SshTunnel::Drop`.
///
/// Uses a small current-thread tokio runtime and the same signal primitives as
/// [`crate::serve`] so behavior is consistent across the two `web` surfaces. If
/// the runtime cannot be built we fall back to a plain poll loop that at least
/// returns when the child exits (teardown still runs on drop; a Ctrl-C in that
/// degraded mode terminates the process, and the OS reaps the tunnel).
fn wait_for_shutdown(tunnel: &mut SshTunnel) {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        while !matches!(tunnel.try_wait(), Ok(Some(_))) {
            std::thread::sleep(POLL_INTERVAL);
        }
        return;
    };

    rt.block_on(async {
        let ctrl_c = async {
            let _ = tokio::signal::ctrl_c().await;
        };

        #[cfg(unix)]
        let terminate = async {
            if let Ok(mut sig) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            {
                sig.recv().await;
            }
        };
        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        // Also return if `ssh` dies on its own (remote server crashed, network
        // dropped, …) so we do not hang forever.
        let child_exit = async {
            while !matches!(tunnel.try_wait(), Ok(Some(_))) {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        };

        tokio::select! {
            _ = ctrl_c => {}
            _ = terminate => {}
            _ = child_exit => {}
        }
    });
}

/// RAII owner of the `ssh` child that guarantees teardown of the tunnel (and,
/// via the remote pty's SIGHUP, the remote `orbit web serve`) on drop.
pub(crate) struct SshTunnel {
    child: Option<Child>,
}

impl SshTunnel {
    pub(crate) fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    /// Non-blocking check for the child's exit status.
    pub(crate) fn try_wait(&mut self) -> Result<Option<ExitStatus>, OrbitError> {
        match &mut self.child {
            Some(child) => child
                .try_wait()
                .map_err(|e| OrbitError::Io(format!("waiting on ssh: {e}"))),
            None => Ok(None),
        }
    }

    /// Terminate the `ssh` child if it is still running. Idempotent.
    pub(crate) fn shutdown(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if let Ok(Some(_)) = child.try_wait() {
            return; // already gone
        }
        terminate_child(&mut child);
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Ask the child to exit gracefully (SIGTERM), then force it (SIGKILL) if it
/// does not within [`TEARDOWN_GRACE`]. Closing `ssh` drops the connection,
/// which delivers SIGHUP to the remote pty session and stops the remote serve.
#[cfg(unix)]
pub(crate) fn terminate_child(child: &mut Child) {
    let pid = child.id() as libc::pid_t;
    // SAFETY: `pid` is our own direct child; signalling it is well-defined.
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + TEARDOWN_GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(_) => break,
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(not(unix))]
pub(crate) fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}
