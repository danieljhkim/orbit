use std::path::Path;
use std::path::PathBuf;

use orbit_common::types::{ExecutorSandboxKind, ResolvedFsProfile, UNRESTRICTED_FS_PROFILE};
use orbit_engine::RuntimeHost;
use orbit_engine::{DispatchError, ResolvedSandbox};

use crate::OrbitRuntime;

pub(crate) fn resolve_executor_sandbox(
    runtime: &OrbitRuntime,
    provider: &str,
    fs_profile: Option<&str>,
    subprocess_cwd: Option<&Path>,
) -> Result<Option<ResolvedSandbox>, DispatchError> {
    let executor = runtime.get_executor_def(provider).map_err(|err| {
        DispatchError::CliInvocationFailed(format!(
            "load executor `{provider}` for sandbox resolution: {err}"
        ))
    })?;
    let Some(executor) = executor else {
        return Ok(None);
    };
    let Some(kind) = executor.sandbox else {
        return Ok(None);
    };
    match kind {
        ExecutorSandboxKind::MacosSandboxExec => {
            #[cfg(not(target_os = "macos"))]
            {
                Err(DispatchError::CliInvocationFailed(format!(
                    "executor `{provider}` declares sandbox `macos-sandbox-exec` but current platform is `{}`",
                    std::env::consts::OS
                )))
            }
            #[cfg(target_os = "macos")]
            {
                let mut resolved =
                    resolve_fs_profile_absolute(runtime, fs_profile, None).map_err(|err| {
                        DispatchError::CliInvocationFailed(format!(
                            "resolve fsProfile for sandbox: {err}"
                        ))
                    })?;
                append_codex_side_write_roots(runtime, provider, &mut resolved)?;
                append_orbit_child_runtime_write_roots(runtime, &mut resolved);
                append_active_worktree_root(runtime, subprocess_cwd, &mut resolved);
                Ok(Some(ResolvedSandbox {
                    kind,
                    fs_profile: resolved,
                    allow_fallback: executor.allow_fallback,
                    managed_worktree: false,
                }))
            }
        }
        ExecutorSandboxKind::LinuxBwrap => {
            #[cfg(not(target_os = "linux"))]
            {
                Err(DispatchError::CliInvocationFailed(format!(
                    "executor `{provider}` declares sandbox `linux-bwrap` but current platform is `{}`",
                    std::env::consts::OS
                )))
            }
            #[cfg(target_os = "linux")]
            {
                let mut resolved = resolve_fs_profile_absolute(runtime, fs_profile, subprocess_cwd)
                    .map_err(|err| {
                        DispatchError::CliInvocationFailed(format!(
                            "resolve fsProfile for linux-bwrap: {err}"
                        ))
                    })?;
                // Runtime and provider conveniences must not turn an activity
                // profile with an empty modify surface into a workspace writer.
                // Besides violating that profile, workspace re-allows make a
                // direct Bubblewrap invocation unable to enforce global
                // non-subtree denies such as `**/.env` for paths created after
                // spawn. Provider state and global Orbit runtime roots remain
                // available below; neither overlaps workspace-relative denies.
                let grants_workspace_modify =
                    resolved.modify.iter().any(|rule| !rule.starts_with('!'));
                if grants_workspace_modify {
                    append_codex_side_write_roots(runtime, provider, &mut resolved)?;
                }
                append_linux_runtime_write_roots(
                    runtime,
                    subprocess_cwd,
                    grants_workspace_modify,
                    &mut resolved,
                )?;
                append_linux_provider_state_roots(&mut resolved)?;
                let managed_worktree = subprocess_cwd
                    .and_then(|cwd| active_worktree_subpath(runtime, cwd))
                    .is_some();
                Ok(Some(ResolvedSandbox {
                    kind,
                    fs_profile: resolved,
                    allow_fallback: executor.allow_fallback,
                    managed_worktree,
                }))
            }
        }
    }
}

/// Resolve the activity's fsProfile against the active policy, then expand
/// every workspace-relative `read` / `modify` rule to an absolute path under
/// the workspace root. The kernel's `subpath` predicate is meaningless for
/// relative paths, so this is the layer that turns Orbit's policy into a
/// payload `sandbox-exec` can enforce.
fn resolve_fs_profile_absolute(
    runtime: &OrbitRuntime,
    fs_profile: Option<&str>,
    workspace_override: Option<&Path>,
) -> Result<ResolvedFsProfile, orbit_common::types::OrbitError> {
    let profile_name = fs_profile.unwrap_or(UNRESTRICTED_FS_PROFILE);
    let resolved = runtime
        .policy_engine()
        .def()
        .effective_profile(profile_name)?;
    let workspace_root = workspace_override
        .unwrap_or(&runtime.paths().repo_root)
        .canonicalize()
        .unwrap_or_else(|_| {
            workspace_override.map_or_else(
                || runtime.paths().repo_root.clone(),
                std::path::Path::to_path_buf,
            )
        });
    let workspace_str = workspace_root.display().to_string();

    Ok(ResolvedFsProfile {
        name: resolved.name,
        read: resolved
            .read
            .into_iter()
            .map(|rule| absolutize_rule(&workspace_str, &rule))
            .collect(),
        modify: resolved
            .modify
            .into_iter()
            .map(|rule| absolutize_rule(&workspace_str, &rule))
            .collect(),
    })
}

fn append_codex_side_write_roots(
    runtime: &OrbitRuntime,
    provider: &str,
    resolved: &mut ResolvedFsProfile,
) -> Result<(), DispatchError> {
    // Codex is the only `backend: cli` provider that ships its own writable
    // root surface (`--add-dir` fed from `writable_dirs_json`). Claude and
    // Gemini have no analogous CLI flag — their startup-time writes are
    // confined to their state directories, which `compile_macos_sandbox_profile`
    // already grants via the per-provider state-dir allowances. If a future
    // provider gains a side-root surface, add a sibling appender. See
    // T20260428-14.
    if provider != "codex" {
        return Ok(());
    }

    let config = RuntimeHost::agent_provider_config(runtime);
    let Some(raw_dirs) = config.get("writable_dirs_json") else {
        return Ok(());
    };
    let writable_dirs: Vec<String> = serde_json::from_str(raw_dirs).map_err(|err| {
        DispatchError::CliInvocationFailed(format!(
            "parse codex writable_dirs_json for sandbox: {err}"
        ))
    })?;
    if writable_dirs.is_empty() {
        return Ok(());
    }

    let workspace_root = runtime
        .paths()
        .repo_root
        .canonicalize()
        .unwrap_or_else(|_| runtime.paths().repo_root.clone());
    let workspace_str = workspace_root.display().to_string();
    for dir in writable_dirs {
        let Some(root) = absolutize_side_write_root(&workspace_str, &dir) else {
            continue;
        };
        // Append even when the root already appears earlier: SBPL is
        // last-match-wins, and these host-owned roots must land after
        // policy-derived denies such as `.orbit/**`.
        resolved.modify.push(root);
    }
    Ok(())
}

/// Allow the nested Orbit processes launched by provider CLIs to initialize
/// only the runtime stores they need while staying inside the outer sandbox.
///
/// Gemini and Claude do not have a codex-style `--add-dir` side channel, but
/// their MCP/tool calls still execute `orbit ...` as a sandbox-inherited child.
/// Those child processes initialize global logs/databases/tasks plus the
/// workspace stores exposed by activity tool allowlists.
///
/// Inventory boundary: this list follows currently activity-exposed Orbit write
/// tools. Registered-but-not-exposed stores such as ADRs and graph write roots
/// stay denied until the corresponding tools are added to those activity
/// allowlists. Keep the grants path-shaped instead of re-allowing the whole
/// home directory or workspace `.orbit` tree.
#[cfg(target_os = "macos")]
fn append_orbit_child_runtime_write_roots(
    runtime: &OrbitRuntime,
    resolved: &mut ResolvedFsProfile,
) {
    let global_root = runtime
        .paths()
        .global_dir
        .canonicalize()
        .unwrap_or_else(|_| runtime.paths().global_dir.clone());
    let global = global_root.display().to_string();

    let workspace_orbit = runtime
        .paths()
        .orbit_dir
        .canonicalize()
        .unwrap_or_else(|_| runtime.paths().orbit_dir.clone());
    let workspace = workspace_orbit.display().to_string();

    for root in [
        format!("{global}/state/logs/**"),
        format!("{global}/orbit.db*"),
        format!("{global}/tasks/**"),
        format!("{workspace}/tasks/**"),
        format!("{workspace}/learnings/**"),
        format!("{workspace}/frictions/**"),
        format!("{workspace}/state/audit/**"),
        format!("{workspace}/state/.id_alloc.lock"),
        format!("{workspace}/state/logs/**"),
        format!("{workspace}/state/semantic.db*"),
    ] {
        append_unique_modify_root(resolved, root);
    }
}

fn append_unique_modify_root(resolved: &mut ResolvedFsProfile, root: String) {
    if !resolved.modify.iter().any(|entry| entry == &root) {
        resolved.modify.push(root);
    }
}

#[cfg(target_os = "linux")]
fn append_linux_runtime_write_roots(
    runtime: &OrbitRuntime,
    _subprocess_cwd: Option<&Path>,
    grants_workspace_modify: bool,
    resolved: &mut ResolvedFsProfile,
) -> Result<(), DispatchError> {
    let global = runtime
        .paths()
        .global_dir
        .canonicalize()
        .unwrap_or_else(|_| runtime.paths().global_dir.clone());
    let workspace = runtime
        .paths()
        .orbit_dir
        .canonicalize()
        .unwrap_or_else(|_| runtime.paths().orbit_dir.clone());

    for directory in [global.join("state/logs"), global.join("tasks")] {
        ensure_owned_directory(&directory)?;
        append_unique_modify_root(resolved, directory.display().to_string());
    }
    for file in [
        global.join("orbit.db"),
        global.join("orbit.db-wal"),
        global.join("orbit.db-shm"),
    ] {
        if file.exists() {
            append_unique_modify_root(resolved, file.display().to_string());
        }
    }

    if !grants_workspace_modify {
        return Ok(());
    }

    for directory in [
        workspace.join("tasks"),
        workspace.join("learnings"),
        workspace.join("frictions"),
        workspace.join("state/audit"),
        workspace.join("state/logs"),
        workspace.join("state/job-runs"),
    ] {
        ensure_owned_directory(&directory)?;
        append_unique_modify_root(resolved, directory.display().to_string());
    }
    for file in [
        workspace.join("state/.id_alloc.lock"),
        workspace.join("state/semantic.db"),
        workspace.join("state/semantic.db-wal"),
        workspace.join("state/semantic.db-shm"),
    ] {
        if file.exists() {
            append_unique_modify_root(resolved, file.display().to_string());
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn append_linux_provider_state_roots(
    resolved: &mut ResolvedFsProfile,
) -> Result<(), DispatchError> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut directories = Vec::new();
    if let Some(path) = std::env::var_os("CODEX_HOME").map(PathBuf::from) {
        directories.push(path);
    } else if let Some(home) = &home {
        directories.push(home.join(".codex"));
    }
    if let Some(path) = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from) {
        directories.push(path);
    } else if let Some(home) = &home {
        directories.push(home.join(".claude"));
    }
    if let Some(home) = &home {
        directories.push(home.join(".gemini"));
        directories.push(home.join(".grok"));
    }
    for directory in directories {
        ensure_owned_directory(&directory)?;
        let canonical = directory.canonicalize().map_err(|error| {
            DispatchError::CliInvocationPermanent(format!(
                "canonicalize Linux provider state root `{}`: {error}",
                directory.display()
            ))
        })?;
        append_unique_modify_root(resolved, canonical.display().to_string());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn ensure_owned_directory(path: &Path) -> Result<(), DispatchError> {
    std::fs::create_dir_all(path).map_err(|error| {
        DispatchError::CliInvocationPermanent(format!(
            "create Linux sandbox runtime root `{}`: {error}",
            path.display()
        ))
    })
}

/// Re-allow the active job-run worktree under `<workspace>/.orbit/state/worktrees/`
/// for every provider, after the policy's `denyModify .orbit/**` rule. Without
/// this, `task_pr_pipeline` runs whose subprocess cwd lives under
/// `.orbit/state/worktrees/orbit-jrun-…` cannot edit their own checkout under
/// the macOS sandbox: SBPL is last-match-wins, the broad `unrestricted` profile
/// allows `<workspace>/**` first, the global deny appends `!<workspace>/.orbit/**`
/// last, and codex was the only provider that re-asserted a writable side-root
/// after that. See T20260508-17.
///
/// Scope is deliberately narrow: only the calling subprocess's cwd is
/// re-allowed, and only when it canonicalizes to a direct child of
/// `<workspace>/.orbit/state/worktrees/`. Cwds outside that prefix yield no
/// change — we do not blanket-reallow `.orbit/**` for non-codex providers.
#[cfg(target_os = "macos")]
fn append_active_worktree_root(
    runtime: &OrbitRuntime,
    subprocess_cwd: Option<&Path>,
    resolved: &mut ResolvedFsProfile,
) {
    let Some(cwd) = subprocess_cwd else {
        return;
    };
    let Some(worktree_root) = active_worktree_subpath(runtime, cwd) else {
        return;
    };
    // Append after the policy denies; SBPL last-match-wins re-grants writes
    // inside the active worktree without widening any path outside it.
    resolved.modify.push(worktree_root);
}

fn active_worktree_subpath(runtime: &OrbitRuntime, subprocess_cwd: &Path) -> Option<String> {
    active_worktree_root(runtime, subprocess_cwd).map(|worktree| worktree.display().to_string())
}

fn active_worktree_root(runtime: &OrbitRuntime, subprocess_cwd: &Path) -> Option<PathBuf> {
    let cwd = subprocess_cwd
        .canonicalize()
        .unwrap_or_else(|_| subprocess_cwd.to_path_buf());
    let workspace_orbit = runtime
        .paths()
        .orbit_dir
        .canonicalize()
        .unwrap_or_else(|_| runtime.paths().orbit_dir.clone());
    let worktrees_root = workspace_orbit.join("state").join("worktrees");
    // Require the cwd to live strictly under `…/.orbit/state/worktrees/`.
    // A bare `worktrees` cwd would re-allow the entire registry; one path
    // segment deeper restricts the grant to a single jrun subtree.
    let relative = cwd.strip_prefix(&worktrees_root).ok()?;
    let mut components = relative.components();
    let first = components.next()?;
    Some(worktrees_root.join(first.as_os_str()))
}

fn absolutize_side_write_root(workspace_root: &str, path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let absolute = if PathBuf::from(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        let trimmed = trimmed.trim_start_matches("./");
        if trimmed.is_empty() || trimmed == "." {
            PathBuf::from(workspace_root)
        } else {
            PathBuf::from(workspace_root).join(trimmed)
        }
    };
    let normalized = absolute.canonicalize().unwrap_or(absolute);
    Some(normalized.display().to_string())
}

fn absolutize_rule(workspace_root: &str, rule: &str) -> String {
    let (negated, body) = rule
        .strip_prefix('!')
        .map(|rest| (true, rest))
        .unwrap_or((false, rule));
    let trimmed = body.trim_start_matches("./");
    let absolute = if PathBuf::from(trimmed).is_absolute() {
        trimmed.to_string()
    } else if trimmed.is_empty() || trimmed == "." {
        workspace_root.to_string()
    } else {
        format!("{}/{}", workspace_root.trim_end_matches('/'), trimmed)
    };
    if negated {
        format!("!{absolute}")
    } else {
        absolute
    }
}
