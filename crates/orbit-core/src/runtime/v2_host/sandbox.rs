use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;

use orbit_common::types::ExecutorSandboxKind;
#[cfg(target_os = "macos")]
use orbit_common::types::{ResolvedFsProfile, UNRESTRICTED_FS_PROFILE};
#[cfg(target_os = "macos")]
use orbit_engine::EnvironmentHost;
use orbit_engine::{DispatchError, ResolvedSandbox};

use crate::OrbitRuntime;

pub(super) fn resolve_executor_sandbox(
    runtime: &OrbitRuntime,
    provider: &str,
    #[cfg(target_os = "macos")] fs_profile: Option<&str>,
    #[cfg(not(target_os = "macos"))] _fs_profile: Option<&str>,
    #[cfg(target_os = "macos")] subprocess_cwd: Option<&Path>,
    #[cfg(not(target_os = "macos"))] _subprocess_cwd: Option<&Path>,
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
                    resolve_fs_profile_absolute(runtime, fs_profile).map_err(|err| {
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
#[cfg(target_os = "macos")]
fn resolve_fs_profile_absolute(
    runtime: &OrbitRuntime,
    fs_profile: Option<&str>,
) -> Result<ResolvedFsProfile, orbit_common::types::OrbitError> {
    let profile_name = fs_profile.unwrap_or(UNRESTRICTED_FS_PROFILE);
    let resolved = runtime
        .policy_engine()
        .def()
        .effective_profile(profile_name)?;
    let workspace_root = runtime
        .paths()
        .repo_root
        .canonicalize()
        .unwrap_or_else(|_| runtime.paths().repo_root.clone());
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

#[cfg(target_os = "macos")]
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

    let config = EnvironmentHost::agent_provider_config(runtime);
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

#[cfg(target_os = "macos")]
fn append_unique_modify_root(resolved: &mut ResolvedFsProfile, root: String) {
    if !resolved.modify.iter().any(|entry| entry == &root) {
        resolved.modify.push(root);
    }
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

#[cfg(target_os = "macos")]
fn active_worktree_subpath(runtime: &OrbitRuntime, subprocess_cwd: &Path) -> Option<String> {
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
    let worktree_dir = worktrees_root.join(first.as_os_str());
    Some(worktree_dir.display().to_string())
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
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
