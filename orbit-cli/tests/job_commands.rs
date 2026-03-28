use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::path::Path;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn orbit_in(dir: &Path) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("orbit").expect("binary exists");
    cmd.current_dir(dir);
    cmd.env("HOME", dir);
    cmd.env("USERPROFILE", dir);
    cmd.env("ORBIT_ROOT", dir.join(".orbit"));
    cmd
}

#[cfg(unix)]
fn orbit_process_in(dir: &Path) -> std::process::Command {
    let orbit_bin = assert_cmd::cargo::cargo_bin!("orbit");
    let mut cmd = std::process::Command::new(orbit_bin);
    cmd.current_dir(dir);
    cmd.env("HOME", dir);
    cmd.env("USERPROFILE", dir);
    cmd.env("ORBIT_ROOT", dir.join(".orbit"));
    cmd
}

fn add_activity(dir: &Path, id: &str) -> String {
    add_activity_with_input_schema(dir, id, "{}")
}

fn add_activity_with_input_schema(dir: &Path, id: &str, input_schema: &str) -> String {
    let output = orbit_in(dir)
        .args([
            "activity",
            "add",
            "--id",
            id,
            "--type",
            "agent_invoke",
            "--description",
            "test spec",
            "--input-schema",
            input_schema,
            "--output-schema",
            "{}",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf8").trim().to_string()
}

fn add_job(dir: &Path, target_id: &str, agent_cli: &str) -> String {
    let output = orbit_in(dir)
        .args([
            "job",
            "add",
            "--target-id",
            target_id,
            "--agent-cli",
            agent_cli,
            "--timeout",
            "30s",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf8").trim().to_string()
}

fn add_job_with_model(dir: &Path, target_id: &str, agent_cli: &str, model: &str) -> String {
    let output = orbit_in(dir)
        .args([
            "job",
            "add",
            "--target-id",
            target_id,
            "--agent-cli",
            agent_cli,
            "--model",
            model,
            "--timeout",
            "30s",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf8").trim().to_string()
}

fn add_job_with_default_timeout(dir: &Path, target_id: &str, agent_cli: &str) -> String {
    let output = orbit_in(dir)
        .args([
            "job",
            "add",
            "--target-id",
            target_id,
            "--agent-cli",
            agent_cli,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf8").trim().to_string()
}

fn add_job_with_max_active_runs(
    dir: &Path,
    target_id: &str,
    agent_cli: &str,
    max_active_runs: u32,
) -> String {
    let output = orbit_in(dir)
        .args([
            "job",
            "add",
            "--target-id",
            target_id,
            "--agent-cli",
            agent_cli,
            "--timeout",
            "30s",
            "--max-active-runs",
            &max_active_runs.to_string(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf8").trim().to_string()
}

fn add_task(dir: &Path, title: &str) -> String {
    let output = orbit_in(dir)
        .args(["task", "add", "--title", title, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let task: Value = serde_json::from_slice(&output).expect("task json");
    task["id"].as_str().expect("task id").to_string()
}

fn write_mock_agent(dir: &Path) -> String {
    let path = dir.join("mock-agent");
    std::fs::write(
        &path,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    )
    .expect("write mock agent");
    #[cfg(unix)]
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod mock agent");
    path.to_string_lossy().to_string()
}

fn write_failing_agent(dir: &Path) -> String {
    let path = dir.join("mock-agent");
    std::fs::write(&path, "#!/bin/sh\necho 'network down' 1>&2\nexit 1\n")
        .expect("write failing agent");
    #[cfg(unix)]
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod failing agent");
    path.to_string_lossy().to_string()
}

fn write_stdin_capturing_agent(dir: &Path, stdin_capture: &Path) -> String {
    let path = dir.join("mock-agent");
    let script = format!(
        "#!/bin/sh\ncat > \"{stdin}\"\nprintf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null,\"durationMs\":1}}'\n",
        stdin = stdin_capture.to_string_lossy(),
    );
    std::fs::write(&path, script).expect("write capturing agent");
    #[cfg(unix)]
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod capturing agent");
    path.to_string_lossy().to_string()
}

#[cfg(unix)]
fn write_signal_trapping_agent(dir: &Path, started: &Path, terminated: &Path) -> String {
    let path = dir.join("mock-agent");
    let script = format!(
        "#!/bin/sh\n\
trap 'printf term > \"{terminated}\"; exit 0' TERM\n\
trap 'printf int > \"{terminated}\"; exit 0' INT\n\
printf ready > \"{started}\"\n\
while :; do\n\
  sleep 1\n\
done\n",
        started = started.to_string_lossy(),
        terminated = terminated.to_string_lossy(),
    );
    std::fs::write(&path, script).expect("write signal-trapping agent");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod signal-trapping agent");
    path.to_string_lossy().to_string()
}

#[cfg(unix)]
fn wait_for<F>(timeout: Duration, mut predicate: F) -> bool
where
    F: FnMut() -> bool,
{
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    predicate()
}

#[cfg(unix)]
fn wait_for_running_run_id(dir: &Path, job_id: &str, timeout: Duration) -> String {
    let mut run_id = None;
    let found = wait_for(timeout, || {
        let output = orbit_in(dir)
            .args([
                "job-run", "list", "--job", job_id, "--status", "running", "--json",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let runs: Value = serde_json::from_slice(&output).expect("running run list json");
        run_id = runs
            .as_array()
            .and_then(|values| values.first())
            .and_then(|value| value.get("run_id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        run_id.is_some()
    });
    assert!(found, "expected a running job run for job '{job_id}'");
    run_id.expect("run id")
}

#[cfg(unix)]
fn wait_for_process_exit(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("try_wait") {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
fn job_add_list_show_json_flow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_id = add_activity(dir.path(), "spec-cli-list");

    let job_id = add_job(dir.path(), &spec_id, "mock-agent");
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
    assert_eq!(show["steps"][0]["target_type"], "activity");
    assert_eq!(show["steps"][0]["target_id"], spec_id);
    assert!(show["steps"][0].get("model").is_none());
    assert_eq!(show["state"], "enabled");
}

#[test]
fn job_add_show_json_includes_model_when_present() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_id = add_activity(dir.path(), "spec-cli-model");

    let job_id = add_job_with_model(dir.path(), &spec_id, "mock-agent", "gpt-5.4");

    let show_output = orbit_in(dir.path())
        .args(["job", "show", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["steps"][0]["model"], "gpt-5.4");
}

#[test]
fn job_add_defaults_timeout_to_twenty_minutes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_id = add_activity(dir.path(), "spec-cli-default-timeout");

    let job_id = add_job_with_default_timeout(dir.path(), &spec_id, "mock-agent");

    let show_output = orbit_in(dir.path())
        .args(["job", "show", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["steps"][0]["timeout_seconds"], 1200); // 20m default = 1200 seconds
}

#[test]
fn job_add_persists_max_active_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_id = add_activity(dir.path(), "spec-cli-max-active-runs");

    let job_id = add_job_with_max_active_runs(dir.path(), &spec_id, "mock-agent", 3);

    let show_output = orbit_in(dir.path())
        .args(["job", "show", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["max_active_runs"], 3);
}

#[test]
fn job_run_creates_run_and_history_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_id = add_activity(dir.path(), "spec-cli-run");
    let agent_cli = write_mock_agent(dir.path());
    let job_id = add_job(dir.path(), &spec_id, &agent_cli);

    let run_output = orbit_in(dir.path())
        .args(["job", "run", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run: Value = serde_json::from_slice(&run_output).expect("run json");
    assert_eq!(run["job_id"], job_id);
    assert_eq!(run["state"], "success");
    assert_eq!(run["attempt"], 1);

    let history_output = orbit_in(dir.path())
        .args(["job", "history", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let history: Value = serde_json::from_slice(&history_output).expect("history json");
    let runs = history.as_array().expect("array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["state"], "success");
    assert_eq!(runs[0]["attempt"], 1);
    assert!(runs[0]["agent_response_json"].is_object());
}

#[test]
fn job_run_with_task_id_passes_input_to_agent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let stdin_capture = dir.path().join("job-run-stdin.json");
    let spec_id = add_activity_with_input_schema(
        dir.path(),
        "spec-cli-run-task-id",
        r#"{"type":"object","properties":{"task_id":{"type":"string"}},"additionalProperties":false}"#,
    );
    let agent_cli = write_stdin_capturing_agent(dir.path(), &stdin_capture);
    let job_id = add_job(dir.path(), &spec_id, &agent_cli);
    let task_id = add_task(dir.path(), "CLI task id propagation");

    orbit_in(dir.path())
        .args([
            "job",
            "run",
            &job_id,
            "--input",
            &format!("task_id={task_id}"),
            "--json",
        ])
        .assert()
        .success();

    let stdin_raw = std::fs::read_to_string(stdin_capture).expect("stdin capture");
    let payload: Value = serde_json::from_str(&stdin_raw).expect("valid stdin payload");
    assert_eq!(payload["input"]["task_id"], task_id);
}

#[test]
fn job_run_with_input_rejects_incompatible_activity_input_schema() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_id = add_activity_with_input_schema(
        dir.path(),
        "spec-cli-run-task-id-invalid",
        r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    );
    let agent_cli = write_mock_agent(dir.path());
    let job_id = add_job(dir.path(), &spec_id, &agent_cli);

    orbit_in(dir.path())
        .args(["job", "run", &job_id, "--input", "task_id=T123"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "job run input does not match activity",
        ))
        .stderr(predicate::str::contains("task_id"));
}

#[test]
fn job_delete_flow() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_id = add_activity(dir.path(), "spec-cli-state");
    let job_id = add_job(dir.path(), &spec_id, "mock-agent");

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

#[test]
fn job_delete_json_returns_deleted_true() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_id = add_activity(dir.path(), "spec-cli-delete-json");
    let job_id = add_job(dir.path(), &spec_id, "mock-agent");

    let output = orbit_in(dir.path())
        .args(["job", "delete", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let deleted: Value = serde_json::from_slice(&output).expect("delete json");
    assert_eq!(deleted["id"], job_id);
    assert_eq!(deleted["deleted"], true);
}

#[test]
fn job_run_failure_json_includes_error_details() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_id = add_activity(dir.path(), "spec-cli-run-fail");
    let agent_cli = write_failing_agent(dir.path());
    let job_id = add_job(dir.path(), &spec_id, &agent_cli);

    let run_output = orbit_in(dir.path())
        .args(["job", "run", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run: Value = serde_json::from_slice(&run_output).expect("run json");
    assert_eq!(run["job_id"], job_id);
    assert_eq!(run["state"], "failed");
    assert_eq!(run["error_code"], "AGENT_INVOCATION_FAILED");
    assert!(
        run["error_message"]
            .as_str()
            .unwrap_or_default()
            .contains("network down")
    );
}

#[cfg(unix)]
#[test]
fn job_run_cancel_terminates_agent_subprocess() {
    let dir = tempfile::tempdir().expect("tempdir");
    let started = dir.path().join("agent-started");
    let terminated = dir.path().join("agent-terminated");
    let spec_id = add_activity(dir.path(), "spec-cli-cancel-signal");
    let agent_cli = write_signal_trapping_agent(dir.path(), &started, &terminated);
    let job_id = add_job(dir.path(), &spec_id, &agent_cli);

    let mut run_child = orbit_process_in(dir.path())
        .args(["job", "run", &job_id, "--json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn orbit job run");

    assert!(
        wait_for(Duration::from_secs(5), || started.exists()),
        "agent subprocess never reported startup"
    );
    let run_id = wait_for_running_run_id(dir.path(), &job_id, Duration::from_secs(5));

    orbit_in(dir.path())
        .args(["job-run", "cancel", &run_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Cancelled job run"));

    assert!(
        wait_for(Duration::from_secs(5), || terminated.exists()),
        "agent subprocess never observed SIGTERM after cancel"
    );
    let _status = wait_for_process_exit(&mut run_child, Duration::from_secs(5))
        .expect("job run command did not exit after cancellation");

    let show_output = orbit_in(dir.path())
        .args(["job-run", "show", &run_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_ne!(show["state"], "running");
    assert_ne!(show["state"], "pending");
}

#[cfg(unix)]
#[test]
fn job_run_sigint_terminates_agent_subprocess() {
    let dir = tempfile::tempdir().expect("tempdir");
    let started = dir.path().join("agent-started");
    let terminated = dir.path().join("agent-terminated");
    let spec_id = add_activity(dir.path(), "spec-cli-sigint-signal");
    let agent_cli = write_signal_trapping_agent(dir.path(), &started, &terminated);
    let job_id = add_job(dir.path(), &spec_id, &agent_cli);

    let mut run_child = orbit_process_in(dir.path())
        .args(["job", "run", &job_id, "--json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn orbit job run");

    assert!(
        wait_for(Duration::from_secs(5), || started.exists()),
        "agent subprocess never reported startup"
    );
    let run_id = wait_for_running_run_id(dir.path(), &job_id, Duration::from_secs(5));

    let status = std::process::Command::new("kill")
        .args(["-INT", &run_child.id().to_string()])
        .status()
        .expect("send SIGINT");
    assert!(status.success(), "expected kill -INT to succeed");

    assert!(
        wait_for(Duration::from_secs(5), || terminated.exists()),
        "agent subprocess never observed termination after SIGINT"
    );
    let _status = wait_for_process_exit(&mut run_child, Duration::from_secs(5))
        .expect("job run command did not exit after SIGINT");

    let show_output = orbit_in(dir.path())
        .args(["job-run", "show", &run_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_ne!(show["state"], "running");
    assert_ne!(show["state"], "pending");
}

#[test]
fn job_run_top_level_list_and_show_work() {
    let dir = tempfile::tempdir().expect("tempdir");
    let success_spec = add_activity(dir.path(), "spec-cli-job-run-success");
    let failed_spec = add_activity(dir.path(), "spec-cli-job-run-failed");

    let success_agent = write_mock_agent(dir.path());
    let success_job_id = add_job(dir.path(), &success_spec, &success_agent);

    let success_run_output = orbit_in(dir.path())
        .args(["job", "run", &success_job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let success_run: Value = serde_json::from_slice(&success_run_output).expect("success run json");

    let failed_agent = write_failing_agent(dir.path());
    let failed_job_id = add_job(dir.path(), &failed_spec, &failed_agent);

    let failed_run_output = orbit_in(dir.path())
        .args(["job", "run", &failed_job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let failed_run: Value = serde_json::from_slice(&failed_run_output).expect("failed run json");

    let show_output = orbit_in(dir.path())
        .args([
            "job-run",
            "show",
            failed_run["run_id"].as_str().expect("run id"),
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["job_id"], failed_job_id);
    assert_eq!(show["run_id"], failed_run["run_id"]);
    assert_eq!(show["state"], "failed");

    let filtered_by_job_output = orbit_in(dir.path())
        .args(["job-run", "list", "--job", &success_job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let filtered_by_job: Value =
        serde_json::from_slice(&filtered_by_job_output).expect("job filtered json");
    let filtered_by_job_runs = filtered_by_job.as_array().expect("array");
    assert_eq!(filtered_by_job_runs.len(), 1);
    assert_eq!(filtered_by_job_runs[0]["run_id"], success_run["run_id"]);

    let failed_only_output = orbit_in(dir.path())
        .args(["job-run", "list", "--status", "failed", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let failed_only: Value = serde_json::from_slice(&failed_only_output).expect("failed list json");
    let failed_only_runs = failed_only.as_array().expect("array");
    assert_eq!(failed_only_runs.len(), 1);
    assert_eq!(failed_only_runs[0]["run_id"], failed_run["run_id"]);

    let limited_output = orbit_in(dir.path())
        .args(["job-run", "list", "--limit", "1", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let limited: Value = serde_json::from_slice(&limited_output).expect("limited json");
    assert_eq!(limited.as_array().expect("array").len(), 1);
}

#[test]
fn job_run_archive_and_delete_mutate_visibility() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_id = add_activity(dir.path(), "spec-cli-job-run-mutate");
    let agent_cli = write_mock_agent(dir.path());
    let job_id = add_job(dir.path(), &spec_id, &agent_cli);

    let run_output = orbit_in(dir.path())
        .args(["job", "run", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run: Value = serde_json::from_slice(&run_output).expect("run json");
    let run_id = run["run_id"].as_str().expect("run id").to_string();

    orbit_in(dir.path())
        .args(["job-run", "archive", &run_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Archived job run"));

    orbit_in(dir.path())
        .args(["job-run", "show", &run_id, "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("job run not found"));

    let list_output = orbit_in(dir.path())
        .args(["job-run", "list", "--job", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list: Value = serde_json::from_slice(&list_output).expect("list json");
    assert!(list.as_array().expect("array").is_empty());

    orbit_in(dir.path())
        .args(["job-run", "delete", &run_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted job run"));
}

#[test]
fn job_add_rejects_legacy_workflow_target_type() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args([
            "job",
            "add",
            "--target-type",
            "workflow",
            "--target-id",
            "wf-1",
            "--agent-cli",
            "mock-agent",
            "--timeout",
            "30s",
        ])
        .assert()
        .failure();
}

#[test]
fn job_add_with_named_id_uses_provided_id() {
    let dir = tempfile::tempdir().expect("tempdir");
    add_activity(dir.path(), "spec-named-id");

    let output = orbit_in(dir.path())
        .args([
            "job",
            "add",
            "--job-id",
            "job-my-named-job",
            "--target-id",
            "spec-named-id",
            "--agent-cli",
            "mock-agent",
            "--timeout",
            "30s",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let returned_id = String::from_utf8(output).expect("utf8").trim().to_string();
    assert_eq!(returned_id, "job-my-named-job");

    let show_output = orbit_in(dir.path())
        .args(["job", "show", "job-my-named-job", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["job_id"], "job-my-named-job");
    assert_eq!(show["steps"][0]["target_id"], "spec-named-id");
}

#[test]
fn job_add_duplicate_named_id_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    add_activity(dir.path(), "spec-dup-id");

    orbit_in(dir.path())
        .args([
            "job",
            "add",
            "--job-id",
            "job-duplicate",
            "--target-id",
            "spec-dup-id",
            "--agent-cli",
            "mock-agent",
        ])
        .assert()
        .success();

    orbit_in(dir.path())
        .args([
            "job",
            "add",
            "--job-id",
            "job-duplicate",
            "--target-id",
            "spec-dup-id",
            "--agent-cli",
            "mock-agent",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("job-duplicate"));
}

#[test]
fn seeded_default_jobs_are_listed_as_enabled_after_init() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path())
        .args(["init"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default_jobs_refreshed="));

    let list_output = orbit_in(dir.path())
        .args(["job", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list: Value = serde_json::from_slice(&list_output).expect("list json");
    let jobs = list.as_array().expect("jobs array");

    let seeded_job = jobs
        .iter()
        .find(|job| job["job_id"] == "job_review_tasks")
        .expect("seeded default job visible");
    assert_eq!(seeded_job["state"], "enabled");
}

#[test]
fn job_list_ops_returns_signal_tier_json() {
    let dir = tempfile::tempdir().expect("tempdir");

    orbit_in(dir.path()).args(["init"]).assert().success();

    let output = orbit_in(dir.path())
        .args(["job", "list", "--ops"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let jobs: Value = serde_json::from_slice(&output).expect("valid json");
    let jobs = jobs.as_array().expect("array");
    assert!(!jobs.is_empty());

    let job = &jobs[0];

    // Required signal fields present.
    assert!(job.get("job_id").is_some());
    assert!(job.get("target_id").is_some());
    assert!(job.get("state").is_some());

    // Verbose fields must be absent.
    assert!(job.get("agent_cli").is_none());
    assert!(job.get("timeout_seconds").is_none());
}

#[test]
fn job_list_shows_last_run_status_in_table_and_json() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_id = add_activity(dir.path(), "spec-list-lastrun");
    let agent_cli = write_mock_agent(dir.path());
    let job_id = add_job(dir.path(), &spec_id, &agent_cli);

    // Before any run: table should have LAST_RUN header and show "never".
    let output = orbit_in(dir.path())
        .args(["job", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).expect("utf8");
    assert!(
        text.contains("LAST_RUN"),
        "table header must include LAST_RUN column"
    );
    assert!(text.contains("never"), "unrun job must show 'never'");

    // After a successful run: table should show "success".
    orbit_in(dir.path())
        .args(["job", "run", &job_id])
        .assert()
        .success();

    let output = orbit_in(dir.path())
        .args(["job", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).expect("utf8");
    assert!(
        text.contains("success"),
        "run job must show 'success' in LAST_RUN"
    );

    // JSON output must include last_run_state and last_run_at.
    let json_output = orbit_in(dir.path())
        .args(["job", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let jobs: Value = serde_json::from_slice(&json_output).expect("json");
    let job = jobs
        .as_array()
        .expect("array")
        .iter()
        .find(|j| j["job_id"] == job_id)
        .expect("job found in list");
    assert_eq!(job["last_run_state"], "success");
    assert!(
        job["last_run_at"].is_string(),
        "last_run_at must be a timestamp string"
    );
}

#[test]
fn job_archive_subcommand_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    orbit_in(dir.path())
        .args(["job", "archive", "jrun-dummy"])
        .assert()
        .failure();
}

#[test]
fn job_delete_moves_file_to_disabled_subdir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_id = add_activity(dir.path(), "spec-delete-disabled");
    let job_id = add_job(dir.path(), &spec_id, "mock-agent");

    // Confirm the job YAML is present in the active jobs directory.
    let jobs_dir = dir.path().join(".orbit").join("jobs").join("jobs");
    let active_path = jobs_dir.join(format!("{job_id}.yaml"));
    assert!(
        active_path.exists(),
        "job yaml must exist in jobs/ before delete"
    );

    orbit_in(dir.path())
        .args(["job", "delete", &job_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted job"));

    // After delete, file must be under jobs/disabled/ not in jobs/.
    let disabled_path = jobs_dir.join("disabled").join(format!("{job_id}.yaml"));
    assert!(!active_path.exists(), "job yaml must be removed from jobs/");
    assert!(
        disabled_path.exists(),
        "job yaml must appear in jobs/disabled/"
    );
}

#[test]
fn job_delete_hides_job_from_list_but_show_still_works() {
    let dir = tempfile::tempdir().expect("tempdir");
    let spec_id = add_activity(dir.path(), "spec-delete-hide");
    let job_id = add_job(dir.path(), &spec_id, "mock-agent");

    orbit_in(dir.path())
        .args(["job", "delete", &job_id])
        .assert()
        .success();

    // job list must not show the deleted job.
    let list_output = orbit_in(dir.path())
        .args(["job", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list: Value = serde_json::from_slice(&list_output).expect("list json");
    assert!(
        !list
            .as_array()
            .unwrap()
            .iter()
            .any(|j| j["job_id"] == job_id),
        "deleted job must not appear in list"
    );

    // job show must still resolve the deleted job (it's in disabled/).
    let show_output = orbit_in(dir.path())
        .args(["job", "show", &job_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let show: Value = serde_json::from_slice(&show_output).expect("show json");
    assert_eq!(show["job_id"], job_id);
    assert_eq!(show["state"], "disabled");
}
