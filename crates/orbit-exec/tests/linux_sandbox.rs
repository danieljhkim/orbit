#![allow(missing_docs)]
// Integration tests use unwrap/expect for fixture setup and print skip details.
#![allow(clippy::expect_used, clippy::print_stdout, clippy::unwrap_used)]
#![cfg(target_os = "linux")]

use std::process::Stdio;

use orbit_common::types::ResolvedFsProfile;
use orbit_exec::{
    LinuxBwrapPostRunGuard, LinuxBwrapSpawnRequest, WriteAnchorKind, bwrap_path,
    bwrap_program_for_audit, compile_linux_bwrap_argv, linux_bwrap_write_grant_diagnostic,
    linux_bwrap_write_grants, prepare_linux_bwrap_write_grants, probe_bwrap,
    spawn_under_linux_bwrap,
};

fn profile(modify: Vec<String>) -> ResolvedFsProfile {
    ResolvedFsProfile {
        name: "test".to_string(),
        read: vec!["/**".to_string()],
        modify,
    }
}

#[test]
fn trusted_resolution_never_consults_path() {
    assert_eq!(bwrap_program_for_audit(), "/usr/bin/bwrap");
    if let Some(path) = bwrap_path() {
        assert_eq!(path.to_string_lossy(), "/usr/bin/bwrap");
    }
}

/// A worktree-shaped profile: broad writable root, the blanket `.orbit` deny,
/// then whatever narrow re-allows the caller wants to exercise.
fn worktree_profile(worktree: &std::path::Path, reallows: Vec<String>) -> ResolvedFsProfile {
    let mut modify = vec![
        format!("{}/**", worktree.display()),
        format!("!{}/.orbit/**", worktree.display()),
    ];
    modify.extend(reallows);
    profile(modify)
}

#[test]
fn absent_granted_file_and_directory_targets_are_both_materialized() {
    let temp = tempfile::tempdir().expect("tempdir");
    let worktree = temp.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let orbit = worktree.join(".orbit");
    let resolved = worktree_profile(
        &worktree,
        vec![
            // Absent file grant, spelled as an exact leaf.
            orbit.join("config.toml").display().to_string(),
            // Absent directory grant, spelled as a subtree.
            format!("{}/**", orbit.join("auto_tasks").display()),
            // A file grant *beneath* a granted directory: the case the old
            // hardcoded `(path, kind)` table missed, because it matched the
            // directory entry by exact tuple and never created the parent.
            format!("{}/**", orbit.join("routines").display()),
        ],
    );

    let prepared =
        prepare_linux_bwrap_write_grants(&resolved, &worktree).expect("prepare policy grants");

    assert!(
        orbit.join("config.toml").is_file(),
        "an absent granted file target must be materialized"
    );
    assert!(
        orbit.join("auto_tasks").is_dir(),
        "an absent granted directory target must be materialized"
    );
    assert!(orbit.join("routines").is_dir());
    assert_eq!(prepared.created.len(), 3, "{:?}", prepared.created);
    assert!(
        prepared.unsatisfied.is_empty(),
        "{:?}",
        prepared.unsatisfied
    );

    // Every prepared anchor now mounts; nothing is dropped.
    let plan = compile_linux_bwrap_argv(&resolved, "/bin/true", &[], Some(&worktree), true)
        .expect("compile");
    assert!(plan.dropped_grants.is_empty(), "{:?}", plan.dropped_grants);
    let joined = plan.args.join(" ");
    for anchor in ["config.toml", "auto_tasks", "routines"] {
        assert!(
            joined.contains(&format!("--bind {0} {0}", orbit.join(anchor).display())),
            "missing re-allow mount for {anchor} in {joined}"
        );
    }
}

#[test]
fn ungranted_paths_are_never_materialized_and_report_the_deny_that_shadows_them() {
    let temp = tempfile::tempdir().expect("tempdir");
    let worktree = temp.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let orbit = worktree.join(".orbit");
    let resolved = worktree_profile(
        &worktree,
        vec![format!("{}/**", orbit.join("auto_tasks").display())],
    );

    prepare_linux_bwrap_write_grants(&resolved, &worktree).expect("prepare policy grants");

    // Granted sibling exists; the ungranted store does not, and preparation
    // must not invent it just because it sits under the same denied parent.
    assert!(orbit.join("auto_tasks").is_dir());
    assert!(
        !orbit.join("tasks").exists(),
        "a path the policy does not grant must never be materialized"
    );

    assert!(
        linux_bwrap_write_grant_diagnostic(&resolved, &orbit.join("auto_tasks/x.yaml"))
            .expect("diagnose granted path")
            .is_none()
    );
    let denied = linux_bwrap_write_grant_diagnostic(&resolved, &orbit.join("tasks/x.yaml"))
        .expect("diagnose ungranted path")
        .expect("ungranted path must be attributable");
    assert!(
        denied.contains("tasks/x.yaml") && denied.contains("denyModify rule"),
        "diagnostic must name the path and the deny that shadows it: {denied}"
    );
}

/// ADR-0286: auto-task *definitions* resolve through the runtime local root
/// (the worktree), while scheduler cursors and coordination state stay under
/// the shared root. Grant computation must keep those two roots apart: the
/// definition anchor is a narrow re-allow beneath the worktree's own `.orbit`
/// deny and is materialized here, while shared-root coordination state is a
/// host-owned broad root this layer never creates and never relocates into the
/// worktree.
#[test]
fn definition_and_cursor_roots_stay_separate_across_local_and_shared_orbit_dirs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let shared = temp.path().join("primary/.orbit");
    let worktree = shared.join("state/worktrees/orbit-jrun-test");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let local_definitions = worktree.join(".orbit/auto_tasks");
    let shared_cursor_state = shared.join("state/auto-tasks.json");

    let resolved = worktree_profile(
        &worktree,
        vec![
            format!("{}/**", local_definitions.display()),
            shared_cursor_state.display().to_string(),
        ],
    );

    let grants = linux_bwrap_write_grants(&resolved);
    let definitions = grants
        .iter()
        .find(|grant| grant.anchor == local_definitions)
        .expect("definition grant resolves through the local root");
    assert_eq!(definitions.kind, WriteAnchorKind::Directory);

    assert!(
        grants
            .iter()
            .all(|grant| grant.anchor.starts_with(&worktree)),
        "cursor/coordination state under the shared root is never a worktree write grant: {grants:?}"
    );

    let prepared =
        prepare_linux_bwrap_write_grants(&resolved, &worktree).expect("prepare policy grants");

    assert_eq!(prepared.created, vec![local_definitions.clone()]);
    assert!(
        prepared.unsatisfied.is_empty(),
        "{:?}",
        prepared.unsatisfied
    );
    assert!(
        local_definitions.is_dir(),
        "definitions are materialized under the runtime local root"
    );
    assert!(
        !shared.join("auto_tasks").exists(),
        "the local-root definition grant must not reach into the shared root"
    );
    assert!(
        !shared_cursor_state.exists(),
        "shared-root coordination state is the host's to create, not the worktree preparer's"
    );

    // The cursor root is a broad host-owned root, not a narrow exception, so
    // compilation refuses to proceed until the host has materialized it. That
    // refusal is what keeps the two roots from collapsing into one.
    let error = compile_linux_bwrap_argv(&resolved, "/bin/true", &[], Some(&worktree), true)
        .expect_err("an absent host-owned root must fail closed");
    assert!(error.to_string().contains("auto-tasks.json"), "{error}");

    std::fs::write(&shared_cursor_state, "{}").expect("host materializes cursor state");
    let plan = compile_linux_bwrap_argv(&resolved, "/bin/true", &[], Some(&worktree), true)
        .expect("compile");
    let joined = plan.args.join(" ");
    assert!(joined.contains(&format!("--bind {0} {0}", shared_cursor_state.display())));
    assert!(joined.contains(&format!("--bind {0} {0}", local_definitions.display())));
}

#[cfg(unix)]
#[test]
fn write_grant_preparation_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let worktree = temp.path().join("worktree");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir_all(&outside).expect("outside");
    symlink(&outside, worktree.join(".orbit")).expect("symlink Orbit root");
    let resolved = worktree_profile(
        &worktree,
        vec![worktree.join(".orbit/config.toml").display().to_string()],
    );

    let error = prepare_linux_bwrap_write_grants(&resolved, &worktree)
        .expect_err("symlink escape must fail closed");

    assert!(error.to_string().contains("must not be a symlink"));
    assert!(!outside.join("config.toml").exists());
}

#[test]
fn argv_is_deterministic_and_orders_denies_after_writable_parent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let denied = workspace.join(".orbit");
    std::fs::create_dir_all(&denied).expect("create fixture");
    let resolved = profile(vec![
        format!("{}/**", workspace.display()),
        format!("!{}/**", denied.display()),
    ]);

    let plan = compile_linux_bwrap_argv(&resolved, "/bin/true", &[], Some(&workspace), false)
        .expect("compile");
    let joined = plan.args.join(" ");
    for required in [
        "--die-with-parent",
        "--new-session",
        "--unshare-all",
        "--share-net",
        "--ro-bind / /",
        "--dev /dev",
        "--proc /proc",
        "--tmpfs /tmp",
    ] {
        assert!(
            joined.contains(required),
            "missing `{required}` in {joined}"
        );
    }
    let writable = joined.find(&format!("--bind {0} {0}", workspace.display()));
    let readonly = joined.find(&format!("--ro-bind {0} {0}", denied.display()));
    assert!(writable.is_some_and(|index| readonly.is_some_and(|deny| index < deny)));

    let repeated = compile_linux_bwrap_argv(&resolved, "/bin/true", &[], Some(&workspace), false)
        .expect("compile twice");
    assert_eq!(plan, repeated);
}

#[test]
fn argv_reallows_only_narrow_existing_paths_after_orbit_deny() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let orbit = workspace.join(".orbit");
    let resources = orbit.join("resources");
    let tasks = orbit.join("tasks");
    std::fs::create_dir_all(&resources).expect("create resources");
    std::fs::create_dir_all(&tasks).expect("create tasks");

    let resolved = profile(vec![
        format!("{}/**", workspace.display()),
        format!("!{}/**", orbit.display()),
        format!("{}/**", resources.display()),
        orbit.join("missing-config.toml").display().to_string(),
        // A later positive ancestor is not a narrow exception and must not
        // mask the read-only `.orbit` mount.
        format!("{}/**", workspace.display()),
        format!("{}/**", tasks.display()),
    ]);
    let plan = compile_linux_bwrap_argv(&resolved, "/bin/true", &[], Some(&workspace), false)
        .expect("compile");
    let joined = plan.args.join(" ");
    let orbit_readonly = joined
        .find(&format!("--ro-bind {0} {0}", orbit.display()))
        .expect("orbit deny mount");
    let resources_writable = joined
        .find(&format!("--bind {0} {0}", resources.display()))
        .expect("resources re-allow mount");
    let tasks_writable = joined
        .find(&format!("--bind {0} {0}", tasks.display()))
        .expect("trusted store re-allow mount");

    assert!(orbit_readonly < resources_writable);
    assert!(orbit_readonly < tasks_writable);
    assert!(
        !joined.contains("missing-config.toml"),
        "a missing narrow exception is skipped instead of making every sandbox invocation fail"
    );
    let dropped = plan
        .dropped_grants
        .iter()
        .find(|grant| grant.anchor.ends_with("missing-config.toml"))
        .expect("a skipped narrow exception must be reported, never silently dropped");
    assert_eq!(
        dropped.rule,
        orbit.join("missing-config.toml").display().to_string()
    );
    assert_eq!(
        joined
            .match_indices(&format!("--bind {0} {0}", workspace.display()))
            .count(),
        2,
        "both broad profile entries are emitted before the deny, never as a re-allow: {joined}"
    );
    let last_workspace = joined
        .rfind(&format!("--bind {0} {0}", workspace.display()))
        .expect("workspace bind");
    assert!(last_workspace < orbit_readonly);
}

#[test]
fn direct_invocation_fails_closed_for_overlapping_non_subtree_deny() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let resolved = profile(vec![
        format!("{}/**", workspace.display()),
        format!("!{}/**/*.env", workspace.display()),
    ]);
    let error = compile_linux_bwrap_argv(&resolved, "/bin/true", &[], None, false)
        .expect_err("direct invocation must fail closed");
    assert!(error.to_string().contains("non-subtree denyModify"));
}

#[test]
fn managed_worktree_guard_rejects_new_forbidden_match() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let resolved = profile(vec![
        format!("{}/**", workspace.display()),
        format!("!{}/**/*.env", workspace.display()),
    ]);
    let guard = LinuxBwrapPostRunGuard::capture(&resolved)
        .expect("capture")
        .expect("guard required");
    std::fs::write(workspace.join("new.env"), "secret").expect("write forbidden fixture");
    let error = guard.verify().expect_err("new forbidden match rejected");
    assert!(error.to_string().contains("before commit"));
}

#[cfg(target_os = "linux")]
#[test]
fn kernel_enforces_allowed_outside_and_subtree_writes_when_available() {
    let probe = probe_bwrap();
    if !probe.available {
        println!("skipping real Bubblewrap test: {}", probe.detail);
        return;
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let denied = workspace.join(".orbit");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&denied).expect("create denied root");
    std::fs::create_dir_all(&outside).expect("create outside root");
    let allowed_file = workspace.join("allowed.txt");
    let outside_file = outside.join("outside.txt");
    let denied_file = denied.join("denied.txt");
    let resolved = profile(vec![
        format!("{}/**", workspace.display()),
        format!("!{}/**", denied.display()),
    ]);
    let script = format!(
        "printf allowed > '{}'; ! printf outside > '{}'; ! printf denied > '{}'",
        allowed_file.display(),
        outside_file.display(),
        denied_file.display()
    );
    let plan = compile_linux_bwrap_argv(
        &resolved,
        "/bin/sh",
        &["-c".to_string(), script],
        Some(&workspace),
        false,
    )
    .expect("compile");
    let mut child = spawn_under_linux_bwrap(LinuxBwrapSpawnRequest {
        plan: &plan,
        env: &[],
        cwd: Some(&workspace),
        stdin: Stdio::null(),
        stdout: Stdio::null(),
        stderr: Stdio::null(),
    })
    .expect("spawn");
    let status = child.wait().expect("wait");
    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(allowed_file).expect("allowed write"),
        "allowed"
    );
    assert!(!outside_file.exists());
    assert!(!denied_file.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn kernel_enforces_versioned_orbit_exceptions_and_protected_stores_when_available() {
    let probe = probe_bwrap();
    if !probe.available {
        println!("skipping real Bubblewrap test: {}", probe.detail);
        return;
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let orbit = workspace.join(".orbit");
    for directory in [
        "resources",
        "state",
        "tasks",
        "learnings",
        "adrs",
        "frictions",
        "future-store",
    ] {
        std::fs::create_dir_all(orbit.join(directory)).expect("create Orbit fixture directory");
    }
    for file in [
        "config.yaml",
        "orbit.db",
        "config.lock",
        "state/run.json",
        "tasks/task.yaml",
        "learnings/learning.yaml",
        "adrs/adr.yaml",
        "frictions/friction.md",
        "future-store/record.json",
        "resources/private.env",
    ] {
        std::fs::write(orbit.join(file), "before").expect("create Orbit fixture file");
    }

    let resolved = profile(vec![
        format!("{}/**", workspace.display()),
        format!("!{}/**", orbit.display()),
        format!("{}/**", orbit.join("auto_tasks").display()),
        format!("{}/**", orbit.join("routines").display()),
        orbit.join("config.yaml").display().to_string(),
        orbit.join("config.toml").display().to_string(),
        format!("{}/**", orbit.join("resources").display()),
        format!("!{}/**/*.env", workspace.display()),
    ]);
    assert!(!orbit.join("config.toml").exists());
    assert!(!orbit.join("routines").exists());
    assert!(!orbit.join("auto_tasks").exists());
    prepare_linux_bwrap_write_grants(&resolved, &workspace)
        .expect("trusted setup prepares absent granted anchors");
    assert_eq!(
        std::fs::read_to_string(orbit.join("config.toml")).expect("prepared empty config"),
        ""
    );
    assert!(orbit.join("routines").is_dir());
    assert!(orbit.join("auto_tasks").is_dir());
    // Ungranted paths under the same denied parent stay absent and unwritable.
    assert!(!orbit.join("state/new.json").exists());
    assert!(!orbit.join("future-store-new").exists());
    assert!(!orbit.join("resources/new.env").exists());
    let allowed = [
        orbit.join("routines/new.yaml"),
        orbit.join("auto_tasks/new.yaml"),
        orbit.join("config.toml"),
    ];
    let denied = [
        orbit.join("state/run.json"),
        orbit.join("tasks/task.yaml"),
        orbit.join("learnings/learning.yaml"),
        orbit.join("adrs/adr.yaml"),
        orbit.join("frictions/friction.md"),
        orbit.join("orbit.db"),
        orbit.join("config.lock"),
        orbit.join("future-store/record.json"),
        orbit.join("resources/private.env"),
    ];
    let script = allowed
        .iter()
        .map(|path| format!("printf allowed > '{}'", path.display()))
        .chain(
            denied
                .iter()
                .map(|path| format!("! printf denied > '{}'", path.display())),
        )
        .collect::<Vec<_>>()
        .join("; ");
    let plan = compile_linux_bwrap_argv(
        &resolved,
        "/bin/sh",
        &["-c".to_string(), script],
        Some(&workspace),
        true,
    )
    .expect("compile");
    let mut child = spawn_under_linux_bwrap(LinuxBwrapSpawnRequest {
        plan: &plan,
        env: &[],
        cwd: Some(&workspace),
        stdin: Stdio::null(),
        stdout: Stdio::null(),
        stderr: Stdio::null(),
    })
    .expect("spawn");
    let status = child.wait().expect("wait");
    assert!(status.success());

    for path in allowed {
        assert_eq!(
            std::fs::read_to_string(path).expect("allowed write"),
            "allowed"
        );
    }
    for path in denied {
        assert_eq!(
            std::fs::read_to_string(path).expect("denied fixture"),
            "before"
        );
    }
}
