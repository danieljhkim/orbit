use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use orbit_common::types::OrbitError;
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

fn sandbox_exec_path_from<I, P>(candidates: I) -> Option<PathBuf>
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

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::super::test_support::{
        ScopeGuard, sandbox_exec_can_apply, sandbox_test_parent, shell_escape,
    };
    use super::*;
    #[test]
    fn sandbox_exec_path_from_uses_trusted_absolute_candidate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("sandbox-exec");
        std::fs::write(&bin, "#!/bin/sh\nexit 0\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("perms");
        }
        assert_eq!(sandbox_exec_path_from([bin.as_path()]), Some(bin));
    }

    #[test]
    fn sandbox_exec_path_from_rejects_relative_candidates() {
        let bin = Path::new("sandbox-exec");
        assert_eq!(sandbox_exec_path_from([bin]), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn spawn_under_macos_sandbox_ignores_fake_sandbox_exec_on_path() {
        if !sandbox_exec_can_apply() {
            return;
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let fake_dir = temp.path().join("fake-bin");
        std::fs::create_dir_all(&fake_dir).expect("fake dir");
        let marker = temp.path().join("fake-used");
        let fake = fake_dir.join("sandbox-exec");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\necho fake > {}\nexit 77\n",
                shell_escape(&marker)
            ),
        )
        .expect("write fake sandbox-exec");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
                .expect("fake perms");
        }

        let poisoned_path = format!("{}:/usr/bin:/bin", fake_dir.display());
        let args = ["-c".to_string(), "exit 0".to_string()];
        let env = [("PATH".to_string(), poisoned_path)];
        let (child, _profile_file) = spawn_under_macos_sandbox(MacosSandboxSpawnRequest {
            profile_text: "(version 1)\n(allow default)\n",
            program: "/bin/sh",
            args: &args,
            env: &env,
            cwd: None,
            stdin: Stdio::null(),
            stdout: Stdio::piped(),
            stderr: Stdio::piped(),
        })
        .expect("spawn sandboxed child");
        let output = child.wait_with_output().expect("wait for child");

        assert!(
            output.status.success(),
            "trusted sandbox-exec should run child despite fake PATH entry; stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !marker.exists(),
            "fake sandbox-exec on PATH should not have been executed"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn spawn_under_macos_sandbox_runs_program_in_provided_cwd() {
        if !sandbox_exec_can_apply() {
            return;
        }

        let parent = sandbox_test_parent("cwd");
        let _cleanup = ScopeGuard(parent.clone());
        let dir = tempfile::Builder::new()
            .prefix("sandbox-cwd-")
            .tempdir_in(&parent)
            .expect("cwd tempdir");
        let cwd = dir.path().canonicalize().expect("canonical cwd");
        let args = ["-c".to_string(), "pwd".to_string()];
        let (child, _profile_file) = spawn_under_macos_sandbox(MacosSandboxSpawnRequest {
            profile_text: "(version 1)\n(allow default)\n",
            program: "/bin/sh",
            args: &args,
            env: &[],
            cwd: Some(&cwd),
            stdin: Stdio::null(),
            stdout: Stdio::piped(),
            stderr: Stdio::piped(),
        })
        .expect("spawn sandboxed child");
        let output = child.wait_with_output().expect("wait for child");

        assert!(
            output.status.success(),
            "sandboxed pwd should succeed; stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("stdout utf8"),
            format!("{}\n", cwd.display())
        );
    }
}
