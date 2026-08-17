use super::super::spawn::sandbox_exec_path_from;
#[cfg(target_os = "macos")]
use super::super::spawn::{MacosSandboxSpawnRequest, spawn_under_macos_sandbox};
#[cfg(target_os = "macos")]
use super::super::test_support::{
    ScopeGuard, sandbox_exec_can_apply, sandbox_test_parent, shell_escape,
};
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::Stdio;

#[test]
fn sandbox_exec_path_from_uses_trusted_absolute_candidate() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = dir.path().join("sandbox-exec");
    std::fs::write(&bin, "#!/bin/sh\nexit 0\n").expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("perms");
    }
    assert_eq!(sandbox_exec_path_from([bin.as_path()]), Some(bin));
}

#[test]
fn sandbox_exec_path_from_rejects_relative_candidates() {
    let bin = Path::new("sandbox-exec");
    assert_eq!(sandbox_exec_path_from([bin]), None);
}

#[cfg(target_os = "macos")]
#[test]
fn spawn_under_macos_sandbox_ignores_fake_sandbox_exec_on_path() {
    if !sandbox_exec_can_apply() {
        return;
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let fake_dir = temp.path().join("fake-bin");
    std::fs::create_dir_all(&fake_dir).expect("fake dir");
    let marker = temp.path().join("fake-used");
    let fake = fake_dir.join("sandbox-exec");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\necho fake > {}\nexit 77\n",
            shell_escape(&marker)
        ),
    )
    .expect("write fake sandbox-exec");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
            .expect("fake perms");
    }

    let poisoned_path = format!("{}:/usr/bin:/bin", fake_dir.display());
    let args = ["-c".to_string(), "exit 0".to_string()];
    let env = [("PATH".to_string(), poisoned_path)];
    let (child, _profile_file) = spawn_under_macos_sandbox(MacosSandboxSpawnRequest {
        profile_text: "(version 1)\n(allow default)\n",
        program: "/bin/sh",
        args: &args,
        env: &env,
        cwd: None,
        stdin: Stdio::null(),
        stdout: Stdio::piped(),
        stderr: Stdio::piped(),
    })
    .expect("spawn sandboxed child");
    let output = child.wait_with_output().expect("wait for child");

    assert!(
        output.status.success(),
        "trusted sandbox-exec should run child despite fake PATH entry; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !marker.exists(),
        "fake sandbox-exec on PATH should not have been executed"
    );
}

/// [ORB-10917] `sandbox-exec` hands its own environment to the confined
/// program, so the launcher must apply exactly the environment the dispatcher
/// composed. The ambient variables are set here rather than read from the
/// developer's shell, and none carries a credential-shaped name — a denylist
/// would forward every one of them.
#[cfg(target_os = "macos")]
#[test]
fn spawn_under_macos_sandbox_gives_the_child_only_the_supplied_environment() {
    if !sandbox_exec_can_apply() {
        return;
    }

    let _ambient = orbit_common::test_env::scoped([
        ("DATABASE_URL", Some("postgres://svc:hunter2@db.internal")),
        ("BILLING_ENDPOINT", Some("https://billing.internal.example")),
        ("ORB_10917_AMBIENT", Some("leaked")),
        ("ANTHROPIC_API_KEY", Some("sk-ant-000000000000000000000")),
    ]);
    let args = ["-c".to_string(), "env".to_string()];
    let env = [
        ("PATH".to_string(), "/usr/bin:/bin".to_string()),
        ("ORB_10917_SUPPLIED".to_string(), "present".to_string()),
    ];
    let (child, _profile_file) = spawn_under_macos_sandbox(MacosSandboxSpawnRequest {
        profile_text: "(version 1)\n(allow default)\n",
        program: "/bin/sh",
        args: &args,
        env: &env,
        cwd: None,
        stdin: Stdio::null(),
        stdout: Stdio::piped(),
        stderr: Stdio::piped(),
    })
    .expect("spawn sandboxed child");
    let output = child.wait_with_output().expect("wait for child");
    let child_env = String::from_utf8_lossy(&output.stdout);

    assert!(
        child_env.contains("ORB_10917_SUPPLIED=present"),
        "supplied vars must reach the confined child: {child_env}"
    );
    for leaked in [
        "DATABASE_URL",
        "BILLING_ENDPOINT",
        "ORB_10917_AMBIENT",
        "ANTHROPIC_API_KEY",
    ] {
        assert!(
            !child_env.contains(leaked),
            "{leaked} must not reach a sandbox-exec provider child: {child_env}"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn spawn_under_macos_sandbox_runs_program_in_provided_cwd() {
    if !sandbox_exec_can_apply() {
        return;
    }

    let parent = sandbox_test_parent("cwd");
    let _cleanup = ScopeGuard(parent.clone());
    let dir = tempfile::Builder::new()
        .prefix("sandbox-cwd-")
        .tempdir_in(&parent)
        .expect("cwd tempdir");
    let cwd = dir.path().canonicalize().expect("canonical cwd");
    let args = ["-c".to_string(), "pwd".to_string()];
    let (child, _profile_file) = spawn_under_macos_sandbox(MacosSandboxSpawnRequest {
        profile_text: "(version 1)\n(allow default)\n",
        program: "/bin/sh",
        args: &args,
        env: &[],
        cwd: Some(&cwd),
        stdin: Stdio::null(),
        stdout: Stdio::piped(),
        stderr: Stdio::piped(),
    })
    .expect("spawn sandboxed child");
    let output = child.wait_with_output().expect("wait for child");

    assert!(
        output.status.success(),
        "sandboxed pwd should succeed; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout utf8"),
        format!("{}\n", cwd.display())
    );
}
