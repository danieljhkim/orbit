//! Unit tests for `orbit web connect` helpers (port selection, ssh arg
//! construction, readiness polling, and tunnel teardown). No real `ssh`
//! process is spawned — a `sleep`/`true` child stands in wherever a live
//! `SshTunnel` is needed.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::process::Command;
use std::time::Duration;

use orbit_core::OrbitError;

use super::super::DEFAULT_DASHBOARD_PORT;
use super::super::connect::{
    ConnectArgs, SshTunnel, build_probe_ssh_args, build_ssh_args, ephemeral_port, poll_until_ready,
    probe_bindable, remote_serve_command, select_local_port, shell_quote,
};

/// Minimal args builder so each test states only what it cares about.
fn args(host: &str, remote_port: u16, root: Option<&str>) -> ConnectArgs {
    ConnectArgs {
        ssh_host: host.to_string(),
        port: None,
        remote_port,
        root: root.map(str::to_string),
        global: false,
        no_open: false,
    }
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
    // Hold a listener open so the port is genuinely in use.
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let busy = listener.local_addr().expect("addr").port();
    assert!(probe_bindable(busy).is_err());
}

#[test]
fn explicit_free_port_is_honored() {
    let free = ephemeral_port().expect("ephemeral port");
    assert_eq!(select_local_port(Some(free)).expect("select"), free);
}

#[test]
fn explicit_busy_port_errors() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
    let busy = listener.local_addr().expect("addr").port();
    let err = select_local_port(Some(busy)).expect_err("busy port must error");
    assert!(matches!(err, OrbitError::InvalidInput(_)));
}

#[test]
fn auto_selection_falls_back_when_default_is_busy() {
    // Occupy the preferred default port; auto-selection must pick something
    // else that is itself bindable. If 7878 is already taken by another
    // process in this environment, the same fallback path runs but we cannot
    // assert deterministically, so we skip in that case.
    if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, DEFAULT_DASHBOARD_PORT)) {
        let chosen = select_local_port(None).expect("auto select");
        assert_ne!(chosen, DEFAULT_DASHBOARD_PORT);
        assert!(probe_bindable(chosen).is_ok());
        drop(listener);
    }
}

// ── ssh argument construction ─────────────────────────────────────────────

#[test]
fn build_ssh_args_basic() {
    let got = build_ssh_args(&args("box", 7878, None), 9999);
    assert_eq!(
        got,
        vec![
            "-tt".to_string(),
            "-o".to_string(),
            "ExitOnForwardFailure=yes".to_string(),
            "-L".to_string(),
            "9999:localhost:7878".to_string(),
            "box".to_string(),
            "orbit web serve --no-open --port 7878".to_string(),
        ]
    );
}

#[test]
fn build_ssh_args_forwards_distinct_local_and_remote_ports() {
    let got = build_ssh_args(&args("user@host", 9000, None), 7000);
    assert!(got.contains(&"7000:localhost:9000".to_string()));
    assert!(got.contains(&"orbit web serve --no-open --port 9000".to_string()));
}

#[test]
fn remote_command_always_passes_no_open() {
    // The remote must never open a browser on the box regardless of the local
    // `--no-open` flag (which only controls *our* browser).
    let mut cfg = args("box", 7878, None);
    cfg.no_open = true;
    assert!(remote_serve_command(&cfg).contains("--no-open"));
    cfg.no_open = false;
    assert!(remote_serve_command(&cfg).contains("--no-open"));
}

#[test]
fn remote_command_shell_quotes_root() {
    let cmd = remote_serve_command(&args("box", 7878, Some("/srv/my ws")));
    assert!(
        cmd.contains("--root '/srv/my ws'"),
        "root with a space must be single-quoted: {cmd}"
    );
}

#[test]
fn remote_command_without_root_has_no_root_flag() {
    let cmd = remote_serve_command(&args("box", 7878, None));
    assert!(!cmd.contains("--root"));
}

#[test]
fn remote_command_passes_global_when_set() {
    let mut cfg = args("box", 7878, None);
    assert!(!remote_serve_command(&cfg).contains("--global"));
    cfg.global = true;
    let cmd = remote_serve_command(&cfg);
    assert!(
        cmd.contains("--global"),
        "remote serve must receive --global: {cmd}"
    );
}

#[test]
fn remote_command_combines_global_and_root() {
    let mut cfg = args("box", 7878, Some("/srv/ws"));
    cfg.global = true;
    let cmd = remote_serve_command(&cfg);
    assert!(cmd.contains("--global"), "{cmd}");
    assert!(cmd.contains("--root '/srv/ws'"), "{cmd}");
}

#[test]
fn shell_quote_escapes_embedded_single_quotes() {
    assert_eq!(shell_quote("plain"), "'plain'");
    assert_eq!(shell_quote("a'b"), "'a'\\''b'");
}

// ── probe (attach) argument construction ──────────────────────────────────

#[test]
fn build_probe_ssh_args_has_no_remote_command() {
    // A probe forward must never invoke anything remotely (`-N`, and no
    // trailing command arg) — that is what keeps attaching to an
    // already-running remote server from ever starting a second one, and
    // keeps disconnecting from it from ever touching that process.
    let got = build_probe_ssh_args(&args("box", 7878, None), 9999);
    assert_eq!(
        got,
        vec![
            "-N".to_string(),
            "-o".to_string(),
            "ExitOnForwardFailure=yes".to_string(),
            "-L".to_string(),
            "9999:localhost:7878".to_string(),
            "box".to_string(),
        ]
    );
    assert!(
        !got.iter().any(|a| a.contains("orbit web serve")),
        "probe args must not carry a remote command: {got:?}"
    );
}

#[test]
fn build_probe_ssh_args_forwards_distinct_local_and_remote_ports() {
    let got = build_probe_ssh_args(&args("user@host", 9000, None), 7000);
    assert!(got.contains(&"7000:localhost:9000".to_string()));
}

// ── readiness polling (attach vs. spawn decision) ─────────────────────────

/// Spawn a background thread that accepts one connection on `listener` and
/// replies with a 200 `/healthz` response, simulating an already-running
/// remote dashboard answering through the forward.
fn serve_one_healthz_ok(listener: TcpListener) {
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 256];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 0\r\n\r\n");
        }
    });
}

/// A `sleep`-backed stand-in for a live `ssh` child: alive but never exits on
/// its own within a test's lifetime.
fn fake_live_tunnel() -> SshTunnel {
    let child = Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn sleep");
    SshTunnel::new(child)
}

#[test]
fn poll_until_ready_true_when_something_already_answers() {
    // This is the "attach" path: a forward with nothing spawned on our side,
    // but something already listening behind it.
    let port = ephemeral_port().expect("ephemeral port");
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).expect("bind");
    serve_one_healthz_ok(listener);

    let mut tunnel = fake_live_tunnel();
    let ready =
        poll_until_ready(&mut tunnel, port, Duration::from_secs(2)).expect("poll should not error");
    assert!(ready, "must report ready once /healthz answers 200");
}

#[test]
fn poll_until_ready_false_on_timeout_when_nothing_answers() {
    // This is the "nothing running yet" path: the forward is up (tunnel still
    // alive) but nobody answers /healthz, so the caller must fall back to
    // spawning a remote server rather than erroring outright.
    let port = ephemeral_port().expect("ephemeral port — left unbound");

    let mut tunnel = fake_live_tunnel();
    let ready = poll_until_ready(&mut tunnel, port, Duration::from_millis(600))
        .expect("timeout is not an error");
    assert!(!ready, "must report not-ready when nothing answers");
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
    let port = ephemeral_port().expect("ephemeral port");
    let child = Command::new("false").spawn().expect("spawn false");
    let mut tunnel = SshTunnel::new(child);

    let err = poll_until_ready(&mut tunnel, port, Duration::from_secs(2))
        .expect_err("an early ssh exit must be an error, not a timeout");
    assert!(matches!(err, OrbitError::Execution(_)));
}

// ── ssh exit classification ───────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn classify_exit_127_points_at_remote_path() {
    use super::super::connect::classify_ssh_exit;
    use std::os::unix::process::ExitStatusExt;

    // waitpid encodes a normal exit code in the high byte.
    let err = classify_ssh_exit(std::process::ExitStatus::from_raw(127 << 8));
    match err {
        OrbitError::Execution(msg) => assert!(msg.contains("PATH"), "got: {msg}"),
        other => panic!("expected Execution, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn classify_exit_255_points_at_ssh_connect() {
    use super::super::connect::classify_ssh_exit;
    use std::os::unix::process::ExitStatusExt;

    let err = classify_ssh_exit(std::process::ExitStatus::from_raw(255 << 8));
    match err {
        OrbitError::Execution(msg) => assert!(msg.contains("connect"), "got: {msg}"),
        other => panic!("expected Execution, got {other:?}"),
    }
}

// ── teardown ──────────────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn terminate_child_stops_a_running_process() {
    use super::super::connect::terminate_child;

    let mut child = Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn sleep");
    assert!(
        child.try_wait().expect("try_wait").is_none(),
        "sleep should still be running before teardown"
    );

    terminate_child(&mut child);

    // terminate_child waits internally, so the exit status is now available.
    assert!(
        child.try_wait().expect("try_wait").is_some(),
        "sleep should be terminated after teardown"
    );
}

#[cfg(unix)]
#[test]
fn tunnel_drop_reaps_child() {
    use super::super::connect::SshTunnel;

    let child = Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("spawn sleep");
    let pid = child.id() as libc::pid_t;

    {
        let _tunnel = SshTunnel::new(child);
    } // Drop tears the tunnel down here.

    // The child is signalled and reaped, so it no longer exists. `kill(pid, 0)`
    // returns -1/ESRCH for an absent process. (A pid-reuse race in this window
    // is vanishingly unlikely for a just-spawned `sleep`.)
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    assert!(!alive, "ssh child must be gone after the tunnel drops");
}
