use std::path::Path;
use std::process::{Child, Command, Stdio};

use orbit_common::types::ExecutorSandboxKind;
use orbit_common::utility::redaction::non_sensitive_env_vars;
use orbit_exec::{
    MacosSandboxSpawnRequest, compile_macos_sandbox_profile, sandbox_exec_available,
    sandbox_exec_unavailable_message, spawn_under_macos_sandbox,
};
use tempfile::NamedTempFile;

use super::super::dispatcher::ResolvedSandbox;

/// Typed spawn failure with a retryability classification (ORB-10006).
///
/// `permanent: true` marks failures that retrying cannot fix — the step
/// retry wrapper fails fast on them instead of burning attempts. Only
/// clearly-deterministic failures are classified permanent (executable
/// missing, permission denied, sandbox profile rejected); everything else
/// stays transient so the step-level retry keeps its pre-ORB-10006 reach.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct SpawnError {
    pub(crate) permanent: bool,
    pub(crate) message: String,
}

impl SpawnError {
    pub(crate) fn transient(message: String) -> Self {
        Self {
            permanent: false,
            message,
        }
    }

    pub(crate) fn permanent(message: String) -> Self {
        Self {
            permanent: true,
            message,
        }
    }

    /// Classify an OS spawn error. `NotFound` (ENOENT) and
    /// `PermissionDenied` (EACCES) are deterministic; resource-exhaustion
    /// signals (EAGAIN, ENOMEM, EMFILE, ENFILE, ...) and anything
    /// unrecognized stay transient — conservative in the direction of
    /// preserving retries.
    pub(crate) fn from_spawn_io(program: &str, err: &std::io::Error) -> Self {
        let message = format!("failed to spawn `{program}`: {err}");
        match err.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                Self::permanent(message)
            }
            _ => Self::transient(message),
        }
    }
}

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
) -> Result<SpawnedChild, SpawnError> {
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
) -> Result<SpawnedChild, SpawnError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(non_sensitive_env_vars())
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
        .map_err(|err| SpawnError::from_spawn_io(program, &err))?;
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
) -> Result<SpawnedChild, SpawnError> {
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
) -> Result<SpawnedChild, SpawnError> {
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
        // A missing trusted sandbox-exec binary won't appear between retry
        // attempts — deterministic environment failure.
        return Err(SpawnError::permanent(format!(
            "{unavailable}; declare allow_fallback: true to permit bare exec"
        )));
    }

    // SBPL compilation happens at spawn time so the orbit-exec dependency
    // stays scoped to this crate. The host returns only a descriptor
    // (`fs_profile` + `kind` + `allow_fallback`) so orbit-core has no
    // direct edge to orbit-exec.
    //
    // A profile that fails to compile is deterministic config — permanent.
    // The sandboxed spawn itself goes through orbit-exec, which erases the
    // io::ErrorKind; classify it transient so retries are preserved.
    let profile_text = compile_macos_sandbox_profile(&sandbox.fs_profile)
        .map_err(|err| SpawnError::permanent(err.to_string()))?;
    let (child, profile_temp) = spawn_under_macos_sandbox(MacosSandboxSpawnRequest {
        profile_text: &profile_text,
        program,
        args,
        env,
        cwd,
        stdin: Stdio::piped(),
        stdout: Stdio::piped(),
        stderr: Stdio::piped(),
    })
    .map_err(|err| SpawnError::transient(err.to_string()))?;
    Ok(SpawnedChild {
        child,
        _profile_temp: Some(profile_temp),
    })
}
