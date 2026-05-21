use std::path::Path;
use std::process::{Child, Command, Stdio};

use orbit_common::types::{ExecutorSandboxKind, OrbitError};
use orbit_exec::{
    MacosSandboxSpawnRequest, compile_macos_sandbox_profile, sandbox_exec_available,
    sandbox_exec_unavailable_message, spawn_under_macos_sandbox,
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

// pub(crate) widened for tests/ layout under ORB-00225; test reaches via exposed surface.
pub(crate) fn spawn_bare(
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
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
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
/// explicit availability flag instead of probing the trusted wrapper. Production
/// routes through the public wrapper which resolves the trusted absolute path; tests
/// can assert the fail-closed and fallback branches without mutating
/// process-global state.
// pub(crate) widened for tests/ layout under ORB-00225; test reaches via exposed surface.
pub(crate) fn spawn_macos_sandboxed_with(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&Path>,
    sandbox: &ResolvedSandbox,
    sandbox_exec_present: bool,
) -> Result<SpawnedChild, OrbitError> {
    if !sandbox_exec_present {
        let unavailable = sandbox_exec_unavailable_message();
        if sandbox.allow_fallback {
            tracing::warn!(
                target: "orbit.engine.cli_runner",
                program = program,
                "{unavailable}; falling back to bare exec because executor declares allow_fallback"
            );
            return spawn_bare(program, args, env, cwd);
        }
        return Err(OrbitError::Execution(format!(
            "{unavailable}; declare allow_fallback: true to permit bare exec"
        )));
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
