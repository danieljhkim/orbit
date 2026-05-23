#![allow(missing_docs)]

use std::ffi::OsString;

use orbit_common::types::OrbitError;
use tempfile::tempdir;

use super::super::super::dispatcher::ResolvedSandbox;
use super::super::spawn::{SpawnedChild, spawn_bare, spawn_macos_sandboxed_with};
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
    match err {
        OrbitError::Execution(msg) => {
            assert!(
                msg.contains("trusted sandbox-exec not available at /usr/bin/sandbox-exec"),
                "unexpected error message: {msg}"
            );
            assert!(
                msg.contains("allow_fallback: true"),
                "error should describe fallback opt-in: {msg}"
            );
        }
        other => panic!("expected Execution error, got {other:?}"),
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
