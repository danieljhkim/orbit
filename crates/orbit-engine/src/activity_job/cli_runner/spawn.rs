use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use orbit_common::types::{ExecutorSandboxKind, OrbitError, ResolvedFsProfile};
use orbit_common::utility::redaction::non_sensitive_env_vars;
use orbit_exec::{
    BwrapProbeOutcome, LinuxBwrapSpawnRequest, MacosSandboxSpawnRequest, UnsatisfiedWriteGrant,
    compile_linux_bwrap_argv, compile_macos_sandbox_profile, linux_bwrap_write_grant_diagnostic,
    prepare_linux_bwrap_write_grants, probe_bwrap, sandbox_exec_available,
    sandbox_exec_unavailable_message, spawn_under_linux_bwrap, spawn_under_macos_sandbox,
};
use tempfile::NamedTempFile;

use super::super::dispatcher::ResolvedSandbox;

const ORBIT_BIN_ENV: &str = "ORBIT_BIN";

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
/// Every CLI-backed provider enters through this resolver so service, routine,
/// dashboard, and interactive dispatches use the same lookup policy.
pub(crate) fn resolve_provider_launcher(
    provider: &str,
    program: &str,
    cwd: Option<&Path>,
) -> Result<String, SpawnError> {
    let path = std::env::var_os("PATH");
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_provider_launcher_with(provider, program, path.as_deref(), home.as_deref(), cwd)
}

/// Pin tools invoked by an agent to the Orbit build that dispatched it.
///
/// Long-lived services may retain a `PATH` whose first `orbit` is an older
/// Cargo install even after the operator deploys `~/.orbit/bin/orbit`. Export
/// the selected binary for hook scripts and put its directory first for bare
/// `orbit tool ...` invocations inside the provider (including Bubblewrap).
pub(crate) fn orbit_tool_env() -> Result<Vec<(String, String)>, SpawnError> {
    let current_exe = std::env::current_exe().map_err(|error| {
        SpawnError::permanent(format!(
            "resolve dispatching Orbit executable for agent tool environment: {error}"
        ))
    })?;
    let configured = std::env::var_os(ORBIT_BIN_ENV);
    let inherited_path = std::env::var_os("PATH");
    orbit_tool_env_with(
        configured.as_deref(),
        &current_exe,
        inherited_path.as_deref(),
    )
}

// pub(crate) widened for sibling tests under the repository's enforced test layout.
pub(crate) fn orbit_tool_env_with(
    configured: Option<&OsStr>,
    current_exe: &Path,
    inherited_path: Option<&OsStr>,
) -> Result<Vec<(String, String)>, SpawnError> {
    let selected = configured
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| current_exe.to_path_buf());
    let selected_text = selected.to_string_lossy().into_owned();

    let Some(bin_dir) = selected
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    else {
        return Ok(vec![(ORBIT_BIN_ENV.to_string(), selected_text)]);
    };

    let mut path_entries = vec![bin_dir.to_path_buf()];
    if let Some(inherited_path) = inherited_path {
        path_entries.extend(
            std::env::split_paths(inherited_path).filter(|entry| entry.as_path() != bin_dir),
        );
    }
    let pinned_path = std::env::join_paths(path_entries)
        .map_err(|error| {
            SpawnError::permanent(format!(
                "construct agent PATH pinned to `{}`: {error}",
                selected.display()
            ))
        })?
        .into_string()
        .map_err(|_| {
            SpawnError::permanent(format!(
                "agent PATH pinned to `{}` is not valid Unicode",
                selected.display()
            ))
        })?;

    Ok(vec![
        (ORBIT_BIN_ENV.to_string(), selected_text),
        ("PATH".to_string(), pinned_path),
    ])
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

/// Materialize the profile's narrow write grants, then compile argv.
///
/// Preparation happens here, at every spawn, rather than once during worktree
/// setup: this is the only layer that sees the *effective* profile — policy
/// rules absolutized against the subprocess cwd plus the host-appended run
/// roots — so it is the only layer whose grant set cannot drift from what the
/// kernel will enforce. Re-deriving per spawn is also what lets a run whose
/// needs grow mid-run pick up anchors on its next provider launch.
///
/// Anchors are only created inside the managed worktree, which is trusted and
/// disposable. Creating one grants nothing new: the effective profile already
/// decided the path is writable.
fn spawn_linux_bwrap(
    program: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&Path>,
    sandbox: &ResolvedSandbox,
) -> Result<SpawnedChild, SpawnError> {
    if let Some(worktree) = sandbox.managed_worktree.then_some(cwd).flatten() {
        let prepared = prepare_linux_bwrap_write_grants(&sandbox.fs_profile, worktree)
            .map_err(|error| SpawnError::permanent(error.to_string()))?;
        if !prepared.created.is_empty() {
            tracing::info!(
                target: "orbit.engine.cli_runner",
                anchors = ?prepared.created,
                "materialized policy-granted sandbox write anchors before launch"
            );
        }
        report_unsatisfied_grants(&prepared.unsatisfied);
    }
    let plan = compile_linux_bwrap_argv(
        &sandbox.fs_profile,
        program,
        args,
        cwd,
        sandbox.managed_worktree,
    )
    .map_err(|error| SpawnError::permanent(error.to_string()))?;
    // Inside a managed worktree, preparation should have satisfied every grant.
    // Anything still unmountable is a defect in the grant set, and failing here
    // — before the provider starts — keeps the denial attributable to a path
    // and a rule instead of surfacing as an EROFS mid-turn.
    if sandbox.managed_worktree && !plan.dropped_grants.is_empty() {
        return Err(SpawnError::permanent(format!(
            "linux-bwrap could not apply {} policy write grant(s): {}",
            plan.dropped_grants.len(),
            describe_grants(&plan.dropped_grants)
        )));
    }
    report_unsatisfied_grants(&plan.dropped_grants);
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

/// Host-owned anchors outside the managed worktree are the host's to create,
/// so a miss there is reported rather than fatal — but never silently.
fn report_unsatisfied_grants(grants: &[UnsatisfiedWriteGrant]) {
    if grants.is_empty() {
        return;
    }
    tracing::warn!(
        target: "orbit.engine.cli_runner",
        detail = %describe_grants(grants),
        "policy grants a sandbox write path that could not be mounted"
    );
}

fn describe_grants(grants: &[UnsatisfiedWriteGrant]) -> String {
    grants
        .iter()
        .map(UnsatisfiedWriteGrant::describe)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Turn a child-reported EROFS into a policy-owned denial when the failing
/// program included the attempted path in stderr. This runs after the real
/// Bubblewrap child exits, so it covers the production invocation boundary
/// rather than merely explaining a path supplied by a unit test.
pub(super) fn linux_bwrap_failed_write_diagnostic(
    profile: &ResolvedFsProfile,
    stderr: &[u8],
    cwd: Option<&Path>,
) -> Result<Option<String>, OrbitError> {
    let stderr = String::from_utf8_lossy(stderr);
    for line in stderr.lines().rev() {
        if !line.contains("Read-only file system") && !line.contains("EROFS") {
            continue;
        }
        for candidate in failed_write_path_candidates(line).into_iter().rev() {
            let path = Path::new(&candidate);
            let attempted = if path.is_absolute() {
                path.to_path_buf()
            } else if let Some(cwd) = cwd {
                cwd.join(path)
            } else {
                continue;
            };
            if let Some(diagnostic) = linux_bwrap_write_grant_diagnostic(profile, &attempted)? {
                return Ok(Some(format!(
                    "Orbit linux-bwrap policy denied the attempted write: {diagnostic}"
                )));
            }
        }
    }
    Ok(None)
}

fn failed_write_path_candidates(line: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for quote in ['\'', '"', '`'] {
        let mut remainder = line;
        while let Some(start) = remainder.find(quote) {
            let after_start = &remainder[start + quote.len_utf8()..];
            let Some(end) = after_start.find(quote) else {
                break;
            };
            let candidate = after_start[..end].trim();
            if !candidate.is_empty() {
                candidates.push(candidate.to_string());
            }
            remainder = &after_start[end + quote.len_utf8()..];
        }
    }

    // Coreutils quotes paths, but language runtimes often render
    // `...: /path: Read-only file system`. Keep a conservative token fallback
    // so those failures are attributable too.
    let prefix = line
        .split_once("Read-only file system")
        .or_else(|| line.split_once("EROFS"))
        .map_or(line, |(prefix, _)| prefix);
    if let Some(token) = prefix.split_whitespace().next_back() {
        let candidate = token
            .trim_matches(|character: char| {
                matches!(character, '\'' | '"' | '`' | ':' | '(' | ')' | '[' | ']')
            })
            .trim();
        if !candidate.is_empty() && !candidates.iter().any(|known| known == candidate) {
            candidates.push(candidate.to_string());
        }
    }
    candidates
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
