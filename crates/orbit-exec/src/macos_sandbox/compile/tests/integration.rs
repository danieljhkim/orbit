use super::super::super::test_support::*;

#[cfg(target_os = "macos")]
use super::super::compile_macos_sandbox_profile;
#[cfg(target_os = "macos")]
use orbit_common::types::ResolvedFsProfile;

#[cfg(target_os = "macos")]
#[test]
fn compiled_profile_allows_nested_orbit_runtime_writes_without_home_orbit_reallow() {
    use std::process::Command;

    if !sandbox_exec_can_apply() {
        return;
    }

    let parent = sandbox_test_parent("orbit-runtime-roots");
    let _cleanup = ScopeGuard(parent.clone());
    let home = parent.join("home");
    let global = home.join(".orbit");
    let workspace = parent.join("repo/.orbit");
    std::fs::create_dir_all(global.join("state/logs")).expect("global log dir");
    std::fs::create_dir_all(global.join("tasks")).expect("global tasks dir");
    std::fs::create_dir_all(workspace.join("state")).expect("workspace state dir");
    std::fs::create_dir_all(workspace.join("adrs/.locks")).expect("workspace adr locks dir");

    let log_path = global.join("state/logs/orbit.jsonl");
    let db_wal_path = global.join("orbit.db-wal");
    let artifact_path = global
        .join("tasks/workspaces/orbit-test/ORB-00009/artifacts/files/planning-duel")
        .join("planner_a.md");
    let id_alloc_lock_path = workspace.join("state/.id_alloc.lock");
    let semantic_wal_path = workspace.join("state/semantic.db-wal");
    let denied_path = global.join("not-allowed.txt");
    let denied_workspace_path = workspace.join("adrs/.locks/should-stay-denied.lock");

    let resolved = ResolvedFsProfile {
        name: "gemini-direct-agent".to_string(),
        read: vec![parent.display().to_string()],
        modify: vec![
            format!("{}/state/logs/**", global.display()),
            format!("{}/orbit.db*", global.display()),
            format!("{}/tasks/**", global.display()),
            format!("!{}/**", workspace.display()),
            format!("{}/state/.id_alloc.lock", workspace.display()),
            format!("{}/state/semantic.db*", workspace.display()),
        ],
    };
    let home_str = home.to_string_lossy().into_owned();
    let profile_text = compile_with_env(
        &resolved,
        EnvOverrides {
            home: Some(&home_str),
            ..Default::default()
        },
    );
    let mut profile_file = tempfile::Builder::new()
        .prefix("orbit-sandbox-test-")
        .suffix(".sb")
        .tempfile()
        .expect("tempfile");
    use std::io::Write;
    profile_file
        .write_all(profile_text.as_bytes())
        .expect("write profile");
    profile_file.flush().expect("flush");

    let script = format!(
        "set -e\n: > {}\n: > {}\nmkdir -p {}\nprintf '%s\\n' '*authored by: gemini / gemini-3.1-pro*' > {}\n: > {}\n: > {}\nif : > {} 2>/dev/null; then exit 99; fi\nif : > {} 2>/dev/null; then exit 98; fi\n",
        shell_escape(&log_path),
        shell_escape(&db_wal_path),
        shell_escape(artifact_path.parent().expect("artifact parent")),
        shell_escape(&artifact_path),
        shell_escape(&id_alloc_lock_path),
        shell_escape(&semantic_wal_path),
        shell_escape(&denied_path),
        shell_escape(&denied_workspace_path),
    );
    let status = Command::new(sandbox_exec_path_for_test())
        .arg("-f")
        .arg(profile_file.path())
        .arg("/bin/sh")
        .arg("-c")
        .arg(script)
        .env("HOME", &home)
        .status()
        .expect("run sandbox-exec");

    assert!(
        status.success(),
        "expected Orbit runtime writes to succeed while arbitrary HOME/.orbit write is denied; status={status:?}"
    );
    assert!(log_path.exists(), "log file should be writable");
    assert!(db_wal_path.exists(), "SQLite sidecar should be writable");
    assert!(
        artifact_path.exists(),
        "planner artifact should be writable"
    );
    assert!(
        id_alloc_lock_path.exists(),
        "workspace id allocator lock should be writable"
    );
    assert!(
        semantic_wal_path.exists(),
        "semantic sidecar should be writable"
    );
    assert!(
        !denied_path.exists(),
        "arbitrary HOME/.orbit write should remain denied"
    );
    assert!(
        !denied_workspace_path.exists(),
        "unrelated workspace .orbit write should remain denied"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn compiled_profile_blocks_writes_outside_modify_scope() {
    use std::process::Command;

    if !sandbox_exec_can_apply() {
        return;
    }

    // The compiled profile broadly allows writes under /tmp,
    // /private/tmp, /private/var/folders, and ~/Library/Caches so
    // agent CLIs can use scratch space. To exercise modify-scope
    // enforcement we need a parent that lives outside all of those.
    let parent = sandbox_test_parent("modify-scope");
    let _cleanup = ScopeGuard(parent.clone());
    let dir = tempfile::Builder::new()
        .prefix("compile-")
        .tempdir_in(&parent)
        .expect("tempdir in parent");
    let allowed = dir.path().join("allowed");
    let blocked = dir.path().join("blocked");
    std::fs::create_dir_all(&allowed).expect("allowed dir");
    std::fs::create_dir_all(&blocked).expect("blocked dir");

    let resolved = ResolvedFsProfile {
        name: "default".to_string(),
        read: vec![dir.path().display().to_string()],
        modify: vec![allowed.display().to_string()],
    };
    let profile_text = compile_macos_sandbox_profile(&resolved).expect("compile sbpl");
    let mut profile_file = tempfile::Builder::new()
        .prefix("orbit-sandbox-test-")
        .suffix(".sb")
        .tempfile()
        .expect("tempfile");
    use std::io::Write;
    profile_file
        .write_all(profile_text.as_bytes())
        .expect("write profile");
    profile_file.flush().expect("flush");

    let allowed_target = allowed.join("ok");
    let allow_status = Command::new(sandbox_exec_path_for_test())
        .arg("-f")
        .arg(profile_file.path())
        .arg("/bin/sh")
        .arg("-c")
        .arg(format!("echo ok > {}", shell_escape(&allowed_target)))
        .status()
        .expect("run sandbox-exec");
    assert!(
        allow_status.success(),
        "expected write inside modify scope to succeed; status={allow_status:?}"
    );
    assert!(
        allowed_target.exists(),
        "allowed file was not written: {allowed_target:?}"
    );

    let blocked_target = blocked.join("nope");
    let deny_status = Command::new(sandbox_exec_path_for_test())
        .arg("-f")
        .arg(profile_file.path())
        .arg("/bin/sh")
        .arg("-c")
        .arg(format!("echo bad > {}", shell_escape(&blocked_target)))
        .status()
        .expect("run sandbox-exec");
    assert!(
        !deny_status.success(),
        "expected write outside modify scope to fail; status={deny_status:?}"
    );
    assert!(
        !blocked_target.exists(),
        "blocked file should not exist: {blocked_target:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn compiled_profile_denies_reads_to_negated_read_path() {
    // Invariant: an SBPL profile compiled from `read: [base, !secrets]`
    // must let the kernel block reads of `secrets/...` while still
    // allowing reads of sibling paths under `base`. This is the
    // runtime complement to `compile_emits_explicit_read_deny_for_negated_read_rule`.
    use std::process::Command;

    if !sandbox_exec_can_apply() {
        return;
    }

    let parent = sandbox_test_parent("read-deny");
    let _cleanup = ScopeGuard(parent.clone());
    let dir = tempfile::Builder::new()
        .prefix("compile-readdeny-")
        .tempdir_in(&parent)
        .expect("tempdir in parent");
    let secrets_dir = dir.path().join("secrets");
    std::fs::create_dir_all(&secrets_dir).expect("secrets dir");
    let secret_path = secrets_dir.join("api.key");
    std::fs::write(&secret_path, b"top-secret").expect("write secret");
    let public_path = dir.path().join("public.txt");
    std::fs::write(&public_path, b"public-data").expect("write public");

    let resolved = ResolvedFsProfile {
        name: "default".to_string(),
        read: vec![
            dir.path().display().to_string(),
            format!("!{}", secrets_dir.display()),
        ],
        modify: vec![],
    };
    let profile_text = compile_macos_sandbox_profile(&resolved).expect("compile sbpl");
    let mut profile_file = tempfile::Builder::new()
        .prefix("orbit-sandbox-test-")
        .suffix(".sb")
        .tempfile()
        .expect("tempfile");
    use std::io::Write;
    profile_file
        .write_all(profile_text.as_bytes())
        .expect("write profile");
    profile_file.flush().expect("flush");

    // Allowed read of public_path succeeds.
    let allow_status = Command::new(sandbox_exec_path_for_test())
        .arg("-f")
        .arg(profile_file.path())
        .arg("/bin/sh")
        .arg("-c")
        .arg(format!("cat {}", shell_escape(&public_path)))
        .status()
        .expect("run sandbox-exec");
    assert!(
        allow_status.success(),
        "public read should be allowed; status={allow_status:?}"
    );

    // Denied read of secret_path fails.
    let deny_status = Command::new(sandbox_exec_path_for_test())
        .arg("-f")
        .arg(profile_file.path())
        .arg("/bin/sh")
        .arg("-c")
        .arg(format!("cat {}", shell_escape(&secret_path)))
        .status()
        .expect("run sandbox-exec");
    assert!(
        !deny_status.success(),
        "secrets read should be denied by negated read rule; status={deny_status:?}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn compiled_profile_for_realistic_agent_loop_profile_allows_repo_writes_denies_dotenv() {
    // Realistic activity profile boundary test (AC #2). Synthesize an
    // `agent_loop`-style profile: read=[repo], modify=[repo, !repo/.env].
    // Exercise allow + deny in one process: writing `repo/src/foo.rs`
    // succeeds; writing `repo/.env` fails. Mirrors how an `agent_loop`
    // step would be sandboxed at runtime.
    use std::process::Command;

    if !sandbox_exec_can_apply() {
        return;
    }

    let parent = sandbox_test_parent("agent-loop-realistic");
    let _cleanup = ScopeGuard(parent.clone());
    let repo = tempfile::Builder::new()
        .prefix("agent-loop-")
        .tempdir_in(&parent)
        .expect("repo tempdir");
    let src_dir = repo.path().join("src");
    std::fs::create_dir_all(&src_dir).expect("src dir");

    let resolved = ResolvedFsProfile {
        name: "agent_loop".to_string(),
        read: vec![repo.path().display().to_string()],
        modify: vec![
            repo.path().display().to_string(),
            format!("!{}/.env", repo.path().display()),
        ],
    };
    let profile_text = compile_macos_sandbox_profile(&resolved).expect("compile sbpl");
    let mut profile_file = tempfile::Builder::new()
        .prefix("orbit-sandbox-test-")
        .suffix(".sb")
        .tempfile()
        .expect("tempfile");
    use std::io::Write;
    profile_file
        .write_all(profile_text.as_bytes())
        .expect("write profile");
    profile_file.flush().expect("flush");

    let source_target = src_dir.join("foo.rs");
    let env_target = repo.path().join(".env");

    let source_status = Command::new(sandbox_exec_path_for_test())
        .arg("-f")
        .arg(profile_file.path())
        .arg("/bin/sh")
        .arg("-c")
        .arg(format!(
            "echo 'fn main() {{}}' > {}",
            shell_escape(&source_target)
        ))
        .status()
        .expect("run sandbox-exec");
    assert!(
        source_status.success(),
        "agent_loop must be able to write source files; status={source_status:?}"
    );
    assert!(source_target.exists(), "source file not written");

    let env_status = Command::new(sandbox_exec_path_for_test())
        .arg("-f")
        .arg(profile_file.path())
        .arg("/bin/sh")
        .arg("-c")
        .arg(format!("echo 'KEY=secret' > {}", shell_escape(&env_target)))
        .status()
        .expect("run sandbox-exec");
    assert!(
        !env_status.success(),
        "agent_loop must be blocked from writing .env; status={env_status:?}"
    );
    assert!(!env_target.exists(), ".env should not have been written");
}

#[cfg(target_os = "macos")]
#[test]
fn compiled_profile_denies_env_glob_without_blocking_other_writes() {
    use std::process::Command;

    if !sandbox_exec_can_apply() {
        return;
    }

    let parent = sandbox_test_parent("env-glob");
    let _cleanup = ScopeGuard(parent.clone());
    let dir = tempfile::Builder::new()
        .prefix("compile-env-")
        .tempdir_in(&parent)
        .expect("tempdir in parent");

    let resolved = ResolvedFsProfile {
        name: "default".to_string(),
        read: vec![dir.path().display().to_string()],
        modify: vec![
            dir.path().display().to_string(),
            format!("!{}/**/*.env", dir.path().display()),
        ],
    };
    let profile_text = compile_macos_sandbox_profile(&resolved).expect("compile sbpl");
    let mut profile_file = tempfile::Builder::new()
        .prefix("orbit-sandbox-test-")
        .suffix(".sb")
        .tempfile()
        .expect("tempfile");
    use std::io::Write;
    profile_file
        .write_all(profile_text.as_bytes())
        .expect("write profile");
    profile_file.flush().expect("flush");

    let allowed_target = dir.path().join("ok.txt");
    let allow_status = Command::new(sandbox_exec_path_for_test())
        .arg("-f")
        .arg(profile_file.path())
        .arg("/bin/sh")
        .arg("-c")
        .arg(format!("echo ok > {}", shell_escape(&allowed_target)))
        .status()
        .expect("run sandbox-exec");
    assert!(
        allow_status.success(),
        "env glob deny should not block non-env writes; status={allow_status:?}"
    );

    let env_target = dir.path().join("blocked.env");
    let deny_status = Command::new(sandbox_exec_path_for_test())
        .arg("-f")
        .arg(profile_file.path())
        .arg("/bin/sh")
        .arg("-c")
        .arg(format!("echo bad > {}", shell_escape(&env_target)))
        .status()
        .expect("run sandbox-exec");
    assert!(
        !deny_status.success(),
        "expected env glob write to fail; status={deny_status:?}"
    );
    assert!(
        !env_target.exists(),
        "env file should not exist: {env_target:?}"
    );
}
