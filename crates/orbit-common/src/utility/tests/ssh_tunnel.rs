//! Unit tests for the shared SSH local-forward tunnel primitive [ORB-10710].
//!
//! No real `ssh` process is spawned — a `sleep`/`false` child stands in
//! wherever a live [`SshTunnel`] is needed.

use std::net::{Ipv4Addr, TcpListener};
use std::process::Command;
use std::time::Duration;

use crate::types::OrbitError;
use crate::utility::ssh_tunnel::{
    SshTunnel, classify_ssh_exit, command_forward_args, ephemeral_port, forward_spec,
    poll_until_ready, probe_bindable, probe_forward_args, require_local_port, select_local_port,
    shell_quote,
};

/// A `sleep`-backed stand-in for a live `ssh` child: alive but never exits on
/// its own within a test's lifetime.
fn fake_live_tunnel() -> SshTunnel {
    let child = Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn sleep for the fake tunnel child");
    SshTunnel::new(child)
}

// ── port selection ────────────────────────────────────────────────────────

#[test]
fn ephemeral_port_is_nonzero_and_bindable() {
    let port = ephemeral_port().expect("ephemeral port");
    assert_ne!(port, 0);
    assert!(probe_bindable(port).is_ok());
}

#[test]
fn probe_bindable_detects_busy_port() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let busy = listener.local_addr().expect("addr").port();
    assert!(probe_bindable(busy).is_err());
}

#[test]
fn require_local_port_rejects_a_busy_port() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let busy = listener.local_addr().expect("addr").port();
    let error = require_local_port(busy).expect_err("a busy port must be refused");
    assert!(matches!(error, OrbitError::InvalidInput(_)));
}

#[test]
fn select_local_port_falls_back_when_the_default_is_busy() {
    // Occupy the caller's preferred default; selection must pick something
    // else that is itself bindable rather than handing back a doomed port.
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let busy = listener.local_addr().expect("addr").port();
    let chosen = select_local_port(None, busy).expect("auto select");
    assert_ne!(chosen, busy);
    assert!(probe_bindable(chosen).is_ok());
}

#[test]
fn select_local_port_uses_a_free_default() {
    let free = ephemeral_port().expect("ephemeral port");
    assert_eq!(select_local_port(None, free).expect("auto select"), free);
}

// ── forward argument construction ─────────────────────────────────────────

#[test]
fn probe_forward_never_carries_a_remote_command() {
    // A probe forward must never invoke anything remotely (`-N`, and no
    // trailing command arg). That is what keeps attaching to an
    // already-running remote server from starting a second one, and keeps
    // disconnecting from it from ever touching that process.
    let args = probe_forward_args("box", 9999, 7878);
    assert_eq!(
        args,
        vec![
            "-N".to_string(),
            "-o".to_string(),
            "ExitOnForwardFailure=yes".to_string(),
            "-L".to_string(),
            "9999:localhost:7878".to_string(),
            "box".to_string(),
        ]
    );
}

#[test]
fn command_forward_allocates_a_pty_and_carries_the_command() {
    let args = command_forward_args("user@host", 7000, 9000, "orbit web serve --no-open");
    assert_eq!(
        args,
        vec![
            "-tt".to_string(),
            "-o".to_string(),
            "ExitOnForwardFailure=yes".to_string(),
            "-L".to_string(),
            "7000:localhost:9000".to_string(),
            "user@host".to_string(),
            "orbit web serve --no-open".to_string(),
        ]
    );
}

#[test]
fn forward_spec_binds_local_to_remote_loopback() {
    assert_eq!(forward_spec(1234, 5678), "1234:localhost:5678");
}

#[test]
fn shell_quote_escapes_embedded_single_quotes() {
    assert_eq!(shell_quote("plain"), "'plain'");
    assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    assert_eq!(shell_quote("/srv/my ws"), "'/srv/my ws'");
}

// ── readiness polling (attach vs. spawn decision) ─────────────────────────

#[test]
fn poll_until_ready_reports_ready_when_the_probe_answers() {
    let mut tunnel = fake_live_tunnel();
    let ready = poll_until_ready(&mut tunnel, || true, Duration::from_secs(2), "test server")
        .expect("poll should not error");
    assert!(ready);
}

#[test]
fn poll_until_ready_reports_timeout_without_killing_the_forward() {
    // The "nothing running yet" path: the forward is up (tunnel still alive)
    // but nobody answers, so the caller must fall back to spawning a remote
    // server rather than erroring outright.
    let mut tunnel = fake_live_tunnel();
    let ready = poll_until_ready(
        &mut tunnel,
        || false,
        Duration::from_millis(600),
        "test server",
    )
    .expect("a timeout is not an error");
    assert!(!ready);
    assert!(
        tunnel.try_wait().expect("try_wait").is_none(),
        "the forward itself must still be alive after a plain timeout"
    );
}

#[cfg(unix)]
#[test]
fn poll_until_ready_errors_when_ssh_exits_early() {
    // If the underlying `ssh` process dies (bad host, auth failure, ...)
    // before either a ready answer or the timeout, that must surface as an
    // error rather than being silently treated as "nothing running yet".
    let child = Command::new("false").spawn().expect("spawn false");
    let mut tunnel = SshTunnel::new(child);
    let error = poll_until_ready(
        &mut tunnel,
        || false,
        Duration::from_secs(2),
        "orbit mcp serve --listen",
    )
    .expect_err("an early ssh exit must be an error, not a timeout");
    assert!(matches!(error, OrbitError::Execution(_)));
}

// ── ssh exit classification ───────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn classify_exit_127_points_at_remote_path() {
    use std::os::unix::process::ExitStatusExt;

    // waitpid encodes a normal exit code in the high byte.
    let error = classify_ssh_exit(
        std::process::ExitStatus::from_raw(127 << 8),
        "orbit web serve",
    );
    match error {
        OrbitError::Execution(message) => assert!(message.contains("PATH"), "got: {message}"),
        other => panic!("expected Execution, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn classify_exit_255_points_at_ssh_connect() {
    use std::os::unix::process::ExitStatusExt;

    let error = classify_ssh_exit(
        std::process::ExitStatus::from_raw(255 << 8),
        "orbit web serve",
    );
    match error {
        OrbitError::Execution(message) => assert!(message.contains("connect"), "got: {message}"),
        other => panic!("expected Execution, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn classify_other_exit_names_the_remote_command() {
    use std::os::unix::process::ExitStatusExt;

    let error = classify_ssh_exit(
        std::process::ExitStatus::from_raw(3 << 8),
        "orbit mcp serve --listen",
    );
    match error {
        OrbitError::Execution(message) => {
            assert!(
                message.contains("orbit mcp serve --listen"),
                "got: {message}"
            );
        }
        other => panic!("expected Execution, got {other:?}"),
    }
}

// ── teardown ──────────────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn shutdown_stops_a_running_forward_and_is_idempotent() {
    let mut tunnel = fake_live_tunnel();
    assert!(
        tunnel.try_wait().expect("try_wait").is_none(),
        "the stand-in child should still be running before teardown"
    );
    tunnel.shutdown();
    tunnel.shutdown(); // second call must be a no-op, not a panic
    assert!(
        tunnel.try_wait().expect("try_wait").is_none(),
        "a shut-down tunnel has released its child handle"
    );
}
