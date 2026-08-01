use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use orbit_common::types::ExecutorSandboxKind;
use orbit_common::utility::redaction::non_sensitive_env_vars;
use orbit_exec::{
    BwrapProbeOutcome, LinuxBwrapSpawnRequest, MacosSandboxSpawnRequest, compile_linux_bwrap_argv,
    compile_macos_sandbox_profile, probe_bwrap, sandbox_exec_available,
    sandbox_exec_unavailable_message, spawn_under_linux_bwrap, spawn_under_macos_sandbox,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SandboxDispatchMetadata {
    pub(super) backend: Option<String>,
    pub(super) trusted_wrapper: Option<String>,
    pub(super) probe_outcome: Option<String>,
    pub(super) write_enforcement: String,
    pub(super) read_enforcement: String,
}

pub(super) struct PreparedSandbox<'a> {
    pub(super) effective: Option<&'a ResolvedSandbox>,
    pub(super) metadata: SandboxDispatchMetadata,
}

/// Resolve availability before provider argv construction. This ordering is
/// security-sensitive: provider-native flags are neutralized only when the
/// outer wrapper is actually usable, while an explicitly allowed bare
/// fallback keeps those flags intact.
pub(super) fn prepare_sandbox_for_dispatch(
    sandbox: Option<&ResolvedSandbox>,
) -> Result<PreparedSandbox<'_>, SpawnError> {
    match sandbox {
        Some(sandbox) if sandbox.kind == ExecutorSandboxKind::LinuxBwrap => {
            let probe = probe_bwrap();
            prepare_linux_sandbox_for_dispatch_with_probe(sandbox, probe)
        }
        Some(sandbox) => Ok(PreparedSandbox {
            effective: Some(sandbox),
            metadata: SandboxDispatchMetadata {
                backend: Some(sandbox.kind.as_str().to_string()),
                trusted_wrapper: None,
                probe_outcome: None,
                write_enforcement: "write_enforced".to_string(),
                read_enforcement: "read_delegated".to_string(),
            },
        }),
        None => Ok(PreparedSandbox {
            effective: None,
            metadata: SandboxDispatchMetadata {
                backend: None,
                trusted_wrapper: None,
                probe_outcome: None,
                write_enforcement: "write_delegated".to_string(),
                read_enforcement: "read_delegated".to_string(),
            },
        }),
    }
}

pub(crate) fn prepare_linux_sandbox_for_dispatch_with_probe<'a>(
    sandbox: &'a ResolvedSandbox,
    probe: BwrapProbeOutcome,
) -> Result<PreparedSandbox<'a>, SpawnError> {
    if probe.available {
        Ok(PreparedSandbox {
            effective: Some(sandbox),
            metadata: SandboxDispatchMetadata {
                backend: Some("linux-bwrap".to_string()),
                trusted_wrapper: Some(probe.trusted_path),
                probe_outcome: Some(probe.detail),
                write_enforcement: "write_enforced".to_string(),
                read_enforcement: "read_delegated".to_string(),
            },
        })
    } else if sandbox.allow_fallback {
        tracing::warn!(
            target: "orbit.engine.cli_runner",
            reason = %probe.detail,
            "linux-bwrap unavailable; falling back to bare exec because executor declares allow_fallback"
        );
        Ok(PreparedSandbox {
            effective: None,
            metadata: SandboxDispatchMetadata {
                backend: Some("bare-fallback".to_string()),
                trusted_wrapper: Some(probe.trusted_path),
                probe_outcome: Some(probe.detail),
                write_enforcement: "write_delegated".to_string(),
                read_enforcement: "read_delegated".to_string(),
            },
        })
    } else {
        Err(SpawnError::permanent(format!(
            "{}; declare allow_fallback: true to permit bare exec",
            probe.detail
        )))
    }
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

/// Resolve a provider launcher without relying solely on the parent process's
/// ambient `PATH`.
///
/// ADR-0259: every CLI-backed provider enters through this resolver so service,
/// routine, dashboard, and interactive dispatches use the same lookup policy.
pub(crate) fn resolve_provider_launcher(
    provider: &str,
    program: &str,
    cwd: Option<&Path>,
) -> Result<String, SpawnError> {
    let path = std::env::var_os("PATH");
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_provider_launcher_with(provider, program, path.as_deref(), home.as_deref(), cwd)
}

// pub(crate) widened for sibling tests under the repository's enforced test layout.
pub(crate) fn resolve_provider_launcher_with(
    provider: &str,
    program: &str,
    path: Option<&OsStr>,
    home: Option<&Path>,
    cwd: Option<&Path>,
) -> Result<String, SpawnError> {
    let configured = Path::new(program);
    if configured.components().count() > 1 {
        return Ok(program.to_string());
    }

    let mut search_dirs = Vec::new();
    let mut seen = HashSet::new();
    if let Some(path) = path {
        for dir in std::env::split_paths(path) {
            let dir = if dir.is_relative() {
                cwd.map_or(dir.clone(), |cwd| cwd.join(&dir))
            } else {
                dir
            };
            if seen.insert(dir.clone()) {
                search_dirs.push(dir);
            }
        }
    }
    if let Some(home) = home {
        for relative in [".local/bin", ".orbit/bin", ".cargo/bin", "bin"] {
            let dir = home.join(relative);
            if seen.insert(dir.clone()) {
                search_dirs.push(dir);
            }
        }
    }

    let mut searched = Vec::with_capacity(search_dirs.len());
    for dir in search_dirs {
        let candidate = dir.join(program);
        searched.push(candidate.clone());
        if candidate.is_file() {
            return Ok(candidate.to_string_lossy().into_owned());
        }
        #[cfg(windows)]
        if configured.extension().is_none() {
            for extension in windows_executable_extensions() {
                let candidate = dir.join(format!("{program}{extension}"));
                searched.push(candidate.clone());
                if candidate.is_file() {
                    return Ok(candidate.to_string_lossy().into_owned());
                }
            }
        }
    }

    let searched = if searched.is_empty() {
        "<no PATH or HOME search locations available>".to_string()
    } else {
        searched
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    Err(SpawnError::permanent(format!(
        "provider launcher `{program}` for provider `{provider}` was not found; searched: {searched}"
    )))
}

#[cfg(windows)]
fn windows_executable_extensions() -> Vec<String> {
    std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|extension| !extension.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
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
        Some(sb) if sb.kind == ExecutorSandboxKind::LinuxBwrap => {
            spawn_linux_bwrap(program, args, env, cwd, sb)
        }
        Some(sb) => Err(SpawnError::permanent(format!(
            "unsupported sandbox backend `{}`",
            sb.kind
        ))),
        None => spawn_bare(program, args, env, cwd),
    }
}

fn spawn_linux_bwrap(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&Path>,
    sandbox: &ResolvedSandbox,
) -> Result<SpawnedChild, SpawnError> {
    let plan = compile_linux_bwrap_argv(
        &sandbox.fs_profile,
        program,
        args,
        cwd,
        sandbox.managed_worktree,
    )
    .map_err(|error| SpawnError::permanent(error.to_string()))?;
    let child = spawn_under_linux_bwrap(LinuxBwrapSpawnRequest {
        plan: &plan,
        env,
        cwd,
        stdin: Stdio::piped(),
        stdout: Stdio::piped(),
        stderr: Stdio::piped(),
    })
    .map_err(|error| SpawnError::transient(error.to_string()))?;
    Ok(SpawnedChild {
        child,
        _profile_temp: None,
    })
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
