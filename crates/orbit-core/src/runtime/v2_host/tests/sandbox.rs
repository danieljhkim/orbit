use orbit_engine::RuntimeHost;
#[cfg(target_os = "linux")]
use orbit_exec::{compile_linux_bwrap_argv, prepare_linux_bwrap_write_grants};

use crate::runtime::v2_host::test_support::seeded_runtime_with_executor;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::runtime::v2_host::test_support::{runtime_with_workspace_layout, seed_executor};

#[test]
fn resolve_executor_sandbox_returns_none_when_executor_has_no_sandbox() {
    let runtime = seeded_runtime_with_executor(None);
    let resolved = runtime
        .resolve_executor_sandbox("codex", None, None)
        .expect("resolve");
    assert!(resolved.is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn resolve_executor_sandbox_returns_linux_descriptor_with_absolute_mounts() {
    let runtime =
        seeded_runtime_with_executor(Some(orbit_common::types::ExecutorSandboxKind::LinuxBwrap));
    let resolved = runtime
        .resolve_executor_sandbox("codex", None, None)
        .expect("resolve")
        .expect("descriptor");
    assert_eq!(
        resolved.kind,
        orbit_common::types::ExecutorSandboxKind::LinuxBwrap
    );
    assert!(!resolved.managed_worktree);
    for entry in &resolved.fs_profile.modify {
        let body = entry.strip_prefix('!').unwrap_or(entry);
        assert!(
            body.starts_with('/'),
            "linux-bwrap mount rule must be absolute: {entry}"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn direct_reviewer_profile_does_not_gain_workspace_runtime_writes() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    seed_executor(
        &runtime,
        "claude",
        Some(orbit_common::types::ExecutorSandboxKind::LinuxBwrap),
    );

    let reviewer = runtime
        .resolve_executor_sandbox("claude", Some("reviewer"), Some(&repo_root))
        .expect("resolve reviewer sandbox")
        .expect("descriptor");
    assert!(!reviewer.managed_worktree);
    let canonical_repo = repo_root.canonicalize().expect("canonical repo");
    assert!(
        reviewer
            .fs_profile
            .modify
            .iter()
            .filter(|rule| !rule.starts_with('!'))
            .all(|rule| !rule.starts_with(&canonical_repo.display().to_string())),
        "read-only activity profile must not gain workspace runtime writes: {:?}",
        reviewer.fs_profile.modify
    );
    assert!(
        reviewer
            .fs_profile
            .read
            .iter()
            .any(|rule| rule.starts_with('!') && rule.ends_with("/**/*.env")),
        "global denyRead rules must remain in the resolved profile: {:?}",
        reviewer.fs_profile.read
    );
    compile_linux_bwrap_argv(
        &reviewer.fs_profile,
        "/bin/true",
        &[],
        Some(&canonical_repo),
        reviewer.managed_worktree,
    )
    .expect("direct reviewer sandbox must compile with default dotenv denies");

    let writer = runtime
        .resolve_executor_sandbox("claude", None, Some(&repo_root))
        .expect("resolve write-capable sandbox")
        .expect("descriptor");
    let error = compile_linux_bwrap_argv(
        &writer.fs_profile,
        "/bin/true",
        &[],
        Some(&canonical_repo),
        writer.managed_worktree,
    )
    .expect_err("a direct write-capable sandbox must still fail closed");
    assert!(error.to_string().contains("non-subtree denyModify"));
}

#[cfg(target_os = "linux")]
#[test]
fn resolve_executor_sandbox_marks_only_specific_orbit_worktree_managed() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    seed_executor(
        &runtime,
        "claude",
        Some(orbit_common::types::ExecutorSandboxKind::LinuxBwrap),
    );
    let worktrees = runtime.paths().orbit_dir.join("state/worktrees");
    let worktree = worktrees.join("orbit-jrun-test");
    std::fs::create_dir_all(&worktree).expect("create worktree");

    let managed = runtime
        .resolve_executor_sandbox("claude", None, Some(&worktree))
        .expect("resolve managed")
        .expect("descriptor");
    assert!(managed.managed_worktree);
    let canonical_worktree = worktree.canonicalize().expect("canonical worktree");
    assert!(
        managed
            .fs_profile
            .modify
            .iter()
            .any(|entry| entry == &format!("{}/**", canonical_worktree.display()))
    );
    let prepared = prepare_linux_bwrap_write_grants(&managed.fs_profile, &worktree)
        .expect("prepare resolved managed-worktree grants");
    assert!(
        prepared.unsatisfied.is_empty(),
        "resolved managed-worktree grants must all be mountable: {:?}",
        prepared.unsatisfied
    );
    let plan = compile_linux_bwrap_argv(
        &managed.fs_profile,
        "/bin/true",
        &[],
        Some(&worktree),
        managed.managed_worktree,
    )
    .expect("compile resolved managed-worktree sandbox");
    assert!(
        plan.dropped_grants.is_empty(),
        "prepared managed-worktree grants must not be dropped: {:?}",
        plan.dropped_grants
    );

    let direct = runtime
        .resolve_executor_sandbox("claude", None, Some(&repo_root))
        .expect("resolve direct")
        .expect("descriptor");
    assert!(!direct.managed_worktree);
}

#[cfg(target_os = "linux")]
#[test]
fn resolve_executor_sandbox_orders_versioned_orbit_exceptions_after_default_deny() {
    let (_root, runtime, _repo_root) = runtime_with_workspace_layout();
    seed_executor(
        &runtime,
        "claude",
        Some(orbit_common::types::ExecutorSandboxKind::LinuxBwrap),
    );
    let worktree = runtime
        .paths()
        .orbit_dir
        .join("state/worktrees/orbit-jrun-versioned-config");
    for directory in ["auto_tasks", "routines", "resources", "state", "tasks"] {
        std::fs::create_dir_all(worktree.join(".orbit").join(directory))
            .expect("create worktree Orbit fixture");
    }
    for file in ["config.yaml", "config.toml"] {
        std::fs::write(worktree.join(".orbit").join(file), "versioned = true")
            .expect("create versioned config fixture");
    }

    let resolved = runtime
        .resolve_executor_sandbox("claude", None, Some(&worktree))
        .expect("resolve")
        .expect("descriptor");
    let modify = &resolved.fs_profile.modify;
    let canonical_worktree = worktree.canonicalize().expect("canonical worktree");
    let orbit = canonical_worktree.join(".orbit");
    let deny = format!("!{}/**", orbit.display());
    let deny_pos = modify
        .iter()
        .position(|rule| rule == &deny)
        .unwrap_or_else(|| panic!("default Orbit deny missing from {modify:?}"));

    for allowed in [
        format!("{}/auto_tasks/**", orbit.display()),
        format!("{}/routines/**", orbit.display()),
        format!("{}/config.yaml", orbit.display()),
        format!("{}/config.toml", orbit.display()),
        format!("{}/resources/**", orbit.display()),
    ] {
        let allow_pos = modify
            .iter()
            .position(|rule| rule == &allowed)
            .unwrap_or_else(|| panic!("versioned exception `{allowed}` missing from {modify:?}"));
        assert!(deny_pos < allow_pos, "exception must follow default deny");
    }
    for protected in [
        format!("{}/state/**", orbit.display()),
        format!("{}/tasks/**", orbit.display()),
        format!("{}/learnings/**", orbit.display()),
        format!("{}/future-store/**", orbit.display()),
    ] {
        assert!(
            !modify.iter().any(|rule| rule == &protected),
            "worktree store must not be re-allowed by policy: {protected} in {modify:?}"
        );
    }
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
    // Registered-but-not-activity-exposed stores remain outside this child-runtime inventory.
    for excluded in [
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
        .unwrap_or_else(|| panic!("semantic store should be re-allowed under sandbox: {modify:?}"));
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
