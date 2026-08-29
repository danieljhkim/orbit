//! [ORB-11055] Nested `orbit.task.update` under the resolved macOS child-runtime
//! profile must initialize `{global}/state/audit` without a blanket `~/.orbit`
//! grant.

#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};

#[cfg(target_os = "macos")]
use orbit_engine::RuntimeHost;
#[cfg(target_os = "macos")]
use orbit_exec::{
    MacosSandboxSpawnRequest, compile_macos_sandbox_profile, sandbox_exec_available,
    sandbox_exec_path, spawn_under_macos_sandbox,
};
#[cfg(target_os = "macos")]
use serde_json::Value;

#[cfg(target_os = "macos")]
use crate::OrbitRuntime;
#[cfg(target_os = "macos")]
use crate::adapter::engine_host::v2_host::test_support::seed_executor;

/// A registered-workspace `orbit.task.update` launched as a nested Orbit
/// command under the resolved Gemini child-runtime profile. The durable task
/// write and global audit-store initialization must both succeed without
/// granting the whole global Orbit root.
#[cfg(target_os = "macos")]
#[test]
fn nested_orbit_task_update_initializes_global_audit_under_sandbox() {
    if !sandbox_exec_can_apply() {
        return;
    }
    let Some(orbit_bin) = locate_orbit_cli_binary() else {
        return;
    };

    let parent = sandbox_test_parent("nested-task-update");
    let _cleanup = ScopeGuard(parent.clone());
    let home = parent.join("home");
    let repo = parent.join("repo");
    std::fs::create_dir_all(&home).expect("create home");
    std::fs::create_dir_all(&repo).expect("create repo");
    init_git_repo(&repo);

    run_orbit(
        &orbit_bin,
        &repo,
        &home,
        &["workspace", "init", "--name", "orb-11055-nested-audit"],
        "workspace init",
    );

    let add = run_orbit(
        &orbit_bin,
        &repo,
        &home,
        &[
            "task",
            "add",
            "--title",
            "Nested sandbox audit write",
            "--description",
            "Registered-workspace fixture for a sandboxed orbit.task.update.",
            "--acceptance-criteria",
            "execution summary persists under the child-runtime profile",
            "--complexity",
            "low",
            "--json",
        ],
        "task add",
    );
    let added: Value = serde_json::from_slice(&add.stdout).expect("task add JSON");
    let task_id = added["id"].as_str().expect("task id").to_string();

    let global = home.join(".orbit");
    let workspace_orbit = repo.join(".orbit");
    let global_str = global
        .canonicalize()
        .unwrap_or_else(|_| global.clone())
        .display()
        .to_string();
    let _ = std::fs::remove_dir_all(global.join("state/audit"));

    let runtime = OrbitRuntime::from_roots(&global, &workspace_orbit).expect("runtime");
    seed_executor(
        &runtime,
        "gemini",
        Some(orbit_types::workflow::ExecutorSandboxKind::MacosSandboxExec),
    );
    let resolved = runtime
        .resolve_executor_sandbox("gemini", None, Some(&repo))
        .expect("resolve")
        .expect("descriptor");
    let modify = &resolved.fs_profile.modify;
    let audit_root = format!("{global_str}/state/audit/**");
    let deny_orbit = format!("!{}/**", workspace_orbit.display());
    let deny_pos = modify
        .iter()
        .position(|entry| entry == &deny_orbit)
        .unwrap_or_else(|| panic!("workspace .orbit deny missing from {modify:?}"));
    let allow_pos = modify
        .iter()
        .position(|entry| entry == &audit_root)
        .unwrap_or_else(|| panic!("global audit grant missing from {modify:?}"));
    assert!(
        deny_pos < allow_pos,
        "global audit grant must follow workspace .orbit deny: {modify:?}"
    );
    assert!(
        !modify.iter().any(|entry| entry == &global_str),
        "must not blanket-reallow the global Orbit root: {modify:?}"
    );

    let home_str = home.to_string_lossy().into_owned();
    let _env = orbit_common::test_env::scoped([("HOME", Some(home_str.as_str()))]);
    let profile_text = compile_macos_sandbox_profile(&resolved.fs_profile, "gemini")
        .expect("compile nested orbit sandbox profile");

    let input = serde_json::json!({
        "id": task_id,
        "execution_summary": "Outcome: success\nChanges:\n- nested orbit.task.update persisted under sandbox-exec\nAssessment: global audit initialized",
        "model": "grok",
    })
    .to_string();
    let args = vec![
        "tool".to_string(),
        "run".to_string(),
        "orbit.task.update".to_string(),
        "--input".to_string(),
        input,
    ];
    let env = vec![
        ("HOME".to_string(), home_str.clone()),
        ("USERPROFILE".to_string(), home_str.clone()),
        (
            "PATH".to_string(),
            "/usr/bin:/bin:/usr/local/bin".to_string(),
        ),
        ("TMPDIR".to_string(), "/tmp".to_string()),
        ("ORBIT_AGENT_NAME".to_string(), "grok".to_string()),
        ("ORBIT_AGENT_MODEL".to_string(), "grok".to_string()),
    ];
    let (child, _profile) = spawn_under_macos_sandbox(MacosSandboxSpawnRequest {
        profile_text: &profile_text,
        program: orbit_bin.to_str().expect("orbit path utf8"),
        args: &args,
        env: &env,
        cwd: Some(&repo),
        stdin: Stdio::null(),
        stdout: Stdio::piped(),
        stderr: Stdio::piped(),
    })
    .expect("spawn nested orbit.task.update");
    let output = child.wait_with_output().expect("wait nested orbit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "sandboxed orbit.task.update failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("file-write-create") && !stderr.contains("Operation not permitted"),
        "nested Orbit must not hit a sandbox EPERM on global audit: {stderr}"
    );

    let shown = run_orbit(
        &orbit_bin,
        &repo,
        &home,
        &["task", "show", &task_id, "--json"],
        "task show after sandboxed update",
    );
    let task: Value = serde_json::from_slice(&shown.stdout).expect("task show JSON");
    let summary = task["execution_summary"].as_str().unwrap_or("");
    assert!(
        summary.contains("nested orbit.task.update persisted"),
        "durable execution summary missing after sandboxed update: {task}"
    );
    assert!(
        global.join("state/audit").is_dir(),
        "global audit store should be initialized: {}",
        global.join("state/audit").display()
    );
}

#[cfg(target_os = "macos")]
fn run_orbit(
    bin: &Path,
    cwd: &Path,
    home: &Path,
    args: &[&str],
    label: &str,
) -> std::process::Output {
    let output = Command::new(bin)
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("ORBIT_ROOT")
        .env_remove("ORBIT_MANAGED_RUN_CONTEXT")
        .env_remove("ORBIT_AGENT_NAME")
        .env_remove("ORBIT_AGENT_MODEL")
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("{label}: spawn orbit: {err}"));
    assert!(
        output.status.success(),
        "{label} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[cfg(target_os = "macos")]
fn locate_orbit_cli_binary() -> Option<PathBuf> {
    if let Some(path) = option_env!("CARGO_BIN_EXE_orbit") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target_root = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("target"));
    for profile in ["debug", "release"] {
        let candidate = target_root.join(profile).join("orbit");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    std::env::var_os("ORBIT_BIN")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

#[cfg(target_os = "macos")]
fn sandbox_exec_can_apply() -> bool {
    if !sandbox_exec_available() {
        return false;
    }
    let Some(path) = sandbox_exec_path() else {
        return false;
    };
    let mut profile_file = tempfile::Builder::new()
        .prefix("orbit-sandbox-probe-")
        .suffix(".sb")
        .tempfile()
        .expect("probe profile");
    use std::io::Write;
    profile_file
        .write_all(b"(version 1)\n(allow default)\n")
        .expect("write probe profile");
    profile_file.flush().expect("flush probe profile");
    Command::new(path)
        .arg("-f")
        .arg(profile_file.path())
        .arg("/usr/bin/true")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn sandbox_test_parent(label: &str) -> PathBuf {
    let roots = [
        Some(std::env::current_dir().expect("current dir")),
        std::env::var_os("HOME").map(PathBuf::from),
    ];
    let suffix = std::process::id().to_string();
    let mut attempts = Vec::new();
    for root in roots.into_iter().flatten() {
        if is_default_write_allow_root(&root) {
            attempts.push(format!(
                "{} is under a broad sandbox write allow",
                root.display()
            ));
            continue;
        }
        let parent = root.join(format!(".orbit-sandbox-test-{suffix}-{label}"));
        match std::fs::create_dir_all(&parent) {
            Ok(()) => return parent,
            Err(err) => attempts.push(format!("{}: {err}", parent.display())),
        }
    }
    panic!(
        "no writable macOS sandbox test parent outside broad write allows: {}",
        attempts.join("; ")
    );
}

#[cfg(target_os = "macos")]
fn is_default_write_allow_root(path: &Path) -> bool {
    let mut roots = vec![
        PathBuf::from("/tmp"),
        PathBuf::from("/private/tmp"),
        PathBuf::from("/private/var/folders"),
        PathBuf::from("/dev"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join("Library/Caches"));
        roots.push(home.join(".orbit/state/logs"));
    }
    let matches = |candidate: &Path| roots.iter().any(|root| candidate.starts_with(root));
    if matches(path) {
        return true;
    }
    path.canonicalize()
        .map(|canonical| matches(&canonical))
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn init_git_repo(repo: &Path) {
    for args in [
        ["init"].as_slice(),
        ["config", "user.name", "Orbit Test"].as_slice(),
        ["config", "user.email", "orbit-test@example.com"].as_slice(),
        ["config", "commit.gpgsign", "false"].as_slice(),
    ] {
        let status = Command::new("git")
            .current_dir(repo)
            .args(args)
            .status()
            .expect("run git");
        assert!(status.success(), "git {args:?} failed: {status:?}");
    }
    std::fs::write(repo.join("README.md"), "# fixture\n").expect("write readme");
    let add = Command::new("git")
        .current_dir(repo)
        .args(["add", "README.md"])
        .status()
        .expect("git add");
    assert!(add.success(), "git add failed");
    let commit = Command::new("git")
        .current_dir(repo)
        .args(["commit", "-m", "fixture"])
        .status()
        .expect("git commit");
    assert!(commit.success(), "git commit failed");
}

#[cfg(target_os = "macos")]
struct ScopeGuard(PathBuf);

#[cfg(target_os = "macos")]
impl Drop for ScopeGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
