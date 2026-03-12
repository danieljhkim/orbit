use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, OnceLock};
use std::thread;

use chrono::{Duration as ChronoDuration, Utc};
use orbit_core::OrbitRuntime;
use orbit_core::command::activity::{ActivityAddParams, ActivityRunParams};
use orbit_core::command::job::JobAddParams;
use orbit_core::job::runtime::{JobRuntime, JobRuntimeConfig, ShutdownSignal};
use orbit_store::Store;
use orbit_types::{JobRetryBackoffStrategy, JobRunState, JobTargetType, OrbitError};
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
            spec_type: "analysis".to_string(),
            description: "runtime test spec".to_string(),
            instruction: "Run the scheduled runtime behavior test.".to_string(),
            input_schema_json,
            output_schema_json: json!({}),
            artifact_path_template: None,
            skill_refs: Vec::new(),
            identity_id: None,
            assigned_to: None,
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
        spec_type: "analysis".to_string(),
        description: "missing skill".to_string(),
        instruction: String::new(),
        input_schema_json: json!({}),
        output_schema_json: json!({}),
        artifact_path_template: None,
        skill_refs: vec!["does-not-exist".to_string()],
        identity_id: None,
        assigned_to: None,
        created_by: None,
    });
    assert!(result.is_err());
}

fn add_scheduled_activity(
    runtime: &OrbitRuntime,
    target_id: &str,
    agent_cli: &str,
    retry_max_attempts: u32,
    retry_backoff_strategy: JobRetryBackoffStrategy,
    retry_initial_delay_seconds: u64,
) -> String {
    add_scheduled_activity_with_timeout(
        runtime,
        target_id,
        agent_cli,
        10,
        retry_max_attempts,
        retry_backoff_strategy,
        retry_initial_delay_seconds,
    )
}

fn add_scheduled_activity_with_timeout(
    runtime: &OrbitRuntime,
    target_id: &str,
    agent_cli: &str,
    timeout_seconds: u64,
    retry_max_attempts: u32,
    retry_backoff_strategy: JobRetryBackoffStrategy,
    retry_initial_delay_seconds: u64,
) -> String {
    runtime
        .add_job(JobAddParams {
            job_id: None,
            target_type: JobTargetType::Activity,
            target_id: target_id.to_string(),
            schedule: "every 1s".to_string(),
            agent_cli: agent_cli.to_string(),
            timeout_seconds,
            retry_max_attempts,
            retry_backoff_strategy,
            retry_initial_delay_seconds,
            initial_state_override: None,
            env_extra: vec![],
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
        0,
        JobRetryBackoffStrategy::None,
        0,
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

fn write_identity_file(data_root: &std::path::Path, id: &str, display_name: &str, role: &str) {
    let identity_root = data_root.join("identities");
    std::fs::create_dir_all(&identity_root).expect("create identity root");
    let content = format!(
        "identity:\n  name: {id}\n  display_name: {display_name}\n  role: {role}\n"
    );
    std::fs::write(identity_root.join(format!("{id}.yaml")), content).expect("write identity");
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

fn write_sqlite_job_config(data_root: &std::path::Path) {
    let db_path = data_root.join("orbit.db").to_string_lossy().to_string();
    write_runtime_config(
        data_root,
        &format!("[job]\npersistence = {{ type = \"sqlite\", path = \"{db_path}\" }}\n"),
    );
}

fn insert_stale_running_run(data_root: &std::path::Path, job_id: &str) -> String {
    let store = Store::open(&data_root.join("orbit.db")).expect("open store");
    store
        .with_transaction(|tx| {
            let old_time = Utc::now() - ChronoDuration::hours(2);
            let run = tx.insert_job_run(job_id, 1, old_time)?;
            let changed = tx.mark_job_run_running(&run.run_id, old_time)?;
            assert!(changed, "run must be marked running");
            Ok(run.run_id)
        })
        .expect("insert stale running run")
}

#[test]
fn scheduled_run_executes_agent_and_records_success_run() {
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
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-success",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

    let due_at = runtime.show_job(&job_id).expect("show job").next_run_at;
    let ran = runtime.run_due_jobs(due_at).expect("run jobs");
    assert_eq!(ran, 1);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobRunState::Success);
    assert_eq!(history[0].attempt, 1);
    assert!(history[0].agent_response_json.is_some());

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
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-success-with-input",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

    let run = runtime
        .run_job_now_with_input(&job_id, json!({ "task_id": "T123" }))
        .expect("run job");

    assert_eq!(run.state, JobRunState::Success);
    let stdin_raw = std::fs::read_to_string(stdin_capture).expect("stdin capture");
    let payload: serde_json::Value = serde_json::from_str(&stdin_raw).expect("valid stdin payload");
    assert_eq!(payload["input"]["task_id"], "T123");
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
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-invalid-input",
        "mock-agent",
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

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
fn job_run_resolves_activity_identity_from_data_root_when_home_differs() {
    let _guard = env_lock().lock().expect("env lock");
    let repo_orbit = tempdir().expect("repo orbit");
    let home = tempdir().expect("home");
    let previous_home = std::env::var("HOME").ok();
    let previous_userprofile = std::env::var("USERPROFILE").ok();
    unsafe {
        std::env::set_var("HOME", home.path());
        std::env::set_var("USERPROFILE", home.path());
    }

    write_identity_file(repo_orbit.path(), "prii", "Prii", "leader");
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
            id: "spec-identity".to_string(),
            spec_type: "analysis".to_string(),
            description: "identity runtime test".to_string(),
            instruction: "Run with an explicit identity.".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            artifact_path_template: None,
            skill_refs: Vec::new(),
            identity_id: Some("prii".to_string()),
            assigned_to: None,
            created_by: None,
        })
        .expect("add activity");
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-identity",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

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
    assert_eq!(payload["identity"]["id"], "prii");
    assert_eq!(payload["identity"]["name"], "Prii");
}

#[test]
fn invalid_agent_json_with_zero_exit_falls_back_to_success() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(&script_path, "#!/bin/sh\nprintf 'not-json'\n");

    add_activity(&runtime, "spec-protocol");
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-protocol",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

    let due_at = runtime.show_job(&job_id).expect("show job").next_run_at;
    let ran = runtime.run_due_jobs(due_at).expect("run jobs");
    assert_eq!(ran, 1);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobRunState::Success);
    assert!(history[0].error_code.is_none());

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
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-invocation",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

    let due_at = runtime.show_job(&job_id).expect("show job").next_run_at;
    let ran = runtime.run_due_jobs(due_at).expect("run jobs");
    assert_eq!(ran, 1);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobRunState::Failed);
    assert_eq!(
        history[0].error_code.as_deref(),
        Some("AGENT_INVOCATION_FAILED")
    );
    assert!(
        history[0]
            .error_message
            .as_deref()
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
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-codex-missing-env",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Failed);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(
        history[0].error_code.as_deref(),
        Some("AGENT_INVOCATION_FAILED")
    );
    let message = history[0].error_message.as_deref().unwrap_or_default();
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
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-codex-sandbox",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

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
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-codex-approval",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

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
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-commit-request",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

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
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-commit-request-isolated",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

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
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-commit-protocol",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Failed);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(
        history[0].error_code.as_deref(),
        Some("AGENT_PROTOCOL_VIOLATION")
    );
    assert!(
        history[0]
            .error_message
            .as_deref()
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
    let job_id = add_scheduled_activity_with_timeout(
        &runtime,
        "spec-timeout",
        &agent_cli,
        1,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Timeout);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history[0].state, JobRunState::Timeout);
    assert_eq!(history[0].error_code.as_deref(), Some("AGENT_TIMEOUT"));
    assert!(
        history[0]
            .error_message
            .as_deref()
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
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-claude-missing-env",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Failed);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(
        history[0].error_code.as_deref(),
        Some("AGENT_INVOCATION_FAILED")
    );
    let message = history[0].error_message.as_deref().unwrap_or_default();
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
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-provider-env-present",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

    let run = runtime.run_job_now(&job_id).expect("run job");
    assert_eq!(run.state, JobRunState::Success);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history[0].state, JobRunState::Success);
    assert!(history[0].error_code.is_none());
}

#[test]
fn run_job_now_applies_retry_policy_and_second_attempt_can_succeed() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let marker = dir.path().join("retry.marker");
    let script_path = dir.path().join("mock-agent");
    let script = format!(
        "#!/bin/sh\nif [ -f \"{marker}\" ]; then\n  printf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null,\"durationMs\":1}}'\n  exit 0\nfi\ntouch \"{marker}\"\nprintf '{{\"schemaVersion\":1,\"status\":\"failed\",\"result\":null,\"error\":{{\"code\":\"FIRST_FAIL\",\"message\":\"first attempt fails\",\"details\":{{}}}},\"durationMs\":1}}'\nexit 1\n",
        marker = marker.to_string_lossy()
    );
    let agent_cli = write_agent_script(&script_path, &script);

    add_activity(&runtime, "spec-retry");
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-retry",
        &agent_cli,
        1,
        JobRetryBackoffStrategy::Fixed,
        0,
    );

    let result = runtime.run_job_now(&job_id).expect("run now");
    assert_eq!(result.state, JobRunState::Success);
    assert_eq!(result.attempt, 2);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].attempt, 2);
    assert_eq!(history[0].state, JobRunState::Success);
    assert_eq!(history[1].attempt, 1);
    assert_eq!(history[1].state, JobRunState::Failed);
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
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-active-lock",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

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
    write_sqlite_job_config(dir.path());
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_activity(&runtime, "spec-history-stale");
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-history-stale",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );
    let stale_run_id = insert_stale_running_run(dir.path(), &job_id);

    let history = runtime.job_history(&job_id).expect("history");
    let stale = history
        .iter()
        .find(|run| run.run_id == stale_run_id)
        .expect("stale run should exist");
    assert_eq!(stale.state, JobRunState::Failed);
    assert_eq!(stale.error_code.as_deref(), Some("AGENT_INVOCATION_FAILED"));
    assert!(
        stale
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("stale active run recovered")
    );
}

#[test]
fn run_job_now_recovers_stale_running_run_and_executes_new_attempt() {
    let dir = tempdir().expect("tempdir");
    write_sqlite_job_config(dir.path());
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_activity(&runtime, "spec-run-now-stale");
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-run-now-stale",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );
    let stale_run_id = insert_stale_running_run(dir.path(), &job_id);

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
    assert_eq!(stale.error_code.as_deref(), Some("AGENT_INVOCATION_FAILED"));
    assert!(
        history.iter().any(|run| run.state == JobRunState::Success),
        "new attempt should complete successfully"
    );
}

#[test]
fn run_due_jobs_recovers_stale_running_run_and_reclaims_job() {
    let dir = tempdir().expect("tempdir");
    write_sqlite_job_config(dir.path());
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_activity(&runtime, "spec-due-stale");
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-due-stale",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );
    let stale_run_id = insert_stale_running_run(dir.path(), &job_id);

    let due_at = runtime.show_job(&job_id).expect("show job").next_run_at;
    let ran = runtime.run_due_jobs(due_at).expect("run due jobs");
    assert_eq!(ran, 1, "job should be reclaimed after stale run recovery");

    let history = runtime.job_history(&job_id).expect("history");
    let stale = history
        .iter()
        .find(|run| run.run_id == stale_run_id)
        .expect("stale run should exist");
    assert_eq!(stale.state, JobRunState::Failed);
    assert!(
        history.iter().any(|run| run.state == JobRunState::Success),
        "reclaimed due job should complete successfully"
    );
}

#[test]
fn concurrent_job_run_invocations_do_not_double_run_job() {
    let dir = tempdir().expect("tempdir");
    let runtime = Arc::new(OrbitRuntime::from_data_root(dir.path()).expect("runtime"));
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nsleep 0.2\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_activity(&runtime, "spec-concurrent");
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-concurrent",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

    let due_at = runtime.show_job(&job_id).expect("show job").next_run_at;
    let barrier = Arc::new(Barrier::new(3));

    let r1 = Arc::clone(&runtime);
    let b1 = Arc::clone(&barrier);
    let due_one = due_at;
    let t1 = thread::spawn(move || {
        b1.wait();
        r1.run_due_jobs(due_one).expect("thread 1 run")
    });

    let r2 = Arc::clone(&runtime);
    let b2 = Arc::clone(&barrier);
    let due_two = due_at;
    let t2 = thread::spawn(move || {
        b2.wait();
        r2.run_due_jobs(due_two).expect("thread 2 run")
    });

    barrier.wait();

    let c1 = t1.join().expect("join t1");
    let c2 = t2.join().expect("join t2");
    assert_eq!(c1 + c2, 1, "job should be claimed exactly once");

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobRunState::Success);

    let audits = runtime.list_audits(25).expect("audits");
    assert!(
        audits.iter().any(|audit| {
            audit.event_type == "JobRunCompleted"
                && audit.payload["data"]["job_id"].as_str() == Some(job_id.as_str())
        }),
        "job run completion should be recorded in audits"
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
            spec_type: "analysis".to_string(),
            description: "schema validation".to_string(),
            instruction: String::new(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            artifact_path_template: None,
            skill_refs: vec!["strict-schema".to_string()],
            identity_id: None,
            assigned_to: None,
            created_by: None,
        })
        .expect("add activity");
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-schema",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

    let due_at = runtime.show_job(&job_id).expect("show job").next_run_at;
    let ran = runtime.run_due_jobs(due_at).expect("run jobs");
    assert_eq!(ran, 1);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobRunState::Failed);
    assert_eq!(
        history[0].error_code.as_deref(),
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
            spec_type: "analysis".to_string(),
            description: "schema validation".to_string(),
            instruction: String::new(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            artifact_path_template: None,
            skill_refs: vec!["strict-complex".to_string()],
            identity_id: None,
            assigned_to: None,
            created_by: None,
        })
        .expect("add activity");
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-complex-schema",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );

    let due_at = runtime.show_job(&job_id).expect("show job").next_run_at;
    let ran = runtime.run_due_jobs(due_at).expect("run jobs");
    assert_eq!(ran, 1);

    let history = runtime.job_history(&job_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, JobRunState::Failed);
    assert_eq!(
        history[0].error_code.as_deref(),
        Some("AGENT_PROTOCOL_VIOLATION")
    );
}

#[test]
fn job_runtime_tick_once_reports_next_wake_time() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_activity(&runtime, "spec-next-wake");
    let job_id = add_scheduled_activity(
        &runtime,
        "spec-next-wake",
        &agent_cli,
        0,
        JobRetryBackoffStrategy::None,
        0,
    );
    let job = runtime.show_job(&job_id).expect("show job");

    let tick = JobRuntime::new(&runtime, JobRuntimeConfig::default())
        .tick_once(Utc::now())
        .expect("tick once");

    assert_eq!(tick.ran, 0);
    assert_eq!(tick.next_wake_at, Some(job.next_run_at));
}

#[test]
fn job_runtime_run_forever_stops_after_shutdown_request() {
    struct CountdownShutdown {
        checks: AtomicUsize,
    }

    impl ShutdownSignal for CountdownShutdown {
        fn should_stop(&self) -> bool {
            self.checks.fetch_add(1, Ordering::SeqCst) >= 1
        }
    }

    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let job_runtime = JobRuntime::new(
        &runtime,
        JobRuntimeConfig {
            idle_sleep: std::time::Duration::from_secs(0),
            max_sleep: std::time::Duration::from_secs(0),
        },
    );
    let shutdown = CountdownShutdown {
        checks: AtomicUsize::new(0),
    };

    job_runtime
        .run_forever(&shutdown)
        .expect("run forever exits cleanly");
}

#[test]
fn run_job_now_manual_schedule_does_not_error_on_cron_validation() {
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
            target_type: JobTargetType::Activity,
            target_id: "spec-manual-run".to_string(),
            schedule: "manual".to_string(),
            agent_cli,
            timeout_seconds: 10,
            retry_max_attempts: 0,
            retry_backoff_strategy: JobRetryBackoffStrategy::None,
            retry_initial_delay_seconds: 0,
            initial_state_override: None,
            env_extra: vec![],
        })
        .expect("add job")
        .job_id;

    let result = runtime
        .run_job_now(&job_id)
        .expect("manual job run must succeed");
    assert_eq!(result.state, JobRunState::Success);
}
