use std::path::Path;
use std::process::{Child, Command, Stdio};

use orbit_common::types::{ExecutorSandboxKind, OrbitError};
use orbit_exec::{
    MacosSandboxSpawnRequest, compile_macos_sandbox_profile, sandbox_exec_available,
    spawn_under_macos_sandbox,
};
use tempfile::NamedTempFile;

use super::super::dispatcher::ResolvedSandbox;

#[derive(Debug)]
pub(super) struct SpawnedChild {
    pub(super) child: Child,
    /// Sandbox profile tempfile, if any. Held until the supervisor returns
    /// so the kernel can keep reading the SBPL profile while the child runs.
    pub(super) _profile_temp: Option<NamedTempFile>,
}

pub(super) fn spawn_child_with_optional_sandbox(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&Path>,
    sandbox: Option<&ResolvedSandbox>,
) -> Result<SpawnedChild, OrbitError> {
    match sandbox {
        Some(sb) if sb.kind == ExecutorSandboxKind::MacosSandboxExec => {
            spawn_macos_sandboxed(program, args, env, cwd, sb)
        }
        Some(_) | None => spawn_bare(program, args, env, cwd),
    }
}

fn spawn_bare(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&Path>,
) -> Result<SpawnedChild, OrbitError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = cwd {
        command.current_dir(path);
    }
    let child = command
        .spawn()
        .map_err(|err| OrbitError::Execution(format!("failed to spawn `{program}`: {err}")))?;
    Ok(SpawnedChild {
        child,
        _profile_temp: None,
    })
}

fn spawn_macos_sandboxed(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&Path>,
    sandbox: &ResolvedSandbox,
) -> Result<SpawnedChild, OrbitError> {
    spawn_macos_sandboxed_with(program, args, env, cwd, sandbox, sandbox_exec_available())
}

/// Test-friendly variant of [`spawn_macos_sandboxed`]: callers pass an
/// explicit availability flag instead of probing `PATH`. Production routes
/// through the public wrapper which always reads the live `PATH`; tests
/// can assert the fail-closed and fallback branches without mutating
/// process-global state.
fn spawn_macos_sandboxed_with(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&Path>,
    sandbox: &ResolvedSandbox,
    sandbox_exec_present: bool,
) -> Result<SpawnedChild, OrbitError> {
    if !sandbox_exec_present {
        if sandbox.allow_fallback {
            tracing::warn!(
                target: "orbit.engine.cli_runner",
                program = program,
                "sandbox-exec not available on PATH; falling back to bare exec because executor declares allow_fallback"
            );
            return spawn_bare(program, args, env, cwd);
        }
        return Err(OrbitError::Execution(
            "sandbox-exec not available on PATH; declare allow_fallback: true to permit bare exec"
                .to_string(),
        ));
    }

    // SBPL compilation happens at spawn time so the orbit-exec dependency
    // stays scoped to this crate. The host returns only a descriptor
    // (`fs_profile` + `kind` + `allow_fallback`) so orbit-core has no
    // direct edge to orbit-exec.
    let profile_text = compile_macos_sandbox_profile(&sandbox.fs_profile)?;
    let (child, profile_temp) = spawn_under_macos_sandbox(MacosSandboxSpawnRequest {
        profile_text: &profile_text,
        program,
        args,
        env,
        cwd,
        stdin: Stdio::piped(),
        stdout: Stdio::piped(),
        stderr: Stdio::piped(),
    })?;
    Ok(SpawnedChild {
        child,
        _profile_temp: Some(profile_temp),
    })
}

#[cfg(test)]
mod tests {
    use orbit_common::types::OrbitError;
    use tempfile::tempdir;

    use super::super::test_support::{sandbox_for_test, sh_args};
    use super::*;

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
    fn spawn_macos_sandboxed_returns_error_when_sandbox_exec_missing_and_fallback_disabled() {
        let sandbox = sandbox_for_test();
        let err = spawn_macos_sandboxed_with("/bin/sh", &[], &[], None, &sandbox, false)
            .expect_err("expected fallback-disabled error");
        match err {
            OrbitError::Execution(msg) => {
                assert!(
                    msg.contains("sandbox-exec not available"),
                    "unexpected error message: {msg}"
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
}
