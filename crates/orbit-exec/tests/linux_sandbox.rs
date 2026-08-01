#![allow(missing_docs)]

use std::process::Stdio;

use orbit_common::types::ResolvedFsProfile;
use orbit_exec::{
    LinuxBwrapPostRunGuard, LinuxBwrapSpawnRequest, bwrap_path, bwrap_program_for_audit,
    compile_linux_bwrap_argv, probe_bwrap, spawn_under_linux_bwrap,
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
