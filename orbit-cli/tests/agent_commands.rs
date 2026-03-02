use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;

fn orbit_in(dir: &Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd.env("HOME", dir);
    cmd.env("USERPROFILE", dir);
    cmd
}

fn add_task_with_instructions(dir: &Path, title: &str, instructions: &str) -> String {
    let output = orbit_in(dir)
        .args([
            "task",
            "add",
            "--title",
            title,
            "--instructions",
            instructions,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf8").trim().to_string()
}

#[test]
fn agent_run_executes_task_instruction_payload() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orbit_dir = dir.path().join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("create orbit dir");
    std::fs::write(
        orbit_dir.join("config.toml"),
        "[task.approval]\nrequired_for_agent = false\n",
    )
    .expect("write config");

    let task_id = add_task_with_instructions(
        dir.path(),
        "agent task",
        r#"{"tool_calls":[{"name":"time.now","input":{}}]}"#,
    );

    orbit_in(dir.path())
        .args(["agent", "run", "--task", &task_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("session_id="))
        .stdout(predicate::str::contains("status=completed"));
}

#[test]
fn agent_run_fails_for_invalid_task_instruction_payload() {
    let dir = tempfile::tempdir().expect("tempdir");
    let task_id = add_task_with_instructions(dir.path(), "bad payload", "not-json");

    orbit_in(dir.path())
        .args(["agent", "run", "--task", &task_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("agent run failed"));
}

#[test]
fn agent_run_requires_approval_when_enabled() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orbit_dir = dir.path().join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("create orbit dir");
    std::fs::write(
        orbit_dir.join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");

    let task_id = add_task_with_instructions(
        dir.path(),
        "needs approval",
        r#"{"tool_calls":[{"name":"time.now","input":{}}]}"#,
    );

    orbit_in(dir.path())
        .args(["agent", "run", "--task", &task_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("task requires approval"));
}

#[test]
fn agent_run_can_auto_approve_on_verbal_flag() {
    let dir = tempfile::tempdir().expect("tempdir");
    let orbit_dir = dir.path().join(".orbit");
    std::fs::create_dir_all(&orbit_dir).expect("create orbit dir");
    std::fs::write(
        orbit_dir.join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");

    let task_id = add_task_with_instructions(
        dir.path(),
        "verbal approval task",
        r#"{"tool_calls":[{"name":"time.now","input":{}}]}"#,
    );

    orbit_in(dir.path())
        .args([
            "agent",
            "run",
            "--task",
            &task_id,
            "--approve-on-verbal",
            "--approved-by",
            "agent",
            "--approval-note",
            "approved verbally",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("status=completed"));
}
