#![allow(missing_docs)]

use std::ffi::OsString;

use tempfile::tempdir;

use super::super::super::dispatcher::ResolvedSandbox;
use super::super::spawn::{
    SpawnError, SpawnedChild, resolve_provider_launcher_with, spawn_bare,
    spawn_macos_sandboxed_with,
};
use super::test_support::{sandbox_for_test, sh_args};

#[test]
fn spawn_bare_runs_program_in_provided_cwd() {
    let temp = tempdir().expect("tempdir");
    let cwd = temp.path().canonicalize().expect("canonical tempdir");
    let SpawnedChild {
        child,
        _profile_temp,
    } = spawn_bare("/bin/sh", &sh_args("pwd"), &[], Some(&cwd)).expect("spawn succeeds");

    let output = child.wait_with_output().expect("wait succeeds");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout utf8"),
        format!("{}\n", cwd.display())
    );
}

#[test]
fn spawn_bare_does_not_inherit_ambient_sensitive_env() {
    let _guard = EnvVarGuard::set("ORBIT_SPAWN_BARE_TEST_TOKEN", "parent-process-secret-value");
    let SpawnedChild {
        child,
        _profile_temp,
    } = spawn_bare(
        "/bin/sh",
        &sh_args(
            "if [ -z \"${ORBIT_SPAWN_BARE_TEST_TOKEN+x}\" ]; then printf unset; else printf 'leaked:%s' \"$ORBIT_SPAWN_BARE_TEST_TOKEN\"; fi",
        ),
        &[],
        None,
    )
    .expect("spawn succeeds");

    let output = child.wait_with_output().expect("wait succeeds");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout utf8"),
        "unset"
    );
}

#[test]
fn spawn_macos_sandboxed_returns_error_when_sandbox_exec_missing_and_fallback_disabled() {
    let sandbox = sandbox_for_test();
    let err = spawn_macos_sandboxed_with("/bin/sh", &[], &[], None, &sandbox, false)
        .expect_err("expected fallback-disabled error");
    assert!(
        err.permanent,
        "missing sandbox-exec is deterministic and must classify permanent"
    );
    assert!(
        err.message
            .contains("trusted sandbox-exec not available at /usr/bin/sandbox-exec"),
        "unexpected error message: {}",
        err.message
    );
    assert!(
        err.message.contains("allow_fallback: true"),
        "error should describe fallback opt-in: {}",
        err.message
    );
}

#[test]
fn spawn_bare_missing_executable_classifies_permanent() {
    let err = spawn_bare("/nonexistent/orbit-test-program", &[], &[], None)
        .expect_err("missing executable must fail");
    assert!(
        err.permanent,
        "ENOENT is deterministic and must classify permanent: {}",
        err.message
    );
    assert!(
        err.message.contains("/nonexistent/orbit-test-program"),
        "error should name the program: {}",
        err.message
    );
}

#[test]
fn spawn_io_error_classification_table() {
    use std::io::{Error, ErrorKind};
    // (kind, expected permanent) — clearly-deterministic failures fail fast;
    // resource exhaustion and everything unrecognized stays retryable.
    let table = [
        (ErrorKind::NotFound, true),
        (ErrorKind::PermissionDenied, true),
        (ErrorKind::WouldBlock, false),  // EAGAIN
        (ErrorKind::OutOfMemory, false), // ENOMEM
        (ErrorKind::Interrupted, false), // EINTR
        (ErrorKind::Other, false),       // unknown → conservative: retryable
    ];
    for (kind, expect_permanent) in table {
        let classified = SpawnError::from_spawn_io("prog", &Error::new(kind, "boom"));
        assert_eq!(
            classified.permanent, expect_permanent,
            "kind {kind:?} misclassified (permanent={})",
            classified.permanent
        );
    }
}

#[test]
fn provider_launcher_resolution_falls_back_to_temp_home_with_scrubbed_path() {
    let temp = tempdir().expect("tempdir");
    let fake_path = temp.path().join("system-bin");
    let home = temp.path().join("home");
    let provider_bin = home.join(".local/bin");
    std::fs::create_dir_all(&fake_path).expect("create fake PATH");
    std::fs::create_dir_all(&provider_bin).expect("create provider bin");
    let launcher = provider_bin.join("claude");
    std::fs::write(&launcher, "#!/bin/sh\nexit 0\n").expect("write fake provider launcher");

    let resolved = resolve_provider_launcher_with(
        "claude",
        "claude",
        Some(fake_path.as_os_str()),
        Some(&home),
        None,
    )
    .expect("HOME fallback should resolve provider");

    assert_eq!(resolved, launcher.to_string_lossy());
}

#[test]
fn missing_provider_launcher_error_names_provider_and_searched_locations() {
    let temp = tempdir().expect("tempdir");
    let fake_path = temp.path().join("system-bin");
    let home = temp.path().join("home");
    std::fs::create_dir_all(&fake_path).expect("create fake PATH");
    std::fs::create_dir_all(&home).expect("create fake HOME");

    let error = resolve_provider_launcher_with(
        "claude",
        "claude",
        Some(fake_path.as_os_str()),
        Some(&home),
        None,
    )
    .expect_err("missing launcher must fail");

    assert!(error.permanent, "missing launcher must remain permanent");
    assert!(
        error.message.contains("provider `claude`"),
        "error should name the provider: {}",
        error.message
    );
    for searched in [
        fake_path.join("claude"),
        home.join(".local/bin/claude"),
        home.join(".orbit/bin/claude"),
        home.join(".cargo/bin/claude"),
        home.join("bin/claude"),
    ] {
        assert!(
            error.message.contains(&searched.display().to_string()),
            "error should name searched location {}: {}",
            searched.display(),
            error.message
        );
    }
}

#[test]
fn spawn_macos_sandboxed_falls_back_to_bare_exec_when_allow_fallback_set() {
    let sandbox = ResolvedSandbox {
        allow_fallback: true,
        ..sandbox_for_test()
    };
    let mut spawned = spawn_macos_sandboxed_with(
        "/bin/sh",
        &["-c".to_string(), "exit 0".to_string()],
        &[],
        None,
        &sandbox,
        false,
    )
    .expect("fallback should succeed");
    // The fallback path returns a SpawnedChild with no profile tempfile
    // because the sandbox-exec wrapper was bypassed.
    assert!(spawned._profile_temp.is_none());
    let _ = spawned.child.wait();
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: this test uses a dedicated variable name and restores the
        // previous value on drop.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: see EnvVarGuard::set.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
