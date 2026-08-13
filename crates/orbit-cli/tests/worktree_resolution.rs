#![allow(missing_docs)]
// Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

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
