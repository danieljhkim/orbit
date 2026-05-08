#[cfg(target_os = "macos")]
use std::path::PathBuf;

use orbit_common::types::ExecutorSandboxKind;
#[cfg(target_os = "macos")]
use orbit_common::types::{ResolvedFsProfile, UNRESTRICTED_FS_PROFILE};
#[cfg(target_os = "macos")]
use orbit_engine::EnvironmentHost;
use orbit_engine::activity_job::{DispatchError, ResolvedSandbox};

use crate::OrbitRuntime;

pub(super) fn resolve_executor_sandbox(
    runtime: &OrbitRuntime,
    provider: &str,
    #[cfg(target_os = "macos")] fs_profile: Option<&str>,
    #[cfg(not(target_os = "macos"))] _fs_profile: Option<&str>,
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
                append_provider_side_write_roots(runtime, provider, &mut resolved)?;
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
fn append_provider_side_write_roots(
    runtime: &OrbitRuntime,
    provider: &str,
    resolved: &mut ResolvedFsProfile,
) -> Result<(), DispatchError> {
    // Codex is the only `backend: cli` provider that ships its own writable
    // root surface (`--add-dir` fed from `writable_dirs_json`). Claude and
    // Gemini have no analogous CLI flag — their startup-time writes are
    // confined to their state directories, which `compile_macos_sandbox_profile`
    // already grants via the per-provider state-dir allowances. If a future
    // provider gains a side-root surface, generalize this branch rather than
    // duplicating it. See T20260428-14.
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
    use orbit_engine::activity_job::V2RuntimeHost;

    use crate::runtime::v2_host::test_support::seeded_runtime_with_executor;
    #[cfg(target_os = "macos")]
    use crate::runtime::v2_host::test_support::{runtime_with_workspace_layout, seed_executor};

    #[test]
    fn resolve_executor_sandbox_returns_none_when_executor_has_no_sandbox() {
        let runtime = seeded_runtime_with_executor(None);
        let resolved = runtime
            .resolve_executor_sandbox("codex", None)
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
            .resolve_executor_sandbox("codex", None)
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
            .resolve_executor_sandbox("codex", None)
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

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn resolve_executor_sandbox_errors_on_non_macos_platform() {
        let runtime = seeded_runtime_with_executor(Some(
            orbit_common::types::ExecutorSandboxKind::MacosSandboxExec,
        ));
        let err = runtime
            .resolve_executor_sandbox("codex", None)
            .expect_err("expected platform-mismatch error");
        let message = format!("{err}");
        assert!(
            message.contains("macos-sandbox-exec"),
            "error must name the sandbox kind: {message}"
        );
    }
}
