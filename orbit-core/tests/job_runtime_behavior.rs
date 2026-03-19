use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use chrono::{Duration as ChronoDuration, Utc};
use orbit_core::OrbitRuntime;
use orbit_core::command::activity::{ActivityAddParams, ActivityRunParams};
use orbit_core::command::job::JobAddParams;
use orbit_core::command::job_run::JobRunListParams;
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
use orbit_types::{
    JobRunState, JobStep, JobTargetType, OrbitError, TaskPriority, TaskStatus, TaskType,
};
use serde_json::json;
use tempfile::tempdir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn add_activity(runtime: &OrbitRuntime, id: &str) {
    add_activity_with_input_schema(runtime, id, json!({}));
}

fn add_activity_with_input_schema(
    runtime: &OrbitRuntime,
    id: &str,
    input_schema_json: serde_json::Value,
) {
    let _ = runtime
        .add_activity(ActivityAddParams {
            id: id.to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "runtime test spec".to_string(),
            input_schema_json,
            output_schema_json: json!({}),
            spec_config: json!({
                "instruction": "Run the scheduled runtime behavior test."
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add activity");
}

#[test]
fn add_activity_rejects_missing_skill_ref() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let result = runtime.add_activity(ActivityAddParams {
        id: "spec-missing-skill".to_string(),
        spec_type: "agent_invoke".to_string(),
        description: "missing skill".to_string(),
        input_schema_json: json!({}),
        output_schema_json: json!({}),
        spec_config: json!({
            "skill_refs": ["does-not-exist"]
        }),
        workspace_path: None,
        created_by: None,
    });
    assert!(result.is_err());
}

fn add_scheduled_activity(runtime: &OrbitRuntime, target_id: &str, agent_cli: &str) -> String {
    add_scheduled_activity_with_timeout(runtime, target_id, agent_cli, 10)
}

fn add_scheduled_activity_with_timeout(
    runtime: &OrbitRuntime,
    target_id: &str,
    agent_cli: &str,
    timeout_seconds: u64,
) -> String {
    runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: None,
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: target_id.to_string(),
                agent_cli: agent_cli.to_string(),
                timeout_seconds,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add job")
        .job_id
}

#[test]
fn activity_run_executes_without_persisted_job() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let args_capture = dir.path().join("activity-args.txt");
    let stdin_capture = dir.path().join("activity-stdin.json");
    let script_path = dir.path().join("mock-agent");
    let script = format!(
        "#!/bin/sh\nprintf '%s' \"$@\" > \"{args}\"\ncat > \"{stdin}\"\nprintf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null,\"durationMs\":3}}'\n",
        args = args_capture.to_string_lossy(),
        stdin = stdin_capture.to_string_lossy(),
    );
    let agent_cli = write_agent_script(&script_path, &script);

    add_activity(&runtime, "spec-direct-run");
    let result = runtime
        .run_activity_now(ActivityRunParams {
            activity_id: "spec-direct-run".to_string(),
            agent_cli,
            timeout_seconds: 10,
        })
        .expect("run activity");

    assert_eq!(result.activity_id, "spec-direct-run");
    assert_eq!(result.state, JobRunState::Success);
    assert_eq!(result.error_code, None);

    let args_raw = std::fs::read_to_string(args_capture).expect("args capture");
    assert!(args_raw.contains("--mode"));
    assert!(args_raw.contains("activity"));
    assert!(!args_raw.contains("--job-id"));

    let stdin_raw = std::fs::read_to_string(stdin_capture).expect("stdin capture");
    assert!(stdin_raw.contains("\"activity\""));
    assert!(!stdin_raw.contains("\"job\""));

    let audits = runtime.list_audits(25).expect("audits");
    assert!(
        audits
            .iter()
            .any(|audit| audit.event_type == "ActivityRunCompleted"),
        "direct activity execution should be auditable"
    );
}

#[test]
fn cli_command_activity_executes_without_agent_cli_and_captures_output_file() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("emit-json.sh");
    let script = "#!/bin/sh\nprintf '{\"cwd\":\"%s\"}' \"$PWD\" > \"$ORBIT_OUTPUT_FILE\"\n";
    std::fs::write(&script_path, script).expect("write script");
    #[cfg(unix)]
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod script");

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-cli-command".to_string(),
            spec_type: "cli_command".to_string(),
            description: "cli command runtime test".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "cwd": { "type": "string" }
                },
                "required": ["cwd"]
            }),
            spec_config: json!({
                "command": script_path.to_string_lossy().to_string(),
                "working_dir": "{{workspace_path}}",
                "expected_exit_codes": [0]
            }),
            workspace_path: Some(dir.path().to_string_lossy().into_owned()),
            created_by: None,
        })
        .expect("add activity");

    let job = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: None,
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "spec-cli-command".to_string(),
                agent_cli: String::new(),
                timeout_seconds: 30,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add job");

    let run = runtime.run_job_now(&job.job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let history = runtime.job_history(&job.job_id).expect("history");
    let response = history[0]
        .steps
        .last()
        .and_then(|step| step.agent_response_json.as_ref())
        .expect("stored step response");
    let expected_cwd = dir.path().canonicalize().expect("canonical cwd");
    assert_eq!(response["cwd"], expected_cwd.to_string_lossy().to_string());
}

#[test]
fn cli_command_failures_redact_sensitive_environment_values_from_error_messages() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    unsafe {
        std::env::set_var("TEST_SECRET_TOKEN", "token-value-to-hide");
    }

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-cli-secret-redaction".to_string(),
            spec_type: "cli_command".to_string(),
            description: "cli command redaction test".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            spec_config: json!({
                "command": "sh",
                "args": ["-c", "printf '%s' \"$TEST_SECRET_TOKEN\" >&2; exit 1"],
                "expected_exit_codes": [0]
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add activity");

    let job = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: None,
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "spec-cli-secret-redaction".to_string(),
                agent_cli: String::new(),
                timeout_seconds: 30,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add job");

    let run = runtime.run_job_now(&job.job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Failed);

    let history = runtime.job_history(&job.job_id).expect("history");
    let error_message = history[0]
        .steps
        .last()
        .and_then(|step| step.error_message.as_deref())
        .expect("error message");
    assert!(
        !error_message.contains("token-value-to-hide"),
        "error message must not retain sensitive env values: {error_message}"
    );
    assert!(
        error_message.contains("[REDACTED_ENV]"),
        "error message should indicate redaction: {error_message}"
    );
}

#[test]
fn agent_with_orphan_stdout_holder_does_not_hang() {
    // Reproduces: job-run stuck in `running` when agent exits successfully but leaves
    // orphan child processes that inherited the stdout pipe write end.
    // Without the fix, `wait_with_output()` hangs indefinitely because the orphan
    // keeps the pipe open. With the fix, the process group is killed after the agent
    // exits, closing all write ends and letting the run complete.
    let dir = tempdir().expect("tempdir");
    let runtime = Arc::new(OrbitRuntime::from_data_root(dir.path()).expect("runtime"));
    let script_path = dir.path().join("mock-agent");
    // The script: consumes stdin, spawns an orphan that holds stdout open for 60s,
    // prints a valid success envelope, then exits cleanly.
    let script = concat!(
        "#!/bin/sh\n",
        "cat > /dev/null\n", // consume stdin
        "sleep 60 &\n",      // orphan process inherits stdout pipe write end
        "printf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
        "exit 0\n",
    );
    let agent_cli = write_agent_script(&script_path, script);

    add_activity(&runtime, "spec-orphan-stdout");
    let job_id = add_scheduled_activity_with_timeout(
        &runtime,
        "spec-orphan-stdout",
        &agent_cli,
        10, // generous agent timeout; the hang occurs before timeout fires
    );

    // Run the job in a thread so we can detect a hang via channel timeout.
    let (tx, rx) = std::sync::mpsc::channel();
    let r = Arc::clone(&runtime);
    let j = job_id.clone();
    thread::spawn(move || {
        let _ = tx.send(r.run_job_now(&j));
    });

    // The agent exits almost immediately; the run must complete well within 5 seconds.
    let result = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("run_job_now must not hang when agent has orphan stdout-holding children")
        .expect("run must succeed");
    assert_eq!(result.state, JobRunState::Success);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobRunState::Success);
}

fn write_agent_script(path: &std::path::Path, body: &str) -> String {
    std::fs::write(path, body).expect("write script");
    #[cfg(unix)]
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod script");
    path.to_string_lossy().to_string()
}

fn write_runtime_config(data_root: &std::path::Path, content: &str) {
    std::fs::write(data_root.join("config.toml"), content).expect("write config");
}

fn init_git_repo(path: &std::path::Path) {
    let status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(path)
        .status()
        .expect("git init");
    assert!(status.success(), "git init must succeed");

    let status = Command::new("git")
        .args(["config", "user.email", "orbit@example.com"])
        .current_dir(path)
        .status()
        .expect("git config email");
    assert!(status.success(), "git config email must succeed");

    let status = Command::new("git")
        .args(["config", "user.name", "Orbit Test"])
        .current_dir(path)
        .status()
        .expect("git config name");
    assert!(status.success(), "git config name must succeed");
}

fn git_current_branch(path: &std::path::Path) -> String {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .expect("git rev-parse");
    assert!(output.status.success(), "git rev-parse must succeed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn git_commit_all(path: &std::path::Path, message: &str) {
    let status = Command::new("git")
        .args(["add", "--all"])
        .current_dir(path)
        .status()
        .expect("git add --all");
    assert!(status.success(), "git add --all must succeed");

    let status = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(path)
        .status()
        .expect("git commit");
    assert!(status.success(), "git commit must succeed");
}

fn prepend_path(dir: &std::path::Path) -> String {
    let current = std::env::var("PATH").unwrap_or_default();
    format!("{}:{}", dir.display(), current)
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobRunFileDocument {
    schema_version: u8,
    run: orbit_types::JobRun,
}

fn insert_stale_running_run(
    runtime: &OrbitRuntime,
    data_root: &std::path::Path,
    job_id: &str,
) -> String {
    let run = runtime.run_job_now(job_id).expect("seed run");
    // jrun.yaml lives inside the run bundle directory
    let jrun_path = data_root
        .join("jobs")
        .join("runs")
        .join(job_id)
        .join(&run.run_id)
        .join("jrun.yaml");
    let raw = std::fs::read_to_string(&jrun_path).expect("read jrun.yaml");
    let mut doc: JobRunFileDocument = serde_yaml::from_str(&raw).expect("parse run doc");
    let old_time = Utc::now() - ChronoDuration::hours(2);
    // Only manipulate run-level fields; step-level fields live in steps/*.yaml
    doc.run.state = JobRunState::Running;
    doc.run.started_at = Some(old_time);
    doc.run.finished_at = None;
    doc.run.duration_ms = None;
    doc.run.created_at = old_time;
    let updated = serde_yaml::to_string(&doc).expect("serialize run doc");
    std::fs::write(&jrun_path, updated).expect("write jrun.yaml");
    run.run_id
}

#[test]
fn job_run_executes_agent_and_records_success_run() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let args_capture = dir.path().join("args.txt");
    let stdin_capture = dir.path().join("stdin.json");
    let script_path = dir.path().join("mock-agent");
    let script = format!(
        "#!/bin/sh\nprintf '%s' \"$@\" > \"{args}\"\ncat > \"{stdin}\"\nprintf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null,\"durationMs\":1}}'\n",
        args = args_capture.to_string_lossy(),
        stdin = stdin_capture.to_string_lossy(),
    );
    let agent_cli = write_agent_script(&script_path, &script);

    add_activity(&runtime, "spec-success");
    let job_id = add_scheduled_activity(&runtime, "spec-success", &agent_cli);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobRunState::Success);
    assert_eq!(history[0].attempt, 1);
    assert!(
        history[0]
            .steps
            .last()
            .and_then(|s| s.agent_response_json.as_ref())
            .is_some()
    );

    let args_raw = std::fs::read_to_string(args_capture).expect("args capture");
    assert!(args_raw.contains("--output"));
    assert!(args_raw.contains("json"));
    assert!(args_raw.contains("--target-type"));

    let stdin_raw = std::fs::read_to_string(stdin_capture).expect("stdin capture");
    assert!(stdin_raw.contains("\"schemaVersion\":1"));
    assert!(stdin_raw.contains("\"activity\""));
    assert!(stdin_raw.contains("\"skills\""));
    assert!(stdin_raw.contains("\"input\""));
    assert!(stdin_raw.contains("\"memory\""));
    assert!(stdin_raw.contains("\"instruction\":\"Run the scheduled runtime behavior test.\""));
}

#[test]
fn run_job_now_with_input_passes_manual_input_to_agent() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let stdin_capture = dir.path().join("job-stdin.json");
    let script_path = dir.path().join("mock-agent");
    let script = format!(
        "#!/bin/sh\ncat > \"{stdin}\"\nprintf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null,\"durationMs\":1}}'\n",
        stdin = stdin_capture.to_string_lossy(),
    );
    let agent_cli = write_agent_script(&script_path, &script);

    add_activity_with_input_schema(
        &runtime,
        "spec-success-with-input",
        json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string" }
            },
            "additionalProperties": false
        }),
    );
    let job_id = add_scheduled_activity(&runtime, "spec-success-with-input", &agent_cli);

    let run = runtime
        .run_job_now_with_input(&job_id, json!({ "task_id": "T123" }))
        .expect("run job");

    assert_eq!(run.state, JobRunState::Success);
    let stdin_raw = std::fs::read_to_string(stdin_capture).expect("stdin capture");
    let payload: serde_json::Value = serde_json::from_str(&stdin_raw).expect("valid stdin payload");
    assert_eq!(payload["input"]["task_id"], "T123");
}

#[test]
fn run_job_now_uses_job_default_input_when_manual_input_is_absent() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let stdin_capture = dir.path().join("job-default-input.json");
    let script_path = dir.path().join("mock-agent");
    let script = format!(
        "#!/bin/sh\ncat > \"{stdin}\"\nprintf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null,\"durationMs\":1}}'\n",
        stdin = stdin_capture.to_string_lossy(),
    );
    let agent_cli = write_agent_script(&script_path, &script);

    add_activity_with_input_schema(
        &runtime,
        "spec-default-input",
        json!({
            "type": "object",
            "properties": {
                "base": { "type": "string" }
            },
            "required": ["base"],
            "additionalProperties": false
        }),
    );

    let job = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({ "base": "main" })),
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "spec-default-input".to_string(),
                agent_cli,
                timeout_seconds: 10,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add job");

    let run = runtime.run_job_now(&job.job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let stdin_raw = std::fs::read_to_string(stdin_capture).expect("stdin capture");
    let payload: serde_json::Value = serde_json::from_str(&stdin_raw).expect("valid stdin payload");
    assert_eq!(payload["input"]["base"], "main");
}

#[test]
fn run_job_now_with_input_overrides_job_default_input() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let stdin_capture = dir.path().join("job-default-override.json");
    let script_path = dir.path().join("mock-agent");
    let script = format!(
        "#!/bin/sh\ncat > \"{stdin}\"\nprintf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null,\"durationMs\":1}}'\n",
        stdin = stdin_capture.to_string_lossy(),
    );
    let agent_cli = write_agent_script(&script_path, &script);

    add_activity_with_input_schema(
        &runtime,
        "spec-default-override",
        json!({
            "type": "object",
            "properties": {
                "base": { "type": "string" },
                "mode": { "type": "string" }
            },
            "required": ["base", "mode"],
            "additionalProperties": false
        }),
    );

    let job = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({ "base": "main", "mode": "auto" })),
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "spec-default-override".to_string(),
                agent_cli,
                timeout_seconds: 10,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add job");

    let run = runtime
        .run_job_now_with_input(&job.job_id, json!({ "base": "release" }))
        .expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let stdin_raw = std::fs::read_to_string(stdin_capture).expect("stdin capture");
    let payload: serde_json::Value = serde_json::from_str(&stdin_raw).expect("valid stdin payload");
    assert_eq!(payload["input"]["base"], "release");
    assert_eq!(payload["input"]["mode"], "auto");
}

#[test]
fn run_job_now_with_input_rejects_schema_mismatch() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    add_activity_with_input_schema(
        &runtime,
        "spec-invalid-input",
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    );
    let job_id = add_scheduled_activity(&runtime, "spec-invalid-input", "mock-agent");

    let err = runtime
        .run_job_now_with_input(&job_id, json!({ "task_id": "T123" }))
        .expect_err("schema mismatch should fail");

    assert!(matches!(err, OrbitError::InvalidInput(_)));
    assert!(
        err.to_string()
            .contains("job run input does not match activity")
    );
}

#[test]
fn run_job_now_finalizes_failed_when_pre_step_setup_errors_after_running() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    add_activity_with_input_schema(
        &runtime,
        "spec-invalid-default-input",
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    );

    let job = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({ "base": "main" })),
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "spec-invalid-default-input".to_string(),
                agent_cli: "mock-agent".to_string(),
                timeout_seconds: 10,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add job");

    let err = runtime
        .run_job_now(&job.job_id)
        .expect_err("schema mismatch should fail after run starts");

    assert!(matches!(err, OrbitError::InvalidInput(_)));
    assert!(
        err.to_string()
            .contains("job run input does not match activity"),
        "{err}"
    );

    let history = runtime.job_history(&job.job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobRunState::Failed);
    assert!(history[0].started_at.is_some());
    assert!(history[0].finished_at.is_some());
    assert_eq!(history[0].steps.len(), 1);
    assert_eq!(history[0].steps[0].state, JobRunState::Failed);
    assert_eq!(
        history[0].steps[0].error_code.as_deref(),
        Some("ACTIVITY_EXECUTION_FAILED")
    );
    assert!(
        history[0].steps[0]
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("job run input does not match activity"),
    );

    let running = runtime
        .list_job_runs(JobRunListParams {
            job_id: Some(job.job_id.clone()),
            state: Some(JobRunState::Running),
            since: None,
            limit: None,
        })
        .expect("running list");
    assert!(running.is_empty(), "run should no longer appear as running");
}

#[test]
fn job_run_omits_identity_block_from_agent_envelope() {
    let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
    let repo_orbit = tempdir().expect("repo orbit");
    let home = tempdir().expect("home");
    let previous_home = std::env::var("HOME").ok();
    let previous_userprofile = std::env::var("USERPROFILE").ok();
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("USERPROFILE", home.path());
    }

    let runtime = OrbitRuntime::from_data_root(repo_orbit.path()).expect("runtime");
    let stdin_capture = repo_orbit.path().join("job-stdin.json");
    let script_path = repo_orbit.path().join("mock-agent");
    let script = format!(
        "#!/bin/sh\ncat > \"{stdin}\"\nprintf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null,\"durationMs\":1}}'\n",
        stdin = stdin_capture.to_string_lossy(),
    );
    let agent_cli = write_agent_script(&script_path, &script);

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-envelope".to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "envelope runtime test".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            spec_config: json!({
                "instruction": "Run without an identity block."
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add activity");
    let job_id = add_scheduled_activity(&runtime, "spec-envelope", &agent_cli);

    let run_result = runtime.run_job_now(&job_id);
    match previous_home {
        Some(value) => unsafe { std::env::set_var("HOME", value) },
        None => unsafe { std::env::remove_var("HOME") },
    }
    match previous_userprofile {
        Some(value) => unsafe { std::env::set_var("USERPROFILE", value) },
        None => unsafe { std::env::remove_var("USERPROFILE") },
    }

    let run = run_result.expect("run job");
    assert_eq!(run.state, JobRunState::Success);
    let stdin_raw = std::fs::read_to_string(stdin_capture).expect("stdin capture");
    let payload: serde_json::Value = serde_json::from_str(&stdin_raw).expect("valid stdin");
    assert!(payload.get("identity").is_none());
}

#[test]
fn invalid_agent_json_with_zero_exit_falls_back_to_success() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(&script_path, "#!/bin/sh\nprintf 'not-json'\n");

    add_activity(&runtime, "spec-protocol");
    let job_id = add_scheduled_activity(&runtime, "spec-protocol", &agent_cli);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobRunState::Success);
    assert!(
        history[0]
            .steps
            .last()
            .and_then(|s| s.error_code.as_deref())
            .is_none()
    );

    let audits = runtime.list_audits(25).expect("audits");
    assert!(
        !audits
            .iter()
            .any(|audit| audit.event_type == "JobProtocolViolation")
    );
}

#[test]
fn invocation_failure_with_stderr_marks_run_failed_with_invocation_error() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\necho 'network down' 1>&2\nexit 1\n",
    );

    add_activity(&runtime, "spec-invocation");
    let job_id = add_scheduled_activity(&runtime, "spec-invocation", &agent_cli);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Failed);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobRunState::Failed);
    assert_eq!(
        history[0]
            .steps
            .last()
            .and_then(|s| s.error_code.as_deref()),
        Some("AGENT_INVOCATION_FAILED")
    );
    assert!(
        history[0]
            .steps
            .last()
            .and_then(|s| s.error_message.as_deref())
            .unwrap_or_default()
            .contains("network down")
    );
}

#[test]
fn codex_job_run_fails_fast_when_required_env_var_is_not_allowlisted() {
    let dir = tempdir().expect("tempdir");
    write_runtime_config(
        dir.path(),
        r#"[execution.env]
inherit = false
pass = ["PATH"]
"#,
    );
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("codex");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_activity(&runtime, "spec-codex-missing-env");
    let job_id = add_scheduled_activity(&runtime, "spec-codex-missing-env", &agent_cli);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Failed);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(
        history[0]
            .steps
            .last()
            .and_then(|s| s.error_code.as_deref()),
        Some("AGENT_INVOCATION_FAILED")
    );
    let message = history[0]
        .steps
        .last()
        .and_then(|s| s.error_message.as_deref())
        .unwrap_or_default();
    assert!(message.contains("HOME"));
    assert!(message.contains("config.toml"));
}

#[test]
fn codex_job_run_uses_workspace_write_sandbox() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let args_capture = dir.path().join("codex-args.txt");
    let script_path = dir.path().join("codex");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{args}\"\ncat > /dev/null\nprintf 'progress on stderr\\n' 1>&2\n",
        args = args_capture.display(),
    );
    let agent_cli = write_agent_script(&script_path, &script);

    add_activity(&runtime, "spec-codex-sandbox");
    let job_id = add_scheduled_activity(&runtime, "spec-codex-sandbox", &agent_cli);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let args = std::fs::read_to_string(args_capture).expect("read args");
    let captured: Vec<&str> = args.lines().collect();
    assert_eq!(captured[0..3], ["exec", "--sandbox", "workspace-write"]);
    assert!(!captured.contains(&"--output-schema"));
}

#[test]
fn codex_job_run_can_enable_approval_requests_via_runtime_config() {
    let dir = tempdir().expect("tempdir");
    write_runtime_config(
        dir.path(),
        r#"[execution.codex]
approval_policy = "on-request"
"#,
    );
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let args_capture = dir.path().join("codex-args.txt");
    let script_path = dir.path().join("codex");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"{args}\"\ncat > /dev/null\nprintf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null,\"durationMs\":1}}'\n",
        args = args_capture.display(),
    );
    let agent_cli = write_agent_script(&script_path, &script);

    add_activity(&runtime, "spec-codex-approval");
    let job_id = add_scheduled_activity(&runtime, "spec-codex-approval", &agent_cli);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let args = std::fs::read_to_string(args_capture).expect("read args");
    let captured: Vec<&str> = args.lines().collect();
    assert_eq!(
        captured[0..5],
        [
            "--ask-for-approval",
            "on-request",
            "exec",
            "--sandbox",
            "workspace-write",
        ]
    );
}

#[test]
fn successful_commit_request_is_executed_via_git_tools() {
    let dir = tempdir().expect("tempdir");
    init_git_repo(dir.path());
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let committed = dir.path().join("committed.txt");
    let ignored = dir.path().join("ignored.txt");
    std::fs::write(&committed, "commit me").expect("write committed file");
    std::fs::write(&ignored, "leave me out").expect("write ignored file");

    let script_path = dir.path().join("mock-agent");
    let script = concat!(
        "#!/bin/sh\n",
        "cat >/dev/null\n",
        "printf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{\"commit\":{\"message\":\"feat: commit selected file\",\"files\":[\"committed.txt\"]}},\"error\":null,\"durationMs\":1}'\n",
    );
    let agent_cli = write_agent_script(&script_path, script);

    add_activity(&runtime, "spec-commit-request");
    let job_id = add_scheduled_activity(&runtime, "spec-commit-request", &agent_cli);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let log = Command::new("git")
        .args(["log", "-1", "--pretty=%B"])
        .current_dir(dir.path())
        .output()
        .expect("git log");
    assert_eq!(
        String::from_utf8_lossy(&log.stdout).trim(),
        "feat: commit selected file"
    );

    let files = Command::new("git")
        .args(["show", "--name-only", "--pretty=", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git show");
    let changed = String::from_utf8_lossy(&files.stdout);
    assert!(changed.contains("committed.txt"));
    assert!(!changed.contains("ignored.txt"));
}

#[test]
fn commit_request_excludes_preexisting_staged_changes() {
    let dir = tempdir().expect("tempdir");
    init_git_repo(dir.path());
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let committed = dir.path().join("committed.txt");
    let unrelated = dir.path().join("unrelated.txt");
    std::fs::write(&committed, "commit me").expect("write committed file");
    std::fs::write(&unrelated, "leave staged").expect("write unrelated file");

    let stage_unrelated = Command::new("git")
        .args(["add", "--", "unrelated.txt"])
        .current_dir(dir.path())
        .status()
        .expect("git add unrelated");
    assert!(stage_unrelated.success(), "git add unrelated must succeed");

    let script_path = dir.path().join("mock-agent");
    let script = concat!(
        "#!/bin/sh\n",
        "cat >/dev/null\n",
        "printf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{\"commit\":{\"message\":\"feat: commit selected file only\",\"files\":[\"committed.txt\"]}},\"error\":null,\"durationMs\":1}'\n",
    );
    let agent_cli = write_agent_script(&script_path, script);

    add_activity(&runtime, "spec-commit-request-isolated");
    let job_id = add_scheduled_activity(&runtime, "spec-commit-request-isolated", &agent_cli);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let files = Command::new("git")
        .args(["show", "--name-only", "--pretty=", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git show");
    let changed = String::from_utf8_lossy(&files.stdout);
    assert!(changed.contains("committed.txt"));
    assert!(!changed.contains("unrelated.txt"));

    let staged = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(dir.path())
        .output()
        .expect("git diff --cached");
    let staged_files = String::from_utf8_lossy(&staged.stdout);
    assert!(staged_files.contains("unrelated.txt"));
    assert!(!staged_files.contains("committed.txt"));
}

#[test]
fn malformed_commit_request_fails_as_protocol_violation() {
    let dir = tempdir().expect("tempdir");
    init_git_repo(dir.path());
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let script = concat!(
        "#!/bin/sh\n",
        "cat >/dev/null\n",
        "printf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{\"commit\":{\"message\":\"feat: empty files\",\"files\":[]}},\"error\":null,\"durationMs\":1}'\n",
    );
    let agent_cli = write_agent_script(&script_path, script);

    add_activity(&runtime, "spec-commit-protocol");
    let job_id = add_scheduled_activity(&runtime, "spec-commit-protocol", &agent_cli);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Failed);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(
        history[0]
            .steps
            .last()
            .and_then(|s| s.error_code.as_deref()),
        Some("AGENT_PROTOCOL_VIOLATION")
    );
    assert!(
        history[0]
            .steps
            .last()
            .and_then(|s| s.error_message.as_deref())
            .unwrap_or_default()
            .contains("result.commit.files must contain at least one path")
    );
}

#[test]
fn empty_stdout_timeout_marks_run_as_timeout() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(&script_path, "#!/bin/sh\nsleep 2\n");

    add_activity(&runtime, "spec-timeout");
    let job_id = add_scheduled_activity_with_timeout(&runtime, "spec-timeout", &agent_cli, 1);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Timeout);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history[0].state, JobRunState::Timeout);
    assert_eq!(
        history[0]
            .steps
            .last()
            .and_then(|s| s.error_code.as_deref()),
        Some("AGENT_TIMEOUT")
    );
    assert!(
        history[0]
            .steps
            .last()
            .and_then(|s| s.error_message.as_deref())
            .unwrap_or_default()
            .contains("timed out")
    );
}

#[test]
fn claude_job_run_fails_fast_when_required_env_var_is_not_allowlisted() {
    let dir = tempdir().expect("tempdir");
    write_runtime_config(
        dir.path(),
        r#"[execution.env]
inherit = false
pass = ["PATH"]
"#,
    );
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("claude");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_activity(&runtime, "spec-claude-missing-env");
    let job_id = add_scheduled_activity(&runtime, "spec-claude-missing-env", &agent_cli);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Failed);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(
        history[0]
            .steps
            .last()
            .and_then(|s| s.error_code.as_deref()),
        Some("AGENT_INVOCATION_FAILED")
    );
    let message = history[0]
        .steps
        .last()
        .and_then(|s| s.error_message.as_deref())
        .unwrap_or_default();
    assert!(message.contains("HOME"));
    assert!(message.contains("config.toml"));
}

#[test]
fn provider_required_env_present_reaches_protocol_validation() {
    let (provider, required_key) = if std::env::var("OPENAI_API_KEY").is_ok() {
        ("codex", "OPENAI_API_KEY")
    } else if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        ("claude", "ANTHROPIC_API_KEY")
    } else {
        return;
    };

    let dir = tempdir().expect("tempdir");
    write_runtime_config(
        dir.path(),
        &format!(
            "[execution.env]\ninherit = false\npass = [\"{required_key}\",\"HOME\",\"PATH\"]\n"
        ),
    );
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join(provider);
    let agent_cli = write_agent_script(&script_path, "#!/bin/sh\nprintf 'not-json'\n");

    add_activity(&runtime, "spec-provider-env-present");
    let job_id = add_scheduled_activity(&runtime, "spec-provider-env-present", &agent_cli);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history[0].state, JobRunState::Success);
    assert!(
        history[0]
            .steps
            .last()
            .and_then(|s| s.error_code.as_deref())
            .is_none()
    );
}

#[test]
fn run_job_now_rejects_when_active_run_exists() {
    let dir = tempdir().expect("tempdir");
    let runtime = Arc::new(OrbitRuntime::from_data_root(dir.path()).expect("runtime"));
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nsleep 0.5\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_activity(&runtime, "spec-active-lock");
    let job_id = add_scheduled_activity(&runtime, "spec-active-lock", &agent_cli);

    let r1 = Arc::clone(&runtime);
    let job_id_thread = job_id.clone();
    let handle = thread::spawn(move || r1.run_job_now(&job_id_thread));
    thread::sleep(std::time::Duration::from_millis(100));

    let err = runtime
        .run_job_now(&job_id)
        .expect_err("second run should be rejected while first is active");
    assert!(matches!(err, OrbitError::JobValidation(_)));
    assert!(err.to_string().contains("already has an active run"));

    let first = handle.join().expect("join");
    assert!(first.is_ok(), "first run should complete successfully");

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(
        history.len(),
        1,
        "second invocation must not insert a pending row"
    );
    assert_eq!(history[0].state, JobRunState::Success);
}

#[test]
fn job_history_recovers_stale_running_run_to_failed() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_activity(&runtime, "spec-history-stale");
    let job_id = add_scheduled_activity(&runtime, "spec-history-stale", &agent_cli);
    let stale_run_id = insert_stale_running_run(&runtime, dir.path(), &job_id);

    let history = runtime.job_history(&job_id).expect("history");
    let stale = history
        .iter()
        .find(|run| run.run_id == stale_run_id)
        .expect("stale run should exist");
    assert_eq!(stale.state, JobRunState::Failed);
    assert_eq!(
        stale.steps.last().and_then(|s| s.error_code.as_deref()),
        Some("AGENT_INVOCATION_FAILED")
    );
    assert!(
        stale
            .steps
            .last()
            .and_then(|s| s.error_message.as_deref())
            .unwrap_or_default()
            .contains("stale active run recovered")
    );
}

#[test]
fn run_job_now_recovers_stale_running_run_and_executes_new_attempt() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_activity(&runtime, "spec-run-now-stale");
    let job_id = add_scheduled_activity(&runtime, "spec-run-now-stale", &agent_cli);
    let stale_run_id = insert_stale_running_run(&runtime, dir.path(), &job_id);

    let result = runtime.run_job_now(&job_id).expect("run now");
    assert_eq!(result.state, JobRunState::Success);

    let history = runtime.job_history(&job_id).expect("history");
    assert!(
        history.iter().any(|run| run.run_id == stale_run_id),
        "stale run should still be present in history"
    );
    let stale = history
        .iter()
        .find(|run| run.run_id == stale_run_id)
        .expect("stale run should exist");
    assert_eq!(stale.state, JobRunState::Failed);
    assert_eq!(
        stale.steps.last().and_then(|s| s.error_code.as_deref()),
        Some("AGENT_INVOCATION_FAILED")
    );
    assert!(
        history.iter().any(|run| run.state == JobRunState::Success),
        "new attempt should complete successfully"
    );
}

#[test]
fn skill_meta_output_schema_violation_marks_run_failed() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let skill_dir = dir.path().join("skills").join("strict-schema");
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: strict-schema
description: Validate output shape.
---

# Strict Schema

## Purpose
Validate output shape.

## Behavioral Constraints
- Deterministic output only.

## Output Requirements
- ok
"#,
    )
    .expect("write skill");
    std::fs::write(
        skill_dir.join("meta.json"),
        r#"{
  "name": "Strict Schema",
  "version": "1.0.0",
  "type": "object",
  "required": ["ok"],
  "properties": {
    "ok": { "type": "boolean" }
  }
}"#,
    )
    .expect("write meta");

    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\ncat >/dev/null\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{\"wrong\":1},\"error\":null,\"durationMs\":1}'\n",
    );

    let _ = runtime
        .add_activity(ActivityAddParams {
            id: "spec-schema".to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "schema validation".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            spec_config: json!({
                "skill_refs": ["strict-schema"]
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add activity");
    let job_id = add_scheduled_activity(&runtime, "spec-schema", &agent_cli);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Failed);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobRunState::Failed);
    assert_eq!(
        history[0]
            .steps
            .last()
            .and_then(|s| s.error_code.as_deref()),
        Some("AGENT_PROTOCOL_VIOLATION")
    );
}

#[test]
fn skill_meta_complex_schema_keywords_are_enforced() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let skill_dir = dir.path().join("skills").join("strict-complex");
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: strict-complex
description: Validate advanced schema behavior.
---

# Strict Complex

## Purpose
Validate advanced schema behavior.

## Behavioral Constraints
- Deterministic output only.

## Output Requirements
- kind
"#,
    )
    .expect("write skill");
    std::fs::write(
        skill_dir.join("meta.json"),
        r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "oneOf": [
    {
      "required": ["kind", "a"],
      "properties": {
        "kind": { "const": "a" },
        "a": { "type": "integer" }
      },
      "additionalProperties": false
    },
    {
      "required": ["kind", "b"],
      "properties": {
        "kind": { "const": "b" },
        "b": { "type": "string" }
      },
      "additionalProperties": false
    }
  ]
}"#,
    )
    .expect("write meta");

    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\ncat >/dev/null\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{\"kind\":\"a\",\"extra\":1},\"error\":null,\"durationMs\":1}'\n",
    );

    let _ = runtime
        .add_activity(ActivityAddParams {
            id: "spec-complex-schema".to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "schema validation".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            spec_config: json!({
                "skill_refs": ["strict-complex"]
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add activity");
    let job_id = add_scheduled_activity(&runtime, "spec-complex-schema", &agent_cli);

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Failed);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobRunState::Failed);
    assert_eq!(
        history[0]
            .steps
            .last()
            .and_then(|s| s.error_code.as_deref()),
        Some("AGENT_PROTOCOL_VIOLATION")
    );
}

#[test]
fn claude_job_run_succeeds_with_mock_binary() {
    // Verifies end-to-end Claude invocation: provider detection, required flags,
    // ANTHROPIC_API_KEY env var availability, and successful run recording.
    let dir = tempdir().expect("tempdir");
    let args_capture = dir.path().join("claude-args.txt");

    // Mock claude binary: assert required flags are present, emit a success envelope.
    let script_path = dir.path().join("claude");
    let script = format!(
        concat!(
            "#!/bin/sh\n",
            "printf '%s' \"$@\" > \"{args}\"\n",
            "cat > /dev/null\n", // consume stdin
            "case \"$*\" in\n",
            "  *--permission-mode*bypassPermissions*--no-session-persistence*)\n",
            "    printf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null,\"durationMs\":1}}'\n",
            "    ;;\n",
            "  *)\n",
            "    echo \"missing required claude flags\" >&2\n",
            "    exit 1\n",
            "    ;;\n",
            "esac\n",
        ),
        args = args_capture.to_string_lossy(),
    );
    let agent_cli = write_agent_script(&script_path, &script);

    // Runtime config: hermetic env — HOME and PATH are sufficient for Claude Code auth.
    write_runtime_config(dir.path(), "[execution.env]\npass = [\"HOME\", \"PATH\"]\n");

    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    add_activity(&runtime, "spec-claude-run");

    let job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: None,
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "spec-claude-run".to_string(),
                agent_cli,
                timeout_seconds: 10,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add job")
        .job_id;

    let result = runtime
        .run_job_now(&job_id)
        .expect("claude job must succeed");

    assert_eq!(result.state, JobRunState::Success, "job must succeed");

    let args_raw = std::fs::read_to_string(args_capture).expect("args capture");
    assert!(
        args_raw.contains("-p"),
        "claude must be invoked with -p: {args_raw}"
    );
    assert!(
        args_raw.contains("--permission-mode") && args_raw.contains("bypassPermissions"),
        "claude must be invoked with --permission-mode bypassPermissions: {args_raw}"
    );
    assert!(
        args_raw.contains("--no-session-persistence"),
        "claude must be invoked with --no-session-persistence: {args_raw}"
    );
}

#[test]
fn run_job_now_executes_job_successfully() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_activity(&runtime, "spec-manual-run");
    let job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: None,
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "spec-manual-run".to_string(),
                agent_cli,
                timeout_seconds: 10,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add job")
        .job_id;

    let result = runtime
        .run_job_now(&job_id)
        .expect("manual job run must succeed");
    assert_eq!(result.state, JobRunState::Success);
}

#[test]
fn agent_step_result_fields_flow_into_next_step_input() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\ncat >/dev/null\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{\"task_id\":\"T123\"},\"error\":null,\"durationMs\":1}'\n",
    );

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-agent-output".to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "agent step output propagation".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            spec_config: json!({
                "instruction": "Return a task_id."
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add agent activity");

    let cli_script_path = dir.path().join("capture-input.sh");
    std::fs::write(
        &cli_script_path,
        "#!/bin/sh\nprintf '{\"seen_task_id\":\"%s\"}' \"$SEEN_TASK_ID\" > \"$ORBIT_OUTPUT_FILE\"\n",
    )
    .expect("write cli script");
    #[cfg(unix)]
    std::fs::set_permissions(&cli_script_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod cli script");

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-cli-consumer".to_string(),
            spec_type: "cli_command".to_string(),
            description: "consume task_id from prior step".to_string(),
            input_schema_json: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" }
                },
                "required": ["task_id"]
            }),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "seen_task_id": { "type": "string" }
                },
                "required": ["seen_task_id"]
            }),
            spec_config: json!({
                "command": cli_script_path.to_string_lossy().to_string(),
                "expected_exit_codes": [0],
                "env": {
                    "SEEN_TASK_ID": "{{input.task_id}}"
                }
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add cli activity");

    let job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: None,
            steps: vec![
                JobStep {
                    target_type: JobTargetType::Activity,
                    target_id: "spec-agent-output".to_string(),
                    agent_cli,
                    timeout_seconds: 10,
                    env_extra: vec![],
                },
                JobStep {
                    target_type: JobTargetType::Activity,
                    target_id: "spec-cli-consumer".to_string(),
                    agent_cli: String::new(),
                    timeout_seconds: 10,
                    env_extra: vec![],
                },
            ],
            initial_state_override: None,
        })
        .expect("add job")
        .job_id;

    let result = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(result.state, JobRunState::Success);

    let history = runtime.job_history(&job_id).expect("job history");
    let cli_output = history[0]
        .steps
        .get(1)
        .and_then(|step| step.agent_response_json.as_ref())
        .expect("cli step output");
    assert_eq!(cli_output["seen_task_id"], json!("T123"));
}

#[test]
fn agent_step_workspace_path_flows_into_cli_working_directory() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let workspace_dir = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace_dir).expect("create workspace");

    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        &format!(
            "#!/bin/sh\ncat >/dev/null\nprintf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{\"task_id\":\"T123\",\"workspace_path\":\"{}\"}},\"error\":null,\"durationMs\":1}}'\n",
            workspace_dir.to_string_lossy()
        ),
    );

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-agent-workspace-output".to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "agent step workspace propagation".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "workspace_path": { "type": "string" }
                },
                "required": ["task_id", "workspace_path"]
            }),
            spec_config: json!({
                "instruction": "Return a task_id and workspace_path."
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add agent activity");

    let cli_script_path = dir.path().join("capture-workdir.sh");
    std::fs::write(
        &cli_script_path,
        "#!/bin/sh\nprintf '{\"cwd\":\"%s\"}' \"$PWD\" > \"$ORBIT_OUTPUT_FILE\"\n",
    )
    .expect("write cli script");
    #[cfg(unix)]
    std::fs::set_permissions(&cli_script_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod cli script");

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-cli-workspace-consumer".to_string(),
            spec_type: "cli_command".to_string(),
            description: "consume workspace_path from prior step".to_string(),
            input_schema_json: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "workspace_path": { "type": "string" }
                },
                "required": ["task_id", "workspace_path"]
            }),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "cwd": { "type": "string" }
                },
                "required": ["cwd"]
            }),
            spec_config: json!({
                "command": cli_script_path.to_string_lossy().to_string(),
                "working_dir": "{{workspace_path}}",
                "expected_exit_codes": [0]
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add cli activity");

    let job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: None,
            steps: vec![
                JobStep {
                    target_type: JobTargetType::Activity,
                    target_id: "spec-agent-workspace-output".to_string(),
                    agent_cli,
                    timeout_seconds: 10,
                    env_extra: vec![],
                },
                JobStep {
                    target_type: JobTargetType::Activity,
                    target_id: "spec-cli-workspace-consumer".to_string(),
                    agent_cli: String::new(),
                    timeout_seconds: 10,
                    env_extra: vec![],
                },
            ],
            initial_state_override: None,
        })
        .expect("add job")
        .job_id;

    let result = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(result.state, JobRunState::Success);

    let history = runtime.job_history(&job_id).expect("job history");
    let cli_output = history[0]
        .steps
        .get(1)
        .and_then(|step| step.agent_response_json.as_ref())
        .expect("cli step output");
    let expected_cwd = workspace_dir.canonicalize().expect("canonical workspace");
    assert_eq!(
        cli_output["cwd"],
        json!(expected_cwd.to_string_lossy().to_string())
    );
}

#[test]
fn agent_step_uses_workspace_path_as_process_current_dir() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let workspace_dir = dir.path().join("task-worktree");
    std::fs::create_dir_all(&workspace_dir).expect("create workspace");

    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\ncat >/dev/null\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{\"cwd\":\"%s\"},\"error\":null,\"durationMs\":1}' \"$PWD\"\n",
    );

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-agent-current-dir".to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "agent step runs inside workspace_path".to_string(),
            input_schema_json: json!({
                "type": "object",
                "properties": {
                    "workspace_path": { "type": "string" }
                }
            }),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "cwd": { "type": "string" }
                },
                "required": ["cwd"]
            }),
            spec_config: json!({
                "instruction": "Return the current working directory."
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add activity");

    let job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({
                "workspace_path": workspace_dir.to_string_lossy().to_string()
            })),
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "spec-agent-current-dir".to_string(),
                agent_cli,
                timeout_seconds: 10,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add job")
        .job_id;

    let result = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(result.state, JobRunState::Success);

    let history = runtime.job_history(&job_id).expect("job history");
    let response = history[0]
        .steps
        .first()
        .and_then(|step| step.agent_response_json.as_ref())
        .expect("agent response");
    let expected_cwd = workspace_dir.canonicalize().expect("canonical workspace");
    assert_eq!(
        response["result"]["cwd"],
        json!(expected_cwd.to_string_lossy().to_string())
    );
}

#[test]
fn create_branch_creates_isolated_worktree_without_mutating_main_checkout() {
    let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
    let dir = tempdir().expect("tempdir");
    let data_root = dir.path().join("orbit");
    std::fs::create_dir_all(&data_root).expect("create data root");
    let repo_root = dir.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    init_git_repo(&repo_root);
    std::fs::write(repo_root.join("README.md"), "seed\n").expect("write seed file");
    git_commit_all(&repo_root, "chore: seed repo");

    let rename_status = Command::new("git")
        .args(["branch", "-M", "agent-main"])
        .current_dir(&repo_root)
        .status()
        .expect("rename branch");
    assert!(rename_status.success(), "git branch -M must succeed");
    assert_eq!(git_current_branch(&repo_root), "agent-main");

    let worktree_root = dir.path().join("worktrees");
    let previous_worktree_root = std::env::var("ORBIT_WORKTREE_ROOT").ok();
    unsafe {
        std::env::set_var("ORBIT_WORKTREE_ROOT", &worktree_root);
    }

    let runtime = OrbitRuntime::from_data_root(&data_root).expect("runtime");
    let task_id = runtime
        .add_task(TaskAddParams {
            title: "Create worktree".to_string(),
            description: "desc".to_string(),
            plan: "plan".to_string(),
            comment: None,
            context_files: vec![],
            workspace_path: Some(repo_root.to_string_lossy().to_string()),
            priority: TaskPriority::High,
            task_type: TaskType::Task,
        })
        .expect("add task")
        .id;
    let capture_script = dir.path().join("capture-worktree-context.sh");
    std::fs::write(
        &capture_script,
        "#!/bin/sh\nbranch=$(git rev-parse --abbrev-ref HEAD)\nprintf '{\"cwd\":\"%s\",\"branch\":\"%s\"}' \"$PWD\" \"$branch\" > \"$ORBIT_OUTPUT_FILE\"\n",
    )
    .expect("write capture script");
    #[cfg(unix)]
    std::fs::set_permissions(&capture_script, std::fs::Permissions::from_mode(0o755))
        .expect("chmod capture script");

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-create-task-worktree".to_string(),
            spec_type: "automation".to_string(),
            description: "create isolated task worktree".to_string(),
            input_schema_json: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "base": { "type": "string" }
                },
                "required": ["task_id", "base"]
            }),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "workspace_path": { "type": "string" },
                    "repo_root": { "type": "string" },
                    "branch": { "type": "string" }
                },
                "required": ["workspace_path", "repo_root", "branch"]
            }),
            spec_config: json!({
                "action": "create_task_worktree"
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add create activity");

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-capture-worktree-context".to_string(),
            spec_type: "cli_command".to_string(),
            description: "capture task worktree context".to_string(),
            input_schema_json: json!({
                "type": "object",
                "properties": {
                    "workspace_path": { "type": "string" },
                    "repo_root": { "type": "string" },
                    "branch": { "type": "string" }
                },
                "required": ["workspace_path", "repo_root", "branch"]
            }),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "cwd": { "type": "string" },
                    "branch": { "type": "string" }
                },
                "required": ["cwd", "branch"]
            }),
            spec_config: json!({
                "command": capture_script.to_string_lossy().to_string(),
                "working_dir": "{{workspace_path}}",
                "expected_exit_codes": [0]
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add capture activity");

    let job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({
                "task_id": task_id.clone(),
                "base": "agent-main",
                "workspace_path": worktree_root
                    .join(task_id.as_str())
                    .to_string_lossy()
                    .to_string()
            })),
            steps: vec![
                JobStep {
                    target_type: JobTargetType::Activity,
                    target_id: "spec-create-task-worktree".to_string(),
                    agent_cli: String::new(),
                    timeout_seconds: 30,
                    env_extra: vec![],
                },
                JobStep {
                    target_type: JobTargetType::Activity,
                    target_id: "spec-capture-worktree-context".to_string(),
                    agent_cli: String::new(),
                    timeout_seconds: 30,
                    env_extra: vec![],
                },
            ],
            initial_state_override: None,
        })
        .expect("add job")
        .job_id;

    let run_result = runtime.run_job_now(&job_id);
    match previous_worktree_root {
        Some(value) => unsafe { std::env::set_var("ORBIT_WORKTREE_ROOT", value) },
        None => unsafe { std::env::remove_var("ORBIT_WORKTREE_ROOT") },
    }

    let result = run_result.expect("run job");
    assert_eq!(result.state, JobRunState::Success);

    let history = runtime.job_history(&job_id).expect("job history");
    let create_output = history[0]
        .steps
        .first()
        .and_then(|step| step.agent_response_json.as_ref())
        .expect("create output");
    let capture_output = history[0]
        .steps
        .get(1)
        .and_then(|step| step.agent_response_json.as_ref())
        .expect("capture output");

    let task_worktree = create_output["workspace_path"]
        .as_str()
        .expect("workspace_path");
    let canonical_task_worktree = std::path::PathBuf::from(task_worktree)
        .canonicalize()
        .expect("canonical task worktree");
    let canonical_worktree_root = worktree_root
        .canonicalize()
        .expect("canonical worktree root");
    let canonical_repo_root = repo_root.canonicalize().expect("canonical repo root");
    assert_ne!(task_worktree, repo_root.to_string_lossy().as_ref());
    assert!(canonical_task_worktree.starts_with(&canonical_worktree_root));
    assert_eq!(
        create_output["repo_root"],
        json!(canonical_repo_root.to_string_lossy().to_string())
    );
    assert_eq!(create_output["branch"], json!(format!("orbit/{task_id}")));
    assert_eq!(capture_output["branch"], json!(format!("orbit/{task_id}")));
    assert_eq!(
        capture_output["cwd"],
        json!(canonical_task_worktree.to_string_lossy().to_string())
    );
    assert_eq!(git_current_branch(&repo_root), "agent-main");
}

#[test]
fn start_task_automation_moves_task_into_progress() {
    let dir = tempdir().expect("tempdir");
    let data_root = dir.path().join("orbit");
    std::fs::create_dir_all(&data_root).expect("create data root");
    let runtime = OrbitRuntime::from_data_root(&data_root).expect("runtime");

    let task = runtime
        .add_task(TaskAddParams {
            title: "Start me".to_string(),
            description: "desc".to_string(),
            plan: "plan".to_string(),
            comment: None,
            context_files: vec![],
            workspace_path: None,
            priority: TaskPriority::Medium,
            task_type: TaskType::Task,
        })
        .expect("add task");
    assert_eq!(task.status, TaskStatus::Backlog);

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-start-task".to_string(),
            spec_type: "automation".to_string(),
            description: "start task".to_string(),
            input_schema_json: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "note": { "type": "string" }
                },
                "required": ["task_id"]
            }),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "status": { "type": "string" }
                },
                "required": ["task_id", "status"]
            }),
            spec_config: json!({"action":"start_task"}),
            workspace_path: None,
            created_by: None,
        })
        .expect("add start activity");

    let job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({
                "task_id": task.id,
                "note": "pipeline ready to implement"
            })),
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "spec-start-task".to_string(),
                agent_cli: String::new(),
                timeout_seconds: 30,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add job")
        .job_id;

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let updated = runtime.get_task(task.id.as_ref()).expect("updated task");
    assert_eq!(updated.status, TaskStatus::InProgress);
    assert_eq!(updated.history.last().expect("history").event, "started");

    let history = runtime.job_history(&job_id).expect("job history");
    let output = history[0].steps[0]
        .agent_response_json
        .as_ref()
        .expect("start output");
    assert_eq!(output["task_id"], json!(task.id));
    assert_eq!(output["status"], json!("in_progress"));
    assert!(output.get("workspace_path").is_none());
}

#[test]
fn update_task_automation_moves_task_to_review_with_summary_comment_and_note() {
    let dir = tempdir().expect("tempdir");
    let data_root = dir.path().join("orbit");
    std::fs::create_dir_all(&data_root).expect("create data root");
    let runtime = OrbitRuntime::from_data_root(&data_root).expect("runtime");

    let task = runtime
        .add_task(TaskAddParams {
            title: "Review me".to_string(),
            description: "desc".to_string(),
            plan: "plan".to_string(),
            comment: None,
            context_files: vec![],
            workspace_path: None,
            priority: TaskPriority::Medium,
            task_type: TaskType::Task,
        })
        .expect("add task");
    let started = runtime
        .start_task(&task.id, Some("picked up".to_string()), None)
        .expect("start task");
    assert_eq!(started.status, TaskStatus::InProgress);

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-update-task".to_string(),
            spec_type: "automation".to_string(),
            description: "update task".to_string(),
            input_schema_json: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "status": { "type": "string" },
                    "execution_summary": { "type": "string" },
                    "files_changed": { "type": "array", "items": { "type": "string" } },
                    "comment": { "type": "string" },
                    "note": { "type": "string" }
                },
                "required": ["task_id", "status"]
            }),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "status": { "type": "string" },
                    "execution_summary": { "type": "string" }
                },
                "required": ["id", "status", "execution_summary"]
            }),
            spec_config: json!({"action":"update_task"}),
            workspace_path: None,
            created_by: None,
        })
        .expect("add update activity");

    let job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({
                "task_id": task.id,
                "status": "review",
                "execution_summary": "Implemented the task and validated targeted tests.",
                "files_changed": ["orbit-core/src/command/task.rs"],
                "comment": "Ready for review",
                "note": "handing off for review"
            })),
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "spec-update-task".to_string(),
                agent_cli: String::new(),
                timeout_seconds: 30,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add job")
        .job_id;

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let updated = runtime.get_task(task.id.as_ref()).expect("updated task");
    assert_eq!(updated.status, TaskStatus::Review);
    assert_eq!(
        updated.execution_summary,
        "Implemented the task and validated targeted tests."
    );
    assert_eq!(updated.comments.last().expect("comment").message, "Ready for review");
    let history = updated.history.last().expect("history");
    assert_eq!(history.event, "moved");
    assert_eq!(history.note.as_deref(), Some("handing off for review"));

    let history = runtime.job_history(&job_id).expect("job history");
    let output = history[0].steps[0]
        .agent_response_json
        .as_ref()
        .expect("update output");
    assert_eq!(output["id"], json!(task.id));
    assert_eq!(output["status"], json!("review"));
    assert_eq!(
        output["execution_summary"],
        json!("Implemented the task and validated targeted tests.")
    );
}

#[test]
fn implement_change_result_status_flows_into_update_task_as_task_status() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let task = runtime
        .add_task(TaskAddParams {
            title: "Implement then persist".to_string(),
            description: "desc".to_string(),
            plan: "plan".to_string(),
            comment: None,
            context_files: vec![],
            workspace_path: None,
            priority: TaskPriority::Medium,
            task_type: TaskType::Task,
        })
        .expect("add task");
    runtime
        .start_task(&task.id, None, None)
        .expect("start task");

    let script_path = dir.path().join("mock-agent");
    let script = concat!(
        "#!/bin/sh\n",
        "cat >/dev/null\n",
        "printf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{\"status\":\"review\",\"summary\":\"Implemented the change.\",\"execution_summary\":\"Implemented the change and validated tests.\",\"files_changed\":[\"orbit-core/src/lib.rs\"],\"note\":\"ready for review\"},\"error\":null,\"durationMs\":1}'\n",
    );
    let agent_cli = write_agent_script(&script_path, script);

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-implement-like".to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "implement step".to_string(),
            input_schema_json: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" }
                },
                "required": ["task_id"]
            }),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "status": { "type": "string" },
                    "summary": { "type": "string" },
                    "execution_summary": { "type": "string" },
                    "files_changed": { "type": "array", "items": { "type": "string" } },
                    "note": { "type": "string" }
                },
                "required": ["status", "summary", "execution_summary"]
            }),
            spec_config: json!({
                "instruction": "Return a synthetic implement_change result."
            }),
            workspace_path: None,
            identity_id: None,
            created_by: None,
        })
        .expect("add implement-like activity");

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-update-task-from-implement".to_string(),
            spec_type: "automation".to_string(),
            description: "update task".to_string(),
            input_schema_json: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "status": { "type": "string" },
                    "execution_summary": { "type": "string" },
                    "files_changed": { "type": "array", "items": { "type": "string" } },
                    "note": { "type": "string" }
                },
                "required": ["task_id", "status"]
            }),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "status": { "type": "string" },
                    "execution_summary": { "type": "string" }
                },
                "required": ["id", "status", "execution_summary"]
            }),
            spec_config: json!({"action":"update_task"}),
            workspace_path: None,
            identity_id: None,
            created_by: None,
        })
        .expect("add update activity");

    let job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({ "task_id": task.id })),
            steps: vec![
                JobStep {
                    target_type: JobTargetType::Activity,
                    target_id: "spec-implement-like".to_string(),
                    agent_cli,
                    timeout_seconds: 30,
                    env_extra: vec![],
                },
                JobStep {
                    target_type: JobTargetType::Activity,
                    target_id: "spec-update-task-from-implement".to_string(),
                    agent_cli: String::new(),
                    timeout_seconds: 30,
                    env_extra: vec![],
                },
            ],
            initial_state_override: None,
        })
        .expect("add job")
        .job_id;

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let updated = runtime.get_task(task.id.as_ref()).expect("updated task");
    assert_eq!(updated.status, TaskStatus::Review);
    assert_eq!(
        updated.execution_summary,
        "Implemented the change and validated tests."
    );
    assert_eq!(
        updated.history.last().and_then(|entry| entry.note.as_deref()),
        Some("ready for review")
    );
}

#[test]
fn commit_changes_automation_commits_dirty_task_worktree() {
    let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
    let dir = tempdir().expect("tempdir");
    let data_root = dir.path().join("orbit");
    std::fs::create_dir_all(&data_root).expect("create data root");
    let repo_root = dir.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    init_git_repo(&repo_root);
    std::fs::write(repo_root.join("README.md"), "seed\n").expect("write seed file");
    git_commit_all(&repo_root, "chore: seed repo");

    let rename_status = Command::new("git")
        .args(["branch", "-M", "agent-main"])
        .current_dir(&repo_root)
        .status()
        .expect("rename branch");
    assert!(rename_status.success(), "git branch -M must succeed");

    let worktree_root = dir.path().join("worktrees");
    let previous_worktree_root = std::env::var("ORBIT_WORKTREE_ROOT").ok();
    unsafe {
        std::env::set_var("ORBIT_WORKTREE_ROOT", &worktree_root);
    }

    let runtime = OrbitRuntime::from_data_root(&data_root).expect("runtime");
    let task_id = runtime
        .add_task(TaskAddParams {
            title: "Refactor automation flow".to_string(),
            description: "desc".to_string(),
            plan: "plan".to_string(),
            comment: None,
            context_files: vec![],
            workspace_path: Some(repo_root.to_string_lossy().to_string()),
            priority: TaskPriority::High,
            task_type: TaskType::Refactor,
        })
        .expect("add task")
        .id;

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-create-task-worktree-for-commit".to_string(),
            spec_type: "automation".to_string(),
            description: "create isolated task worktree".to_string(),
            input_schema_json: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "base": { "type": "string" },
                    "workspace_path": { "type": "string" }
                },
                "required": ["task_id", "base", "workspace_path"]
            }),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "workspace_path": { "type": "string" },
                    "repo_root": { "type": "string" },
                    "branch": { "type": "string" }
                },
                "required": ["workspace_path", "repo_root", "branch"]
            }),
            spec_config: json!({
                "action": "create_task_worktree"
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add create activity");

    let create_job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({
                "task_id": task_id,
                "base": "agent-main",
                "workspace_path": repo_root.to_string_lossy().to_string()
            })),
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "spec-create-task-worktree-for-commit".to_string(),
                agent_cli: String::new(),
                timeout_seconds: 30,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add create job")
        .job_id;

    let create_run = runtime.run_job_now(&create_job_id).expect("run create job");
    assert_eq!(create_run.state, JobRunState::Success);
    let create_history = runtime.job_history(&create_job_id).expect("create history");
    let create_output = create_history[0].steps[0]
        .agent_response_json
        .as_ref()
        .expect("create output");
    let workspace_path = create_output["workspace_path"]
        .as_str()
        .expect("workspace_path")
        .to_string();
    let branch = create_output["branch"]
        .as_str()
        .expect("branch")
        .to_string();

    let worktree_path = std::path::PathBuf::from(&workspace_path);
    std::fs::write(worktree_path.join("README.md"), "seed\nupdated\n").expect("update tracked");
    std::fs::write(worktree_path.join("new-file.txt"), "new\n").expect("write new file");

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-commit-task-worktree".to_string(),
            spec_type: "automation".to_string(),
            description: "commit task changes".to_string(),
            input_schema_json: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "workspace_path": { "type": "string" },
                    "repo_root": { "type": "string" },
                    "branch": { "type": "string" }
                },
                "required": ["task_id", "workspace_path", "repo_root", "branch"]
            }),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "repo_root": { "type": "string" },
                    "workspace_path": { "type": "string" },
                    "branch": { "type": "string" },
                    "commit_message": { "type": "string" },
                    "commit_sha": { "type": "string" },
                    "changed_files": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["repo_root", "workspace_path", "branch", "commit_message", "commit_sha", "changed_files"]
            }),
            spec_config: json!({
                "action": "commit_task_changes"
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add commit activity");

    let commit_job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({
                "task_id": task_id,
                "workspace_path": workspace_path,
                "repo_root": repo_root.to_string_lossy().to_string(),
                "branch": branch,
                "summary": "Implemented the automation refactor."
            })),
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "spec-commit-task-worktree".to_string(),
                agent_cli: String::new(),
                timeout_seconds: 30,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add commit job")
        .job_id;

    let commit_run = runtime.run_job_now(&commit_job_id).expect("run commit job");
    match previous_worktree_root {
        Some(value) => unsafe { std::env::set_var("ORBIT_WORKTREE_ROOT", value) },
        None => unsafe { std::env::remove_var("ORBIT_WORKTREE_ROOT") },
    }
    assert_eq!(commit_run.state, JobRunState::Success);

    let commit_history = runtime.job_history(&commit_job_id).expect("commit history");
    let commit_output = commit_history[0].steps[0]
        .agent_response_json
        .as_ref()
        .expect("commit output");
    assert_eq!(commit_output["branch"], json!(format!("orbit/{task_id}")));
    assert_eq!(
        commit_output["commit_message"],
        json!(format!(
            "refactor: Refactor automation flow [{task_id}]\n\nImplemented the automation refactor."
        ))
    );
    assert_eq!(
        commit_output["changed_files"],
        json!(vec!["README.md", "new-file.txt"])
    );

    let log = Command::new("git")
        .args(["log", "-1", "--pretty=%B"])
        .current_dir(&worktree_path)
        .output()
        .expect("git log");
    assert_eq!(
        String::from_utf8_lossy(&log.stdout).trim(),
        format!(
            "refactor: Refactor automation flow [{task_id}]\n\nImplemented the automation refactor."
        )
    );
    assert_eq!(git_current_branch(&repo_root), "agent-main");
}

#[test]
fn commit_task_changes_uses_summary_from_input() {
    // Regression: commit_task_changes must accept summary from pipeline input, not from task store.
    // The task store has no execution_summary; the automation should succeed because summary is in input.
    // When summary is absent from input the automation must fail with a clear error.
    let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
    let dir = tempdir().expect("tempdir");
    let data_root = dir.path().join("orbit");
    std::fs::create_dir_all(&data_root).expect("create data root");
    let repo_root = dir.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    init_git_repo(&repo_root);
    std::fs::write(repo_root.join("README.md"), "seed\n").expect("write seed file");
    git_commit_all(&repo_root, "chore: seed repo");
    Command::new("git")
        .args(["branch", "-M", "agent-main"])
        .current_dir(&repo_root)
        .status()
        .expect("rename branch");

    let worktree_root = dir.path().join("worktrees");
    let previous_worktree_root = std::env::var("ORBIT_WORKTREE_ROOT").ok();
    unsafe {
        std::env::set_var("ORBIT_WORKTREE_ROOT", &worktree_root);
    }

    let runtime = OrbitRuntime::from_data_root(&data_root).expect("runtime");
    let task_id = runtime
        .add_task(TaskAddParams {
            title: "Fix bundle atomicity".to_string(),
            description: "desc".to_string(),
            plan: "plan".to_string(),
            comment: None,
            context_files: vec![],
            workspace_path: Some(repo_root.to_string_lossy().to_string()),
            priority: TaskPriority::High,
            task_type: TaskType::Issue,
        })
        .expect("add task")
        .id;
    // Intentionally do NOT set execution_summary on the task — the automation must not need it.

    // Create worktree via automation so the branch exists.
    runtime
        .add_activity(ActivityAddParams {
            id: "spec-create-wt-regression".to_string(),
            spec_type: "automation".to_string(),
            description: "create worktree".to_string(),
            input_schema_json: json!({"type":"object","properties":{"task_id":{"type":"string"},"base":{"type":"string"},"workspace_path":{"type":"string"}},"required":["task_id","base","workspace_path"]}),
            output_schema_json: json!({"type":"object","properties":{"workspace_path":{"type":"string"},"branch":{"type":"string"}},"required":["workspace_path","branch"]}),
            spec_config: json!({"action":"create_task_worktree"}),
            workspace_path: None,
            created_by: None,
        })
        .expect("add create activity");
    let create_job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({"task_id": task_id, "base": "agent-main", "workspace_path": repo_root.to_string_lossy().to_string()})),
            steps: vec![JobStep { target_type: JobTargetType::Activity, target_id: "spec-create-wt-regression".to_string(), agent_cli: String::new(), timeout_seconds: 30, env_extra: vec![] }],
            initial_state_override: None,
        })
        .expect("add create job")
        .job_id;
    let create_run = runtime.run_job_now(&create_job_id).expect("run create");
    assert_eq!(create_run.state, JobRunState::Success);
    let create_history = runtime.job_history(&create_job_id).expect("create history");
    let create_output = create_history[0].steps[0]
        .agent_response_json
        .as_ref()
        .expect("output");
    let workspace_path = create_output["workspace_path"]
        .as_str()
        .expect("workspace_path")
        .to_string();
    let branch = create_output["branch"]
        .as_str()
        .expect("branch")
        .to_string();

    std::fs::write(
        std::path::Path::new(&workspace_path).join("fix.rs"),
        "fixed\n",
    )
    .expect("write");

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-commit-regression".to_string(),
            spec_type: "automation".to_string(),
            description: "commit changes".to_string(),
            input_schema_json: json!({"type":"object","properties":{"task_id":{"type":"string"},"workspace_path":{"type":"string"},"repo_root":{"type":"string"},"branch":{"type":"string"},"summary":{"type":"string"}},"required":["task_id","workspace_path","repo_root","branch","summary"]}),
            output_schema_json: json!({"type":"object","properties":{"commit_message":{"type":"string"}},"required":["commit_message"]}),
            spec_config: json!({"action":"commit_task_changes"}),
            workspace_path: None,
            created_by: None,
        })
        .expect("add commit activity");

    // Case 1: summary present in input, no execution_summary in store → must succeed.
    let success_job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({"task_id": task_id, "workspace_path": workspace_path, "repo_root": repo_root.to_string_lossy().to_string(), "branch": branch, "summary": "Hardened bundle writes using staged directory rename."})),
            steps: vec![JobStep { target_type: JobTargetType::Activity, target_id: "spec-commit-regression".to_string(), agent_cli: String::new(), timeout_seconds: 30, env_extra: vec![] }],
            initial_state_override: None,
        })
        .expect("add success job")
        .job_id;
    let success_run = runtime
        .run_job_now(&success_job_id)
        .expect("run success job");
    match previous_worktree_root.as_ref() {
        Some(value) => unsafe { std::env::set_var("ORBIT_WORKTREE_ROOT", value) },
        None => unsafe { std::env::remove_var("ORBIT_WORKTREE_ROOT") },
    }
    assert_eq!(
        success_run.state,
        JobRunState::Success,
        "must succeed when summary is in input"
    );

    // Case 2: summary absent from input → must fail with clear error.
    let task2_id = runtime
        .add_task(TaskAddParams {
            title: "Fix bundle atomicity 2".to_string(),
            description: "desc".to_string(),
            plan: "plan".to_string(),
            comment: None,
            context_files: vec![],
            workspace_path: Some(repo_root.to_string_lossy().to_string()),
            priority: TaskPriority::High,
            task_type: TaskType::Issue,
        })
        .expect("add task2")
        .id;
    let fail_job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({"task_id": task2_id, "workspace_path": workspace_path, "repo_root": repo_root.to_string_lossy().to_string(), "branch": branch, "summary": ""})),
            steps: vec![JobStep { target_type: JobTargetType::Activity, target_id: "spec-commit-regression".to_string(), agent_cli: String::new(), timeout_seconds: 30, env_extra: vec![] }],
            initial_state_override: None,
        })
        .expect("add fail job")
        .job_id;
    let fail_run = runtime.run_job_now(&fail_job_id).expect("run fail job");
    assert_eq!(
        fail_run.state,
        JobRunState::Failed,
        "must fail when summary is absent"
    );
    let fail_history = runtime.job_history(&fail_job_id).expect("fail history");
    let error_msg = fail_history[0].steps[0]
        .error_message
        .as_deref()
        .unwrap_or("");
    assert!(
        error_msg.contains("requires a non-empty summary"),
        "error should mention missing summary, got: {error_msg}"
    );
}

#[test]
fn open_pr_automation_uses_task_title_and_commit_output() {
    let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
    let dir = tempdir().expect("tempdir");
    let data_root = dir.path().join("orbit");
    std::fs::create_dir_all(&data_root).expect("create data root");
    let repo_root = dir.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    init_git_repo(&repo_root);
    std::fs::write(repo_root.join("README.md"), "seed\n").expect("write seed file");
    git_commit_all(&repo_root, "chore: seed repo");

    // Set up a local bare repo as "origin" so git.push succeeds without a real remote.
    let bare_dir = dir.path().join("origin.git");
    std::fs::create_dir_all(&bare_dir).expect("create bare dir");
    let status = Command::new("git")
        .args(["init", "--bare", "-q"])
        .current_dir(&bare_dir)
        .status()
        .expect("git init --bare");
    assert!(status.success(), "git init --bare must succeed");
    let status = Command::new("git")
        .args(["remote", "add", "origin", &bare_dir.to_string_lossy()])
        .current_dir(&repo_root)
        .status()
        .expect("git remote add origin");
    assert!(status.success(), "git remote add must succeed");

    let gh_dir = dir.path().join("bin");
    std::fs::create_dir_all(&gh_dir).expect("create gh dir");
    let title_capture = dir.path().join("gh-title.txt");
    let body_capture = dir.path().join("gh-body.txt");
    let base_capture = dir.path().join("gh-base.txt");
    let head_capture = dir.path().join("gh-head.txt");
    let gh_script = gh_dir.join("gh");
    let gh_body = format!(
        concat!(
            "#!/bin/sh\n",
            "if [ \"$1\" = \"pr\" ] && [ \"$2\" = \"create\" ]; then\n",
            "  shift 2\n",
            "  while [ $# -gt 0 ]; do\n",
            "    case \"$1\" in\n",
            "      --title) printf '%s' \"$2\" > \"{title}\"; shift 2 ;;\n",
            "      --body) printf '%s' \"$2\" > \"{body}\"; shift 2 ;;\n",
            "      --base) printf '%s' \"$2\" > \"{base}\"; shift 2 ;;\n",
            "      --head) printf '%s' \"$2\" > \"{head}\"; shift 2 ;;\n",
            "      *) shift ;;\n",
            "    esac\n",
            "  done\n",
            "  printf 'https://github.com/example/orbit/pull/42\\n'\n",
            "  exit 0\n",
            "fi\n",
            "if [ \"$1\" = \"pr\" ] && [ \"$2\" = \"view\" ]; then\n",
            "  printf '{{\"number\":42,\"title\":\"captured\",\"body\":\"captured\",\"headRefName\":\"orbit/test\",\"files\":[],\"commits\":[]}}'\n",
            "  exit 0\n",
            "fi\n",
            "exit 1\n",
        ),
        title = title_capture.display(),
        body = body_capture.display(),
        base = base_capture.display(),
        head = head_capture.display(),
    );
    std::fs::write(&gh_script, gh_body).expect("write gh script");
    #[cfg(unix)]
    std::fs::set_permissions(&gh_script, std::fs::Permissions::from_mode(0o755)).expect("chmod gh");

    let previous_path = std::env::var("PATH").ok();
    unsafe {
        std::env::set_var("PATH", prepend_path(&gh_dir));
    }

    let runtime = OrbitRuntime::from_data_root(&data_root).expect("runtime");
    let task_id = runtime
        .add_task(TaskAddParams {
            title: "Wire Orbit PR automation".to_string(),
            description: "desc".to_string(),
            plan: "plan".to_string(),
            comment: None,
            context_files: vec![],
            workspace_path: Some(repo_root.to_string_lossy().to_string()),
            priority: TaskPriority::High,
            task_type: TaskType::Task,
        })
        .expect("add task")
        .id;

    // Create the task branch locally so git.push has something to push.
    let status = Command::new("git")
        .args(["checkout", "-b", &format!("orbit/{task_id}")])
        .current_dir(&repo_root)
        .status()
        .expect("git checkout -b task branch");
    assert!(status.success(), "checkout task branch must succeed");
    let status = Command::new("git")
        .args(["checkout", "-"])
        .current_dir(&repo_root)
        .status()
        .expect("git checkout - back");
    assert!(status.success(), "checkout back must succeed");

    runtime
        .update_task(
            &task_id,
            TaskUpdateParams {
                title: None,
                description: None,
                plan: None,
                execution_summary: None,
                comment: None,
                status: Some(TaskStatus::InProgress),
                branch: Some(Some(format!("orbit/{task_id}"))),
                pr_number: None,
            },
        )
        .expect("prepare task");

    runtime
        .add_activity(ActivityAddParams {
            id: "spec-open-pr-from-task".to_string(),
            spec_type: "automation".to_string(),
            description: "open pr from task".to_string(),
            input_schema_json: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string" },
                    "repo_root": { "type": "string" },
                    "branch": { "type": "string" },
                    "base": { "type": "string" }
                },
                "required": ["task_id", "repo_root"]
            }),
            output_schema_json: json!({
                "type": "object",
                "properties": {
                    "pr_url": { "type": "string" },
                    "pr_number": { "type": "string" },
                    "title": { "type": "string" },
                    "body": { "type": "string" }
                },
                "required": ["pr_url", "pr_number", "title", "body"]
            }),
            spec_config: json!({
                "action": "open_pr_from_task"
            }),
            workspace_path: None,
            created_by: None,
        })
        .expect("add pr activity");

    let job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: Some(json!({
                "task_id": task_id,
                "repo_root": repo_root.to_string_lossy().to_string(),
                "branch": format!("orbit/{task_id}"),
                "base": "agent-main",
                "commit_message": format!("feat: Wire Orbit PR automation [{task_id}]\n\nOrbit owns PR creation now."),
                "changed_files": ["orbit-core/src/lib.rs", "orbit-engine/src/executor/automation.rs"]
            })),
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "spec-open-pr-from-task".to_string(),
                agent_cli: String::new(),
                timeout_seconds: 30,
                env_extra: vec![],
            }],
            initial_state_override: None,
        })
        .expect("add job")
        .job_id;

    let run_result = runtime.run_job_now(&job_id);
    match previous_path {
        Some(value) => unsafe { std::env::set_var("PATH", value) },
        None => unsafe { std::env::remove_var("PATH") },
    }
    let run = run_result.expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let history = runtime.job_history(&job_id).expect("history");
    let output = history[0].steps[0]
        .agent_response_json
        .as_ref()
        .expect("pr output");
    assert_eq!(
        output["pr_url"],
        json!("https://github.com/example/orbit/pull/42")
    );
    assert_eq!(output["pr_number"], json!("42"));
    assert_eq!(
        std::fs::read_to_string(title_capture).expect("title"),
        "Wire Orbit PR automation"
    );
    let body_content = std::fs::read_to_string(body_capture).expect("body");
    assert!(
        body_content.contains("Orbit owns PR creation now."),
        "body: {body_content}"
    );
    assert!(
        body_content.contains("## Files Changed"),
        "body: {body_content}"
    );
    assert!(
        body_content.contains("automation.rs"),
        "body: {body_content}"
    );
    assert_eq!(
        std::fs::read_to_string(base_capture).expect("base"),
        "agent-main"
    );
    assert_eq!(
        std::fs::read_to_string(head_capture).expect("head"),
        format!("orbit/{task_id}")
    );

    let updated_task = runtime.get_task(&task_id).expect("updated task");
    assert_eq!(updated_task.pr_number.as_deref(), Some("42"));
    assert_eq!(updated_task.status, TaskStatus::Review);
}

#[test]
fn multi_step_same_agent_cli_each_step_gets_its_own_env_extra() {
    // Regression: env_extra was looked up by matching agent_cli (first match), so step 2
    // sharing the same agent_cli as step 1 would receive step 1's allowlist.
    let dir = tempdir().expect("tempdir");

    // Hermetic env: only the codex-required vars pass by default; each step adds its own.
    write_runtime_config(
        dir.path(),
        r#"[execution.env]
inherit = false
pass = ["HOME", "PATH"]
"#,
    );

    // Set the variables in the test process so they can be passed through.
    // Safety: test binary is single-threaded at this point; no concurrent env reads.
    unsafe {
        std::env::set_var("STEP1_SECRET", "alpha");
        std::env::set_var("STEP2_SECRET", "beta");
    }

    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    // Agent dumps its environment to a file named by a counter.
    let counter_file = dir.path().join("call_count");
    std::fs::write(&counter_file, "0").expect("write counter");
    let env_dir = dir.path().join("env_captures");
    std::fs::create_dir_all(&env_dir).expect("create env dir");

    // Provider detection uses the binary filename; name it "codex" so it is recognized.
    let script_path = dir.path().join("codex");
    let script = format!(
        concat!(
            "#!/bin/sh\n",
            "n=$(cat \"{counter}\")\n",
            "n=$((n + 1))\n",
            "printf '%s' \"$n\" > \"{counter}\"\n",
            "env > \"{env_dir}/$n.env\"\n",
            "cat > /dev/null\n",
            "printf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null,\"durationMs\":1}}'\n",
        ),
        counter = counter_file.display(),
        env_dir = env_dir.display(),
    );
    let agent_cli = write_agent_script(&script_path, &script);

    add_activity(&runtime, "spec-multi-env-step1");
    add_activity(&runtime, "spec-multi-env-step2");

    let job_id = runtime
        .add_job(JobAddParams {
            job_id: None,
            default_input: None,
            steps: vec![
                JobStep {
                    target_type: JobTargetType::Activity,
                    target_id: "spec-multi-env-step1".to_string(),
                    agent_cli: agent_cli.clone(),
                    timeout_seconds: 10,
                    env_extra: vec!["STEP1_SECRET".to_string()],
                },
                JobStep {
                    target_type: JobTargetType::Activity,
                    target_id: "spec-multi-env-step2".to_string(),
                    agent_cli: agent_cli.clone(),
                    timeout_seconds: 10,
                    env_extra: vec!["STEP2_SECRET".to_string()],
                },
            ],
            initial_state_override: None,
        })
        .expect("add job")
        .job_id;

    let result = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(result.state, JobRunState::Success);

    let step1_env = std::fs::read_to_string(env_dir.join("1.env")).expect("step1 env");
    let step2_env = std::fs::read_to_string(env_dir.join("2.env")).expect("step2 env");

    // Step 1 must see STEP1_SECRET but not STEP2_SECRET.
    assert!(
        step1_env.contains("STEP1_SECRET=alpha"),
        "step1 should have STEP1_SECRET"
    );
    assert!(
        !step1_env.contains("STEP2_SECRET"),
        "step1 must not have STEP2_SECRET"
    );

    // Step 2 must see STEP2_SECRET but not STEP1_SECRET.
    assert!(
        step2_env.contains("STEP2_SECRET=beta"),
        "step2 should have STEP2_SECRET"
    );
    assert!(
        !step2_env.contains("STEP1_SECRET"),
        "step2 must not have STEP1_SECRET"
    );
}
