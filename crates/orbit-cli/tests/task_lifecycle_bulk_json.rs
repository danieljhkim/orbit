use std::fs;
use std::path::Path;
use std::process::Output;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

#[test]
fn approve_all_proposed_json_no_proposed_stdout_is_json() {
    assert_no_proposed_json_stdout("approve", &[]);
}

#[test]
fn reject_all_proposed_json_no_proposed_stdout_is_json() {
    assert_no_proposed_json_stdout("reject", &["--note", "not needed"]);
}

#[test]
fn approve_all_proposed_json_abort_stdout_is_json() {
    assert_aborted_json_stdout("approve", &[]);
}

#[test]
fn reject_all_proposed_json_abort_stdout_is_json() {
    assert_aborted_json_stdout("reject", &["--note", "not now"]);
}

fn assert_no_proposed_json_stdout(command: &str, extra_args: &[&str]) {
    let workspace = TestWorkspace::new();
    let output = workspace.run(
        &task_lifecycle_args(command, extra_args),
        None,
        "run bulk lifecycle command",
    );

    assert_json_array_stdout(&output);
    assert_stderr_contains(&output, "No proposed tasks found.");
}

fn assert_aborted_json_stdout(command: &str, extra_args: &[&str]) {
    let workspace = TestWorkspace::new();
    workspace.add_proposed_task();

    let output = workspace.run(
        &task_lifecycle_args(command, extra_args),
        Some("n\n"),
        "run bulk lifecycle command",
    );

    assert_json_array_stdout(&output);
    assert_stderr_contains(&output, "Proceed? [y/N]");
    assert_stderr_contains(&output, "Aborted.");
}

fn task_lifecycle_args<'a>(command: &'a str, extra_args: &'a [&'a str]) -> Vec<&'a str> {
    let mut args = vec!["task", command, "--all-proposed", "--json"];
    args.extend(extra_args);
    args
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
            &["workspace", "init", "--name", "bulk-json-test"],
            None,
            "initialize workspace",
        );
        workspace
    }

    fn add_proposed_task(&self) {
        let output = self.run(
            &[
                "task",
                "add",
                "--title",
                "Needs decision",
                "--description",
                "Created by the bulk lifecycle JSON integration test.",
                "--acceptance-criteria",
                "decision recorded",
                "--json",
            ],
            None,
            "add proposed task",
        );
        let task: Value = serde_json::from_slice(&output.stdout).expect("task add JSON");
        assert_eq!(task["status"], json!("proposed"));
    }

    fn run(&self, args: &[&str], stdin: Option<&str>, label: &str) -> Output {
        let output = run_orbit(&self.work, &self.home, args, stdin);
        assert!(
            output.status.success(),
            "{label} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

fn run_orbit(cwd: &Path, home: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut command = cargo_bin_cmd!("orbit");
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("ORBIT_ROOT")
        .args(args);
    if let Some(input) = stdin {
        command.write_stdin(input);
    }
    command.output().expect("run orbit")
}

fn assert_json_array_stdout(output: &Output) {
    let value: Value = serde_json::from_slice(&output.stdout).expect("stdout is pure JSON");
    assert_eq!(value, Value::Array(Vec::new()));
}

fn assert_stderr_contains(output: &Output, expected: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected),
        "stderr missing {expected:?}\nstderr:\n{stderr}"
    );
}
