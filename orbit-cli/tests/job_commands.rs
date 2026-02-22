use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::Path;

fn orbit_in(dir: &Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd
}

fn add_task_with_tool_calls(dir: &Path, title: &str, instructions_json: &str) -> String {
    let output = orbit_in(dir)
        .args([
            "task",
            "add",
            "--title",
            title,
            "--instructions",
            instructions_json,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf8").trim().to_string()
}

fn add_job(dir: &Path, task_id: &str, name: &str, schedule: &str) -> String {
    let output = orbit_in(dir)
        .args([
            "job",
            "add",
            "--task",
            task_id,
            "--schedule",
            schedule,
            "--name",
            name,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf8").trim().to_string()
}

#[test]
fn job_add_list_show_json_flow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let task_id = add_task_with_tool_calls(
        dir.path(),
        "job-cli",
        r#"{"tool_calls":[{"name":"time.now","input":{}}]}"#,
    );

    let job_id = add_job(dir.path(), &task_id, "cli-job", "every 1m");
    assert!(job_id.starts_with("job-"), "unexpected job id: {job_id}");

    let list_output = orbit_in(dir.path())
        .args(["job", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list: Value = serde_json::from_slice(&list_output).expect("list json");
    let arr = list.as_array().expect("array");
    assert!(arr.iter().any(|job| job["job_id"] == job_id));

    let show_output = orbit_in(dir.path())
        .args(["job", "show", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["job_id"], job_id);
    assert_eq!(show["task_id"], task_id);
    assert_eq!(show["schedule_spec"], "every 1m");
    assert_eq!(show["state"], "active");
}

#[test]
fn job_run_creates_session_and_history_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let task_id = add_task_with_tool_calls(
        dir.path(),
        "job-cli-run",
        r#"{"tool_calls":[{"name":"time.now","input":{}}]}"#,
    );
    let job_id = add_job(dir.path(), &task_id, "runner", "every 1m");

    let run_output = orbit_in(dir.path())
        .args(["job", "run", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run: Value = serde_json::from_slice(&run_output).expect("run json");
    assert_eq!(run["job_id"], job_id);
    assert_eq!(run["status"], "succeeded");

    let history_output = orbit_in(dir.path())
        .args(["job", "history", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let history: Value = serde_json::from_slice(&history_output).expect("history json");
    let sessions = history.as_array().expect("array");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["status"], "succeeded");
    assert_eq!(sessions[0]["trigger"], "manual");
}

#[test]
fn job_pause_resume_delete_flow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let task_id = add_task_with_tool_calls(
        dir.path(),
        "job-cli-state",
        r#"{"tool_calls":[{"name":"time.now","input":{}}]}"#,
    );
    let job_id = add_job(dir.path(), &task_id, "stateful", "every 1m");

    orbit_in(dir.path())
        .args(["job", "pause", &job_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Paused job"));

    let paused_output = orbit_in(dir.path())
        .args(["job", "show", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let paused: Value = serde_json::from_slice(&paused_output).expect("paused json");
    assert_eq!(paused["state"], "paused");

    orbit_in(dir.path())
        .args(["job", "resume", &job_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Resumed job"));

    let resumed_output = orbit_in(dir.path())
        .args(["job", "show", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let resumed: Value = serde_json::from_slice(&resumed_output).expect("resumed json");
    assert_eq!(resumed["state"], "active");

    orbit_in(dir.path())
        .args(["job", "delete", &job_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted job"));

    let list_output = orbit_in(dir.path())
        .args(["job", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list: Value = serde_json::from_slice(&list_output).expect("list json");
    let arr = list.as_array().expect("array");
    assert!(!arr.iter().any(|job| job["job_id"] == job_id));
}
