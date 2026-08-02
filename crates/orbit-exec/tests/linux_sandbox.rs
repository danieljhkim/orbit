#![allow(missing_docs)]
#![cfg(target_os = "linux")]

use std::process::Stdio;

use orbit_common::types::ResolvedFsProfile;
use orbit_exec::{
    LinuxBwrapPostRunGuard, LinuxBwrapSpawnRequest, bwrap_path, bwrap_program_for_audit,
    compile_linux_bwrap_argv, prepare_linux_bwrap_versioned_config_targets, probe_bwrap,
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

#[test]
fn versioned_config_preparation_preserves_existing_targets_and_profile_narrowing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let worktree = temp.path().join("worktree");
    std::fs::create_dir_all(worktree.join(".orbit/routines")).expect("existing routines");
    std::fs::write(worktree.join(".orbit/config.toml"), "existing = true\n")
        .expect("existing config");
    let resolved = profile(vec![
        "**".to_string(),
        "!.orbit/**".to_string(),
        ".orbit/config.yaml".to_string(),
        ".orbit/config.toml".to_string(),
        ".orbit/auto_tasks/**".to_string(),
        ".orbit/routines/**".to_string(),
    ]);

    prepare_linux_bwrap_versioned_config_targets(
        &worktree,
        &[
            "file:.orbit/config.toml".to_string(),
            "dir:.orbit/routines".to_string(),
            "dir:.orbit/resources".to_string(),
            "dir:.orbit/config.yaml".to_string(),
            "file:.orbit/auto_tasks".to_string(),
        ],
        &resolved,
    )
    .expect("prepare exact allowed targets");

    assert_eq!(
        std::fs::read_to_string(worktree.join(".orbit/config.toml")).expect("read existing config"),
        "existing = true\n"
    );
    assert!(worktree.join(".orbit/routines").is_dir());
    assert!(!worktree.join(".orbit/resources").exists());
    assert!(
        !worktree.join(".orbit/config.yaml").exists(),
        "a directory selector must not prepare a file target"
    );
    assert!(
        !worktree.join(".orbit/auto_tasks").exists(),
        "a file selector must not prepare a directory target"
    );
}

#[cfg(unix)]
#[test]
fn versioned_config_preparation_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let worktree = temp.path().join("worktree");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&worktree).expect("worktree");
    std::fs::create_dir_all(&outside).expect("outside");
    symlink(&outside, worktree.join(".orbit")).expect("symlink Orbit root");
    let resolved = profile(vec![
        "**".to_string(),
        "!.orbit/**".to_string(),
        ".orbit/config.toml".to_string(),
    ]);

    let error = prepare_linux_bwrap_versioned_config_targets(
        &worktree,
        &["file:.orbit/config.toml".to_string()],
        &resolved,
    )
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
    prepare_linux_bwrap_versioned_config_targets(
        &workspace,
        &[
            "file:.orbit/config.toml".to_string(),
            "dir:.orbit/routines".to_string(),
            "file:.orbit/state/new.json".to_string(),
            "dir:.orbit/future-store-new".to_string(),
            "file:.orbit/resources/new.env".to_string(),
        ],
        &resolved,
    )
    .expect("trusted setup prepares absent allowed targets");
    assert_eq!(
        std::fs::read_to_string(orbit.join("config.toml")).expect("prepared empty config"),
        ""
    );
    assert!(orbit.join("routines").is_dir());
    assert!(!orbit.join("state/new.json").exists());
    assert!(!orbit.join("future-store-new").exists());
    assert!(!orbit.join("resources/new.env").exists());
    let allowed = [orbit.join("routines/new.yaml"), orbit.join("config.toml")];
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
