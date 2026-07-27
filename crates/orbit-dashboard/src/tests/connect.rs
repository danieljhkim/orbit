//! Unit tests for `orbit web connect` helpers (port selection, ssh arg
//! construction, and tunnel teardown). No real `ssh` process is spawned.

use std::net::{Ipv4Addr, TcpListener};
use std::process::Command;

use orbit_core::OrbitError;

use super::super::DEFAULT_DASHBOARD_PORT;
use super::super::connect::{
    ConnectArgs, build_ssh_args, ephemeral_port, probe_bindable, remote_serve_command,
    select_local_port, shell_quote,
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
