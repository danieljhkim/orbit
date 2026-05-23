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

#[cfg(test)]
mod tests {
    use orbit_engine::V2RuntimeHost;

    use crate::runtime::v2_host::test_support::seeded_runtime_with_executor;
    #[cfg(target_os = "macos")]
    use crate::runtime::v2_host::test_support::{runtime_with_workspace_layout, seed_executor};

    #[test]
    fn resolve_executor_sandbox_returns_none_when_executor_has_no_sandbox() {
        let runtime = seeded_runtime_with_executor(None);
        let resolved = runtime
            .resolve_executor_sandbox("codex", None, None)
            .expect("resolve");
        assert!(resolved.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_executor_sandbox_returns_descriptor_with_absolutized_modify_paths() {
        let runtime = seeded_runtime_with_executor(Some(
            orbit_common::types::ExecutorSandboxKind::MacosSandboxExec,
        ));
        let resolved = runtime
            .resolve_executor_sandbox("codex", None, None)
            .expect("resolve")
            .expect("descriptor");
        assert_eq!(
            resolved.kind,
            orbit_common::types::ExecutorSandboxKind::MacosSandboxExec
        );
        let workspace_root = runtime
            .paths()
            .repo_root
            .canonicalize()
            .unwrap_or_else(|_| runtime.paths().repo_root.clone());
        let workspace_str = workspace_root.display().to_string();
        for entry in &resolved.fs_profile.modify {
            let body = entry.strip_prefix('!').unwrap_or(entry);
            assert!(
                body.starts_with('/') || body == workspace_str,
                "modify entry must be absolutized: {entry}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_executor_sandbox_appends_codex_side_write_roots_after_policy_denies() {
        let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
        seed_executor(
            &runtime,
            "codex",
            Some(orbit_common::types::ExecutorSandboxKind::MacosSandboxExec),
        );

        let resolved = runtime
            .resolve_executor_sandbox("codex", None, None)
            .expect("resolve")
            .expect("descriptor");
        let modify = &resolved.fs_profile.modify;
        let workspace_orbit = runtime
            .paths()
            .orbit_dir
            .canonicalize()
            .unwrap_or_else(|_| runtime.paths().orbit_dir.clone())
            .display()
            .to_string();
        let workspace_orbit_deny = format!("!{workspace_orbit}/**");
        let deny_pos = modify
            .iter()
            .position(|entry| entry == &workspace_orbit_deny)
            .unwrap_or_else(|| {
                panic!(
                    "default policy should deny workspace .orbit writes via {workspace_orbit_deny}; modify={modify:?}"
                )
            });
        let allow_pos = modify
            .iter()
            .rposition(|entry| entry == &workspace_orbit)
            .expect("codex side write root should re-allow workspace .orbit");

        assert!(
            deny_pos < allow_pos,
            "codex side write root must be appended after policy deny: {modify:?}"
        );
        let global_orbit = runtime
            .paths()
            .global_dir
            .canonicalize()
            .unwrap_or_else(|_| runtime.paths().global_dir.clone())
            .display()
            .to_string();
        assert!(
            modify.iter().any(|entry| entry == &global_orbit),
            "codex side write roots should include global .orbit: {modify:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_executor_sandbox_appends_gemini_orbit_runtime_roots_without_home_reallow() {
        let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
        seed_executor(
            &runtime,
            "gemini",
            Some(orbit_common::types::ExecutorSandboxKind::MacosSandboxExec),
        );

        let resolved = runtime
            .resolve_executor_sandbox("gemini", None, None)
            .expect("resolve")
            .expect("descriptor");
        let modify = &resolved.fs_profile.modify;
        let global = runtime
            .paths()
            .global_dir
            .canonicalize()
            .unwrap_or_else(|_| runtime.paths().global_dir.clone())
            .display()
            .to_string();
        let workspace_orbit = runtime
            .paths()
            .orbit_dir
            .canonicalize()
            .unwrap_or_else(|_| runtime.paths().orbit_dir.clone())
            .display()
            .to_string();
        let workspace_orbit_deny = format!("!{workspace_orbit}/**");
        let deny_pos = modify
            .iter()
            .position(|entry| entry == &workspace_orbit_deny)
            .unwrap_or_else(|| {
                panic!(
                    "default policy should deny workspace .orbit writes via {workspace_orbit_deny}; modify={modify:?}"
                )
            });
        let expected = [
            format!("{global}/state/logs/**"),
            format!("{global}/orbit.db*"),
            format!("{global}/tasks/**"),
            format!("{workspace_orbit}/tasks/**"),
            format!("{workspace_orbit}/learnings/**"),
            format!("{workspace_orbit}/frictions/**"),
            format!("{workspace_orbit}/state/audit/**"),
            format!("{workspace_orbit}/state/.id_alloc.lock"),
            format!("{workspace_orbit}/state/logs/**"),
            format!("{workspace_orbit}/state/semantic.db*"),
        ];
        for root in expected {
            let allow_pos = modify.iter().position(|entry| entry == &root);
            assert!(
                allow_pos.is_some(),
                "gemini sandbox should allow Orbit runtime root {root}; modify={modify:?}"
            );
            assert!(
                deny_pos < allow_pos.expect("root position checked above"),
                "Orbit runtime root {root} must be re-allowed after workspace .orbit deny: {modify:?}"
            );
        }
        assert!(
            !modify.iter().any(|entry| entry == &global),
            "gemini sandbox must not re-allow the whole global Orbit root: {modify:?}"
        );
        assert!(
            !modify.iter().any(|entry| entry == &workspace_orbit),
            "gemini sandbox must not re-allow the whole workspace .orbit root: {modify:?}"
        );
        // Registered-but-not-activity-exposed write stores are intentionally
        // outside this child-runtime inventory. If agent_implement, agent_review,
        // step_failure_recovery, or dispatch_agent exposes ADR or graph write
        // tools, revisit this allowlist alongside those surfaces.
        for excluded in [
            format!("{workspace_orbit}/adrs/**"),
            format!("{workspace_orbit}/knowledge/**"),
            format!("{workspace_orbit}/state/knowledge/**"),
        ] {
            assert!(
                !modify.iter().any(|entry| entry == &excluded),
                "gemini sandbox must not allow non-activity-exposed Orbit store {excluded}: {modify:?}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_executor_sandbox_appends_workspace_semantic_store_after_policy_deny() {
        let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
        seed_executor(
            &runtime,
            "gemini",
            Some(orbit_common::types::ExecutorSandboxKind::MacosSandboxExec),
        );

        let resolved = runtime
            .resolve_executor_sandbox("gemini", None, None)
            .expect("resolve")
            .expect("descriptor");
        let modify = &resolved.fs_profile.modify;
        let workspace_orbit = runtime
            .paths()
            .orbit_dir
            .canonicalize()
            .unwrap_or_else(|_| runtime.paths().orbit_dir.clone())
            .display()
            .to_string();
        let workspace_orbit_deny = format!("!{workspace_orbit}/**");
        let deny_pos = modify
            .iter()
            .position(|entry| entry == &workspace_orbit_deny)
            .unwrap_or_else(|| {
                panic!(
                    "default policy should deny workspace .orbit writes via {workspace_orbit_deny}; modify={modify:?}"
                )
            });
        let semantic_store = format!("{workspace_orbit}/state/semantic.db*");
        let allow_pos = modify
            .iter()
            .position(|entry| entry == &semantic_store)
            .unwrap_or_else(|| {
                panic!("semantic store should be re-allowed under sandbox: {modify:?}")
            });
        assert!(
            deny_pos < allow_pos,
            "semantic store re-allow must come after policy deny: {modify:?}"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn resolve_executor_sandbox_errors_on_non_macos_platform() {
        let runtime = seeded_runtime_with_executor(Some(
            orbit_common::types::ExecutorSandboxKind::MacosSandboxExec,
        ));
        let err = runtime
            .resolve_executor_sandbox("codex", None, None)
            .expect_err("expected platform-mismatch error");
        let message = format!("{err}");
        assert!(
            message.contains("macos-sandbox-exec"),
            "error must name the sandbox kind: {message}"
        );
    }

    /// Claude has no codex-style writable-dirs flag, so a worktree under
    /// `.orbit/state/worktrees/` was unwriteable under the macOS sandbox
    /// before T20260508-17. The host now appends the active worktree subpath
    /// after the policy deny so SBPL last-match-wins re-grants writes there.
    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_executor_sandbox_reallows_claude_active_worktree_under_orbit() {
        let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
        seed_executor(
            &runtime,
            "claude",
            Some(orbit_common::types::ExecutorSandboxKind::MacosSandboxExec),
        );

        let workspace_orbit = runtime
            .paths()
            .orbit_dir
            .canonicalize()
            .unwrap_or_else(|_| runtime.paths().orbit_dir.clone());
        let worktree = workspace_orbit
            .join("state")
            .join("worktrees")
            .join("orbit-jrun-20260508-9999");
        std::fs::create_dir_all(&worktree).expect("create worktree");

        let resolved = runtime
            .resolve_executor_sandbox("claude", None, Some(&worktree))
            .expect("resolve")
            .expect("descriptor");
        let modify = &resolved.fs_profile.modify;
        let workspace_orbit_str = workspace_orbit.display().to_string();
        let workspace_orbit_deny = format!("!{workspace_orbit_str}/**");
        let deny_pos = modify
            .iter()
            .position(|entry| entry == &workspace_orbit_deny)
            .unwrap_or_else(|| {
                panic!(
                    "default policy should deny workspace .orbit writes via {workspace_orbit_deny}; modify={modify:?}"
                )
            });
        let worktree_str = worktree
            .canonicalize()
            .unwrap_or_else(|_| worktree.clone())
            .display()
            .to_string();
        let allow_pos = modify
            .iter()
            .rposition(|entry| entry == &worktree_str)
            .unwrap_or_else(|| {
                panic!(
                    "active worktree subpath should re-allow under sandbox: expected {worktree_str} in {modify:?}"
                )
            });
        assert!(
            deny_pos < allow_pos,
            "active worktree re-allow must come after policy deny: {modify:?}"
        );
    }

    /// Regression guard against a blanket reallow: when the cwd is NOT under
    /// `.orbit/state/worktrees/`, no extra modify entry should be appended for
    /// non-codex providers. Otherwise a misconfigured activity could quietly
    /// widen the sandbox.
    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_executor_sandbox_does_not_reallow_for_non_worktree_cwd() {
        let (_root, runtime, repo_root) = runtime_with_workspace_layout();
        seed_executor(
            &runtime,
            "claude",
            Some(orbit_common::types::ExecutorSandboxKind::MacosSandboxExec),
        );

        // Repo root is a sibling of `.orbit`, well outside the worktrees prefix.
        let resolved = runtime
            .resolve_executor_sandbox("claude", None, Some(&repo_root))
            .expect("resolve")
            .expect("descriptor");
        let modify = &resolved.fs_profile.modify;
        let workspace_orbit = runtime
            .paths()
            .orbit_dir
            .canonicalize()
            .unwrap_or_else(|_| runtime.paths().orbit_dir.clone())
            .display()
            .to_string();
        // No reallow of `<workspace>/.orbit` itself for non-codex providers.
        assert!(
            !modify.iter().any(|entry| entry == &workspace_orbit),
            "claude must not blanket-reallow workspace .orbit when cwd is outside worktrees: {modify:?}"
        );
        // No reallow rooted at `.orbit/state/worktrees` either.
        let worktrees_root = format!("{workspace_orbit}/state/worktrees");
        assert!(
            !modify
                .iter()
                .any(|entry| entry.strip_prefix('!').unwrap_or(entry) == worktrees_root.as_str()),
            "claude must not reallow the worktrees root directly: {modify:?}"
        );
    }

    /// A cwd that resolves exactly to `.orbit/state/worktrees/` (no specific
    /// jrun child) must not yield a grant — that would re-allow every worktree
    /// in the registry. Only one path segment deeper qualifies.
    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_executor_sandbox_rejects_bare_worktrees_root_cwd() {
        let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
        seed_executor(
            &runtime,
            "claude",
            Some(orbit_common::types::ExecutorSandboxKind::MacosSandboxExec),
        );
        let workspace_orbit = runtime
            .paths()
            .orbit_dir
            .canonicalize()
            .unwrap_or_else(|_| runtime.paths().orbit_dir.clone());
        let worktrees_root = workspace_orbit.join("state").join("worktrees");
        std::fs::create_dir_all(&worktrees_root).expect("create worktrees root");

        let resolved = runtime
            .resolve_executor_sandbox("claude", None, Some(&worktrees_root))
            .expect("resolve")
            .expect("descriptor");
        let modify = &resolved.fs_profile.modify;
        let worktrees_root_str = worktrees_root
            .canonicalize()
            .unwrap_or_else(|_| worktrees_root.clone())
            .display()
            .to_string();
        assert!(
            !modify.iter().any(|entry| entry == &worktrees_root_str),
            "bare worktrees-root cwd must not re-allow the registry: {modify:?}"
        );
    }
}
