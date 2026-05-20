#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
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

fn assert_root_fields(value: &Value, shared_root: &Path, local_root: &Path) {
    let shared = shared_root.to_string_lossy();
    let local = local_root.to_string_lossy();
    assert_eq!(string_field(value, "shared_root"), shared.as_ref());
    assert_eq!(string_field(value, "local_root"), local.as_ref());
    assert_eq!(string_field(value, "workspace_root"), shared.as_ref());
    assert_eq!(string_field(value, "root"), shared.as_ref());
    assert_eq!(string_field(value, "selected_root"), shared.as_ref());
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
    set_orbit_root_env(&mut command, orbit_root);
    let assert = command.assert().success();
    serde_json::from_slice(&assert.get_output().stdout).expect("orbit json output")
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
