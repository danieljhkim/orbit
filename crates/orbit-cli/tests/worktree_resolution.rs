#![allow(missing_docs)]
// Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;
use std::thread;
use std::time::{Duration, Instant};

use assert_cmd::Command as AssertCommand;
use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn config_show_reports_shared_and_local_roots_for_git_worktrees_and_overrides() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let main_repo = temp.path().join("repo");
    let linked_worktree = temp.path().join("repo-worktree");
    fs::create_dir_all(&home).expect("create home");
    fs::create_dir_all(&main_repo).expect("create main repo");

    run_git(&main_repo, &["init"]);
    run_git(&main_repo, &["config", "user.name", "Orbit Test"]);
    run_git(
        &main_repo,
        &["config", "user.email", "orbit-test@example.com"],
    );
    run_git(&main_repo, &["config", "commit.gpgsign", "false"]);
    fs::write(main_repo.join("README.md"), "# orbit\n").expect("write readme");
    run_git(&main_repo, &["add", "README.md"]);
    run_git(&main_repo, &["commit", "-m", "initial"]);
    run_git(
        &main_repo,
        &[
            "worktree",
            "add",
            "-b",
            "orbit-worktree-resolution",
            linked_worktree.to_str().expect("utf8 worktree path"),
        ],
    );

    run_orbit_success(&main_repo, &home, &["workspace", "init"], None);

    let main_repo = fs::canonicalize(&main_repo).expect("canonicalize main repo");
    let linked_worktree = fs::canonicalize(&linked_worktree).expect("canonicalize linked worktree");
    let main_orbit = main_repo.join(".orbit");
    let linked_orbit = linked_worktree.join(".orbit");

    let from_main = run_orbit_json(&main_repo, &home, &["config", "show", "--json"], None);
    assert_root_fields(&from_main, &main_orbit, &main_orbit);

    let from_worktree =
        run_orbit_json(&linked_worktree, &home, &["config", "show", "--json"], None);
    assert_root_fields(&from_worktree, &main_orbit, &linked_orbit);
    assert!(
        !linked_orbit.exists(),
        "linked worktree local root should be resolved but not created"
    );

    let explicit_root = main_orbit.to_string_lossy().to_string();
    let from_root_override = run_orbit_json(
        &linked_worktree,
        &home,
        &["--root", &explicit_root, "config", "show", "--json"],
        None,
    );
    assert_root_fields(&from_root_override, &main_orbit, &main_orbit);

    let from_env = run_orbit_json(
        &linked_worktree,
        &home,
        &["config", "show", "--json"],
        Some(&main_orbit),
    );
    assert_root_fields(&from_env, &main_orbit, &main_orbit);
    assert!(
        !linked_orbit.exists(),
        "resolution should not materialize a linked-worktree .orbit directory"
    );
}

/// [ORB-10821] `orbit --root <custom> run job` must execute in that custom
/// store. Before the fix the parent persisted the run under `--root` and
/// reported `submitted`, then the detached worker rediscovered `$HOME/.orbit`
/// and exited with `job run not found`, leaving the run pending forever.
#[test]
fn run_job_with_explicit_root_completes_instead_of_staying_pending() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    let custom_root = temp.path().join("custom-orbit");
    fs::create_dir_all(&home).expect("create home");
    fs::create_dir_all(&repo).expect("create repo");
    init_git_repo(&repo);

    run_orbit_success(
        &repo,
        &home,
        &[
            "--root",
            custom_root.to_str().expect("utf8 custom root"),
            "workspace",
            "init",
        ],
        None,
    );

    let job_path = repo.join("root-override-smoke.yaml");
    fs::write(
        &job_path,
        r#"schemaVersion: 2
kind: Job
metadata:
  name: root_override_smoke
spec:
  state: enabled
  kind: workflow
  steps:
    - id: nap
      default_input:
        seconds: 0
      spec:
        type: deterministic
        action: sleep
        config: {}
"#,
    )
    .expect("write smoke job");

    let custom_root_arg = custom_root.to_string_lossy().into_owned();
    let job_path_arg = job_path.to_string_lossy().into_owned();
    let submitted = run_orbit_json(
        &repo,
        &home,
        &[
            "--root",
            &custom_root_arg,
            "run",
            "job",
            &job_path_arg,
            "--json",
        ],
        None,
    );
    let run_id = submitted["run_id"]
        .as_str()
        .unwrap_or_else(|| panic!("expected run_id in {submitted}"))
        .to_string();
    assert_eq!(submitted["state"].as_str(), Some("submitted"));

    let shown = wait_for_run_terminal_state(&repo, &home, &custom_root_arg, &run_id);
    let state = shown["run"]["state"].as_str().unwrap_or("missing");
    let worker_log = custom_root
        .join("state/logs")
        .join(format!("{run_id}.worker.log"));
    let worker_log_text = fs::read_to_string(&worker_log).unwrap_or_default();
    assert_eq!(
        state, "success",
        "run {run_id} must complete under --root {custom_root_arg}; last show={shown}; worker log:\n{worker_log_text}"
    );
}

#[test]
fn doctor_graph_cleanup_uses_split_roots_and_keeps_json_stdout_clean() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let main_repo = temp.path().join("repo");
    let linked_worktree = temp.path().join("repo-doctor");
    fs::create_dir_all(&home).expect("create home");
    fs::create_dir_all(&main_repo).expect("create main repo");

    init_git_repo(&main_repo);
    run_git(
        &main_repo,
        &[
            "worktree",
            "add",
            "-b",
            "orbit-worktree-doctor",
            linked_worktree.to_str().expect("utf8 worktree path"),
        ],
    );
    run_orbit_success(&main_repo, &home, &["workspace", "init"], None);

    let local_marker = linked_worktree.join(".orbit/graph/local.db");
    let shared_marker = main_repo.join(".orbit/knowledge/graph/shared.db");
    for marker in [&local_marker, &shared_marker] {
        fs::create_dir_all(marker.parent().expect("graph parent")).expect("create graph parent");
        fs::write(marker, b"retired").expect("write graph marker");
    }

    let ordinary = run_orbit_json(&linked_worktree, &home, &["doctor", "--json"], None);
    assert!(local_marker.exists());
    assert!(shared_marker.exists());
    assert!(
        ordinary
            .as_array()
            .expect("doctor rows")
            .iter()
            .all(|row| row["check"] != "graph-index")
    );

    let cleaned = run_orbit_json(
        &linked_worktree,
        &home,
        &["doctor", "--remove-graph", "--json"],
        None,
    );
    assert!(!local_marker.exists());
    assert!(!shared_marker.exists());
    assert!(
        cleaned
            .as_array()
            .expect("doctor rows")
            .iter()
            .all(|row| row["check"] != "graph-index")
    );

    // Parsing succeeds again with no cleanup prose mixed into stdout, and
    // absence remains a successful no-op.
    run_orbit_json(
        &linked_worktree,
        &home,
        &["doctor", "--remove-graph", "--json"],
        None,
    );
}

/// ORB-10668: the operator path the tool surface could not serve — an ADR
/// authored inside a job worktree, carried proposed -> accepted with `orbit adr`
/// alone from that worktree, while the same command run from the hub still
/// fails closed on the federation guard.
fn assert_root_fields(value: &Value, shared_root: &Path, local_root: &Path) {
    let shared = shared_root.to_string_lossy();
    let local = local_root.to_string_lossy();
    assert_eq!(string_field(value, "shared_root"), shared.as_ref());
    assert_eq!(string_field(value, "local_root"), local.as_ref());
    assert!(
        value.get("workspace_root").is_none(),
        "legacy `workspace_root` alias must be removed from `config show --json` output (use `shared_root`)"
    );
    assert!(
        value.get("root").is_none(),
        "legacy `root` alias must be removed from `config show --json` output (use `shared_root`)"
    );
    assert!(
        value.get("selected_root").is_none(),
        "legacy `selected_root` alias must be removed from `config show --json` output (use `shared_root`)"
    );
}

fn wait_for_run_terminal_state(cwd: &Path, home: &Path, custom_root: &str, run_id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let last = run_orbit_json(
            cwd,
            home,
            &["--root", custom_root, "run", "show", run_id, "--json"],
            None,
        );
        if last["run"]["state"]
            .as_str()
            .is_some_and(|state| state != "pending" && state != "running")
        {
            return last;
        }
        assert!(
            Instant::now() < deadline,
            "run {run_id} stayed non-terminal under --root {custom_root}; last show={last}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn string_field<'a>(value: &'a Value, field: &str) -> &'a str {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("expected string field `{field}` in {value}"))
}

fn run_orbit_success(cwd: &Path, home: &Path, args: &[&str], orbit_root: Option<&Path>) {
    let mut command = cargo_bin_cmd!("orbit");
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .args(args);
    set_orbit_root_env(&mut command, orbit_root);
    command.assert().success();
}

fn run_orbit_json(cwd: &Path, home: &Path, args: &[&str], orbit_root: Option<&Path>) -> Value {
    let mut command = cargo_bin_cmd!("orbit");
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .args(args);
    clear_agent_identity_env(&mut command);
    set_orbit_root_env(&mut command, orbit_root);
    let assert = command.assert().success();
    serde_json::from_slice(&assert.get_output().stdout).expect("orbit json output")
}

/// Prevent ambient managed-run identity from changing child command behavior.
fn clear_agent_identity_env(command: &mut AssertCommand) {
    command
        .env_remove("ORBIT_AGENT_NAME")
        .env_remove("ORBIT_AGENT_MODEL")
        .env_remove("ORBIT_MANAGED_RUN_CONTEXT")
        .env_remove("ORBIT_RUN_ID");
}

fn set_orbit_root_env(command: &mut AssertCommand, orbit_root: Option<&Path>) {
    match orbit_root {
        Some(path) => {
            command.env("ORBIT_ROOT", path);
        }
        None => {
            command.env_remove("ORBIT_ROOT");
        }
    }
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = StdCommand::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git -C {} {} failed\nstdout:\n{}\nstderr:\n{}",
        cwd.display(),
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_git_repo(main_repo: &Path) {
    run_git(main_repo, &["init"]);
    run_git(main_repo, &["config", "user.name", "Orbit Test"]);
    run_git(
        main_repo,
        &["config", "user.email", "orbit-test@example.com"],
    );
    run_git(main_repo, &["config", "commit.gpgsign", "false"]);
    fs::write(main_repo.join("README.md"), "# orbit\n").expect("write readme");
    run_git(main_repo, &["add", "README.md"]);
    run_git(main_repo, &["commit", "-m", "initial"]);
}
