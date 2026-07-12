#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! End-to-end coverage for the trimmed `orbit task` surface (ORB-10000):
//! approve/reject/unarchive folded into `update --status` and `prune-context`
//! folded into `lint --fix`. Lock administration lives under the top-level
//! `orbit locks` command (`list`/`release`, ORB-00420).

use std::fs;
use std::path::Path;
use std::process::Output;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

#[test]
fn update_status_backlog_restores_archived_task() {
    let workspace = TestWorkspace::new();
    let id = workspace.add_task("Restore me");
    workspace.run(&["task", "update", &id, "--status", "backlog"], "approve");
    workspace.run(&["task", "archive", &id], "archive");

    let restored = workspace.task_json(&["task", "update", &id, "--status", "backlog", "--json"]);
    assert_eq!(restored["status"], json!("backlog"));
}

#[test]
fn update_status_rejected_rejects_illegal_jump_from_done() {
    let workspace = TestWorkspace::new();
    let id = workspace.add_task("Terminal task");
    workspace.drive_to_done(&id);

    let output = workspace.run_raw(&["task", "update", &id, "--status", "rejected"]);
    assert!(
        !output.status.success(),
        "done -> rejected must fail:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("done"),
        "stderr should name the terminal status:\n{stderr}"
    );
}

#[test]
fn update_status_performs_approve_and_reject_transitions() {
    let workspace = TestWorkspace::new();

    // proposed -> backlog (former `approve`).
    let id = workspace.add_task("Approve via update");
    let task = workspace.task_json(&["task", "update", &id, "--status", "backlog", "--json"]);
    assert_eq!(task["status"], json!("backlog"));

    // proposed -> rejected (former `reject`).
    let id = workspace.add_task("Reject via update");
    let task = workspace.task_json(&["task", "update", &id, "--status", "rejected", "--json"]);
    assert_eq!(task["status"], json!("rejected"));
}

#[test]
fn locks_list_projects_files_held_by_active_tasks() {
    let workspace = TestWorkspace::new();
    fs::write(workspace.work.join("held.rs"), "// held\n").expect("write held file");

    let id = workspace.add_task("Holds a lock");
    workspace.run(&["task", "update", &id, "--status", "backlog"], "approve");
    workspace.run(
        &[
            "task",
            "update",
            &id,
            "--plan",
            "1) hold the file",
            "--context",
            "file:held.rs",
            "--status",
            "in-progress",
        ],
        "start with context",
    );

    let output = workspace.run(&["locks", "list", "--json"], "locks list");
    let value: Value = serde_json::from_slice(&output.stdout).expect("locked JSON");
    assert_eq!(value["total_tasks"], json!(1));
    assert_eq!(value["locked_files"], json!(["file:held.rs"]));
    assert_eq!(value["by_task"][0]["id"], json!(id));

    let text = workspace.run(&["locks", "list"], "locks list text");
    let stdout = String::from_utf8_lossy(&text.stdout);
    assert!(stdout.contains("file:held.rs"), "{stdout}");
}

#[test]
fn lint_fix_sweep_drops_stale_context_entries() {
    let workspace = TestWorkspace::new();
    fs::write(workspace.work.join("real.rs"), "// real\n").expect("write real file");
    fs::write(workspace.work.join("ghost.rs"), "// ghost\n").expect("write ghost file");

    let id = workspace.add_task("Has stale context");
    workspace.run(
        &[
            "task",
            "update",
            &id,
            "--context",
            "file:real.rs,file:ghost.rs",
        ],
        "set context files",
    );
    fs::remove_file(workspace.work.join("ghost.rs")).expect("remove ghost file");

    // Dry run first: reports the stale entry without writing.
    let dry = workspace.run(&["task", "lint", "--json"], "lint sweep dry run");
    let dry: Value = serde_json::from_slice(&dry.stdout).expect("dry-run JSON");
    assert_eq!(dry["dry_run"], json!(true));
    assert_eq!(dry["total_dropped"], json!(1));
    assert_eq!(dry["tasks_written"], json!(0));

    // Apply.
    let fixed = workspace.run(&["task", "lint", "--fix", "--json"], "lint sweep fix");
    let fixed: Value = serde_json::from_slice(&fixed.stdout).expect("fix JSON");
    assert_eq!(fixed["dry_run"], json!(false));
    assert_eq!(fixed["total_dropped"], json!(1));
    assert_eq!(fixed["tasks_written"], json!(1));
    assert_eq!(fixed["tasks"][0]["dropped"], json!(["file:ghost.rs"]));

    let task = workspace.task_json(&["task", "show", &id, "--json"]);
    assert_eq!(task["context_files"], json!(["file:real.rs"]));

    // Idempotent: nothing left to prune.
    let again = workspace.run(&["task", "lint", "--fix", "--json"], "lint sweep again");
    let again: Value = serde_json::from_slice(&again.stdout).expect("second fix JSON");
    assert_eq!(again["total_dropped"], json!(0));
}

#[test]
fn task_add_attributes_from_model_flag_and_managed_identity_env() {
    let workspace = TestWorkspace::new();

    let explicit = workspace.task_json(&[
        "task",
        "add",
        "--title",
        "Explicit model",
        "--description",
        "Model flag attribution",
        "--model",
        "gpt-5.6-sol",
        "--json",
    ]);
    assert_eq!(explicit["created_by"], json!("gpt-5.6-sol"));

    let output = run_orbit_with_identity(
        &workspace.work,
        &workspace.home,
        &[
            "task",
            "add",
            "--title",
            "Managed identity",
            "--description",
            "Environment attribution",
            "--json",
        ],
        "codex",
        "gpt-5.6-terra",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let managed: Value = serde_json::from_slice(&output.stdout).expect("managed task JSON");
    assert_eq!(managed["created_by"], json!("gpt-5.6-terra"));
}

#[test]
fn locks_release_reaches_admin_tool_bypassing_agent_gate() {
    let workspace = TestWorkspace::new();

    // `orbit.task.locks.release` is inactive on the agent tool surface, so
    // `orbit locks release` must reach it through the admin `runtime.run_tool`
    // bypass. An unknown reservation yields a structured `released: false`, NOT
    // the `ensure_tool_agent_facing` rejection.
    let output = workspace.run(
        &["locks", "release", "no-such-reservation"],
        "locks release",
    );
    let value: Value = serde_json::from_slice(&output.stdout).expect("release JSON");
    assert_eq!(value["released"], json!(false));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("inactive on the agent tool surface"),
        "locks-release must bypass the agent-surface gate:\n{stderr}"
    );
}

struct TestWorkspace {
    _temp: TempDir,
    home: std::path::PathBuf,
    work: std::path::PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let temp = tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let work = temp.path().join("work");
        fs::create_dir_all(&home).expect("create home");
        fs::create_dir_all(&work).expect("create work");

        let workspace = Self {
            _temp: temp,
            home,
            work,
        };
        workspace.run(
            &["workspace", "init", "--name", "trimmed-surface-test"],
            "initialize workspace",
        );
        workspace
    }

    fn add_task(&self, title: &str) -> String {
        let output = self.run(
            &[
                "task",
                "add",
                "--title",
                title,
                "--description",
                "Created by the trimmed-surface integration test.",
                "--acceptance-criteria",
                "status lands where the update says",
                "--json",
            ],
            "add task",
        );
        let task: Value = serde_json::from_slice(&output.stdout).expect("task add JSON");
        assert_eq!(task["status"], json!("proposed"));
        task["id"].as_str().expect("task id").to_string()
    }

    fn drive_to_done(&self, id: &str) {
        self.run(&["task", "update", id, "--status", "backlog"], "approve");
        self.run(
            &[
                "task",
                "update",
                id,
                "--plan",
                "1) do it",
                "--status",
                "in-progress",
            ],
            "start",
        );
        self.run(
            &[
                "task",
                "update",
                id,
                "--execution-summary",
                "did it",
                "--status",
                "review",
            ],
            "to review",
        );
        self.run(&["task", "update", id, "--status", "done"], "to done");
    }

    fn task_json(&self, args: &[&str]) -> Value {
        let output = self.run(args, "task JSON command");
        serde_json::from_slice(&output.stdout).expect("task JSON output")
    }

    fn run(&self, args: &[&str], label: &str) -> Output {
        let output = self.run_raw(args);
        assert!(
            output.status.success(),
            "{label} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn run_raw(&self, args: &[&str]) -> Output {
        run_orbit(&self.work, &self.home, args)
    }
}

fn run_orbit(cwd: &Path, home: &Path, args: &[&str]) -> Output {
    let mut command = cargo_bin_cmd!("orbit");
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("ORBIT_ROOT")
        .env_remove("ORBIT_AGENT_NAME")
        .env_remove("ORBIT_AGENT_MODEL")
        .env_remove("ORBIT_MANAGED_RUN_CONTEXT")
        .args(args);
    command.output().expect("run orbit")
}

fn run_orbit_with_identity(
    cwd: &Path,
    home: &Path,
    args: &[&str],
    agent: &str,
    model: &str,
) -> Output {
    let mut command = cargo_bin_cmd!("orbit");
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("ORBIT_ROOT")
        .env("ORBIT_AGENT_NAME", agent)
        .env("ORBIT_AGENT_MODEL", model)
        .env("ORBIT_MANAGED_RUN_CONTEXT", "1")
        .args(args);
    command.output().expect("run orbit with managed identity")
}
