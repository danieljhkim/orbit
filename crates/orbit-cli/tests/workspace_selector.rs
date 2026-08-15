#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::Value;
use tempfile::tempdir;

#[test]
fn global_workspace_flag_selects_by_name_and_id_from_a_foreign_checkout() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let orbit_repo = temp.path().join("orbit");
    let other_repo = temp.path().join("other");
    let elsewhere = temp.path().join("elsewhere");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&elsewhere).expect("elsewhere");

    init_git_repo(&orbit_repo);
    init_git_repo(&other_repo);

    run_orbit(
        &orbit_repo,
        &home,
        &[
            "init",
            "--non-interactive",
            "--host-name",
            "selector-host",
            "--task-prefix",
            "SEL",
        ],
    )
    .success();
    run_orbit(
        &orbit_repo,
        &home,
        &["workspace", "init", "--name", "orbit"],
    )
    .success();
    run_orbit(
        &other_repo,
        &home,
        &["workspace", "init", "--name", "other"],
    )
    .success();

    let created = run_orbit_json(
        &orbit_repo,
        &home,
        &[
            "task",
            "add",
            "--title",
            "Orbit-only task",
            "--description",
            "Must stay in the orbit workspace",
            "--json",
        ],
    );
    let orbit_task_id = created["id"].as_str().expect("created id").to_string();

    let by_name = run_orbit_json(
        &elsewhere,
        &home,
        &[
            "--workspace",
            "orbit",
            "task",
            "list",
            "--limit",
            "10",
            "--json",
        ],
    );
    assert!(
        task_ids(&by_name).contains(&orbit_task_id),
        "name selector from a foreign cwd must list orbit tasks: {by_name}"
    );

    let by_id = run_orbit_json(
        &elsewhere,
        &home,
        &[
            "--workspace",
            "ws_orbit",
            "task",
            "list",
            "--limit",
            "10",
            "--json",
        ],
    );
    assert!(
        task_ids(&by_id).contains(&orbit_task_id),
        "logical id selector from a foreign cwd must list orbit tasks: {by_id}"
    );

    let by_path = run_orbit_json(
        &elsewhere,
        &home,
        &[
            "--workspace",
            orbit_repo.to_str().expect("utf8"),
            "task",
            "list",
            "--limit",
            "10",
            "--json",
        ],
    );
    assert!(
        task_ids(&by_path).contains(&orbit_task_id),
        "absolute checkout path must list orbit tasks: {by_path}"
    );

    let other_cwd = run_orbit_json(
        &other_repo,
        &home,
        &["task", "list", "--limit", "10", "--json"],
    );
    assert!(
        !task_ids(&other_cwd).contains(&orbit_task_id),
        "cwd discovery without --workspace must keep binding the other checkout: {other_cwd}"
    );

    let shown_from_foreign_checkout = run_orbit_json(
        &other_repo,
        &home,
        &["task", "show", &orbit_task_id, "--json"],
    );
    assert_eq!(shown_from_foreign_checkout["id"], orbit_task_id);
    assert_eq!(shown_from_foreign_checkout["workspace"]["name"], "orbit");
    assert_eq!(shown_from_foreign_checkout["workspace"]["id"], "ws_orbit");

    let shown_outside_a_workspace = run_orbit_json(
        &elsewhere,
        &home,
        &["task", "show", &orbit_task_id, "--json"],
    );
    assert_eq!(shown_outside_a_workspace["id"], orbit_task_id);

    let explicit_miss = run_orbit(
        &elsewhere,
        &home,
        &[
            "--workspace",
            "other",
            "task",
            "show",
            &orbit_task_id,
            "--json",
        ],
    )
    .failure();
    assert!(
        !String::from_utf8_lossy(&explicit_miss.get_output().stdout).contains("Orbit-only task"),
        "an explicit foreign workspace must not disclose the task"
    );
}

#[test]
fn global_workspace_flag_fails_closed_on_unknown_selector() {
    let temp = tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&home).expect("home");
    init_git_repo(&repo);
    run_orbit(
        &repo,
        &home,
        &[
            "init",
            "--non-interactive",
            "--host-name",
            "selector-host",
            "--task-prefix",
            "SEL",
        ],
    )
    .success();
    run_orbit(&repo, &home, &["workspace", "init", "--name", "orbit"]).success();

    let assert = run_orbit(
        &repo,
        &home,
        &[
            "--workspace",
            "no-such-workspace",
            "task",
            "list",
            "--limit",
            "1",
        ],
    )
    .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("no-such-workspace"),
        "unknown selector must be named: {stderr}"
    );
}

fn task_ids(value: &Value) -> Vec<String> {
    let items = value
        .as_array()
        .cloned()
        .or_else(|| value.get("tasks").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    items
        .iter()
        .filter_map(|task| {
            task.get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect()
}

fn run_orbit(cwd: &Path, home: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    let mut command = cargo_bin_cmd!("orbit");
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("ORBIT_ROOT")
        .env_remove("ORBIT_AGENT_NAME")
        .env_remove("ORBIT_AGENT_MODEL")
        .env_remove("ORBIT_MANAGED_RUN_CONTEXT")
        .env_remove("ORBIT_RUN_ID")
        .args(args);
    command.assert()
}

fn run_orbit_json(cwd: &Path, home: &Path, args: &[&str]) -> Value {
    let assert = run_orbit(cwd, home, args).success();
    serde_json::from_slice(&assert.get_output().stdout).expect("orbit json output")
}

fn init_git_repo(repo: &Path) {
    fs::create_dir_all(repo).expect("create repo");
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.name", "Orbit Test"]);
    run_git(repo, &["config", "user.email", "orbit-test@example.com"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    fs::write(repo.join("README.md"), "# repo\n").expect("write readme");
    run_git(repo, &["add", "README.md"]);
    run_git(repo, &["commit", "-m", "initial"]);
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
