#![allow(missing_docs)]
// [ORB-10009] Fixture setup uses unwrap/expect for readability.
#![allow(clippy::expect_used, clippy::unwrap_used)]
#![cfg(target_os = "linux")]

//! [ORB-10009] Linux-side contract tests for orbit-exec's sandbox surface.
//!
//! Orbit has no Linux kernel sandbox: `sandbox-exec` (SBPL) is macOS-only,
//! and no Landlock/seccomp/namespace confinement exists yet. These tests
//! pin the two halves of that contract on a real Linux host:
//!
//! 1. **Fail closed** — the macOS sandbox primitives must refuse to run
//!    (never silently spawn an *unsandboxed* child) when `sandbox-exec` is
//!    unavailable. `orbit-core`'s dispatcher-level twin
//!    (`resolve_executor_sandbox_errors_on_non_macos_platform`) covers the
//!    executor path.
//! 2. **Honesty tripwire** — `run_process` + `NoSandbox` provides *zero*
//!    kernel-level filesystem confinement on Linux; the policy layer
//!    (`PolicyEngine::check_resolved`, [ORB-00418]) is the only enforcement
//!    seam and is therefore load-bearing. If Linux sandboxing (e.g.
//!    Landlock) is ever added, this test flips and forces the expectations
//!    — and the CI story — to be updated together. Gate any future
//!    kernel-primitive tests on runtime feature detection (probe, then
//!    skip-with-message), not compile-time cfg alone.

use std::process::Stdio;

use orbit_exec::{
    EnvironmentMode, ExecRequest, MacosSandboxSpawnRequest, NoSandbox, StdinMode, run_process,
    sandbox_exec_available, sandbox_exec_unavailable_message, spawn_under_macos_sandbox,
};

/// Skip-with-message helper for host prerequisites (mirrors the
/// `sandbox_exec_can_apply()` self-skip pattern on the macOS leg).
#[allow(clippy::print_stderr)]
fn host_has(path: &str) -> bool {
    let present = std::path::Path::new(path).exists();
    if !present {
        eprintln!("skipping: `{path}` not present on this host");
    }
    present
}

#[test]
fn sandbox_exec_is_unavailable_on_linux() {
    assert!(
        !sandbox_exec_available(),
        "no trusted sandbox-exec should exist on Linux"
    );
    let message = sandbox_exec_unavailable_message();
    assert!(
        message.contains("sandbox-exec"),
        "unavailable message should name the missing wrapper: {message}"
    );
}

#[test]
fn spawn_under_macos_sandbox_fails_closed_on_linux() {
    // The spawn primitive must error out — not fall back to an unsandboxed
    // child — when the trusted wrapper is missing.
    let marker = tempfile::NamedTempFile::new().expect("marker file");
    let script = format!("echo escaped > {}", marker.path().display());
    let args = ["-c".to_string(), script];
    let result = spawn_under_macos_sandbox(MacosSandboxSpawnRequest {
        profile_text: "(version 1)\n(allow default)\n",
        program: "/bin/sh",
        args: &args,
        env: &[],
        cwd: None,
        stdin: Stdio::null(),
        stdout: Stdio::null(),
        stderr: Stdio::null(),
    });
    assert!(result.is_err(), "must fail closed without sandbox-exec");
    let content = std::fs::read_to_string(marker.path()).expect("read marker");
    assert!(
        content.is_empty(),
        "no child may run when the sandbox wrapper is unavailable; marker: {content:?}"
    );
}

/// NOTE: this documents *current* behavior, deliberately. A child spawned
/// through `run_process` on Linux can read any file its uid can — orbit-exec
/// applies no kernel confinement here. The tool/policy layer
/// (`orbit-tools` + `PolicyEngine::check_resolved`) is the only Linux
/// enforcement seam; its integration tests live in
/// `orbit-tools/tests/fs_enforcement_linux.rs`. If this test ever fails
/// because confinement appeared, that is a *feature* — rewrite the Linux
/// enforcement suite around the new primitive.
#[test]
fn run_process_applies_no_kernel_fs_confinement_on_linux() {
    if !host_has("/bin/cat") {
        return;
    }

    // A file well outside any conceivable workspace/profile allowance.
    let outside = tempfile::NamedTempFile::new().expect("outside file");
    std::fs::write(outside.path(), "outside-the-profile").expect("write outside file");

    let request = ExecRequest {
        program: "/bin/cat".to_string(),
        args: vec![outside.path().display().to_string()],
        current_dir: None,
        timeout_ms: Some(10_000),
        stdin_mode: StdinMode::Null,
        environment_mode: EnvironmentMode::Inherit,
        debug: false,
    };
    let result = run_process(&request, &NoSandbox).expect("spawn child");
    assert!(result.success, "stderr: {}", result.stderr);
    assert_eq!(
        result.stdout, "outside-the-profile",
        "child reads outside paths freely: policy-layer checks are the only Linux enforcement"
    );
}
