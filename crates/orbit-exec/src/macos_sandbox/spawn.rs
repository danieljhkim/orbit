use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use orbit_common::types::OrbitError;
use orbit_common::utility::redaction::non_sensitive_env_vars;
use tempfile::NamedTempFile;

const TRUSTED_SANDBOX_EXEC_PATHS: &[&str] = &["/usr/bin/sandbox-exec"];

pub struct MacosSandboxSpawnRequest<'a> {
    pub profile_text: &'a str,
    pub program: &'a str,
    pub args: &'a [String],
    pub env: &'a [(String, String)],
    pub cwd: Option<&'a Path>,
    pub stdin: Stdio,
    pub stdout: Stdio,
    pub stderr: Stdio,
}

pub fn spawn_under_macos_sandbox(
    request: MacosSandboxSpawnRequest<'_>,
) -> Result<(Child, NamedTempFile), OrbitError> {
    let MacosSandboxSpawnRequest {
        profile_text,
        program,
        args,
        env,
        cwd,
        stdin,
        stdout,
        stderr,
    } = request;

    let mut profile_file = tempfile::Builder::new()
        .prefix("orbit-sandbox-")
        .suffix(".sb")
        .tempfile()
        .map_err(|err| {
            OrbitError::Execution(format!("failed to create sandbox profile tempfile: {err}"))
        })?;
    use std::io::Write;
    profile_file
        .write_all(profile_text.as_bytes())
        .map_err(|err| {
            OrbitError::Execution(format!("failed to write sandbox profile tempfile: {err}"))
        })?;
    profile_file
        .flush()
        .map_err(|err| OrbitError::Execution(format!("failed to flush sandbox profile: {err}")))?;

    let profile_path = profile_file.path().to_path_buf();

    let sandbox_exec_path = sandbox_exec_path_or_error()?;
    let mut command = Command::new(&sandbox_exec_path);
    command
        .arg("-f")
        .arg(&profile_path)
        .arg(program)
        .args(args)
        .env_clear()
        .envs(non_sensitive_env_vars())
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr);
    if let Some(path) = cwd {
        command.current_dir(path);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let child = command.spawn().map_err(|err| {
        OrbitError::Execution(format!(
            "failed to spawn trusted sandbox-exec `{}` around `{program}`: {err}",
            sandbox_exec_path.display()
        ))
    })?;
    Ok((child, profile_file))
}

/// Returns the stable program path used in audit logs for sandboxed CLI
/// invocations. The real spawn path is resolved again at execution time so
/// missing binaries still fail closed.
pub fn sandbox_exec_program_for_audit() -> &'static str {
    TRUSTED_SANDBOX_EXEC_PATHS[0]
}

/// Returns `true` if a trusted absolute `sandbox-exec` binary is available.
pub fn sandbox_exec_available() -> bool {
    sandbox_exec_path().is_some()
}

/// Human-facing reason used when fail-closed sandboxing cannot find the
/// trusted wrapper.
pub fn sandbox_exec_unavailable_message() -> String {
    format!(
        "trusted sandbox-exec not available at {}",
        TRUSTED_SANDBOX_EXEC_PATHS.join(", ")
    )
}

/// Resolve `sandbox-exec` from trusted absolute locations only.
pub fn sandbox_exec_path() -> Option<PathBuf> {
    sandbox_exec_path_from(TRUSTED_SANDBOX_EXEC_PATHS.iter().map(Path::new))
}

fn sandbox_exec_path_or_error() -> Result<PathBuf, OrbitError> {
    sandbox_exec_path().ok_or_else(|| OrbitError::Execution(sandbox_exec_unavailable_message()))
}

// pub(crate) widened for sibling-layout tests in macos_sandbox/tests/spawn.rs (ORB-00241)
pub(crate) fn sandbox_exec_path_from<I, P>(candidates: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    candidates
        .into_iter()
        .map(|candidate| candidate.as_ref().to_path_buf())
        .find(|candidate| candidate.is_absolute() && is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && (meta.permissions().mode() & 0o111) != 0,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}
