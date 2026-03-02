use std::sync::{Arc, Barrier};
use std::thread;

use chrono::{Duration as ChronoDuration, Utc};
use orbit_core::OrbitRuntime;
use orbit_core::command::job::JobAddParams;
use orbit_core::command::scheduler::SchedulerAddParams;
use orbit_store::Store;
use orbit_types::{
    OrbitError, SchedulerRetryBackoffStrategy, SchedulerRunState, SchedulerTargetType,
};
use serde_json::json;
use tempfile::tempdir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn add_job(runtime: &OrbitRuntime, id: &str) {
    let _ = runtime
        .add_job(JobAddParams {
            id: id.to_string(),
            spec_type: "analysis".to_string(),
            description: "runtime test spec".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            artifact_path_template: None,
            skill_refs: Vec::new(),
            identity_id: None,
            assigned_to: None,
            created_by: None,
        })
        .expect("add job");
}

#[test]
fn add_job_rejects_missing_skill_ref() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let result = runtime.add_job(JobAddParams {
        id: "spec-missing-skill".to_string(),
        spec_type: "analysis".to_string(),
        description: "missing skill".to_string(),
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

fn add_scheduled_job(
    runtime: &OrbitRuntime,
    target_id: &str,
    agent_cli: &str,
    retry_max_attempts: u32,
    retry_backoff_strategy: SchedulerRetryBackoffStrategy,
    retry_initial_delay_seconds: u64,
) -> String {
    runtime
        .add_scheduler(SchedulerAddParams {
            target_type: SchedulerTargetType::Job,
            target_id: target_id.to_string(),
            schedule: "every 1s".to_string(),
            agent_cli: agent_cli.to_string(),
            timeout_seconds: 10,
            retry_max_attempts,
            retry_backoff_strategy,
            retry_initial_delay_seconds,
        })
        .expect("add scheduler")
        .scheduler_id
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

fn write_sqlite_scheduler_config(data_root: &std::path::Path) {
    let db_path = data_root.join("orbit.db").to_string_lossy().to_string();
    write_runtime_config(
        data_root,
        &format!("[scheduler]\npersistence = {{ type = \"sqlite\", path = \"{db_path}\" }}\n"),
    );
}

fn insert_stale_running_run(data_root: &std::path::Path, scheduler_id: &str) -> String {
    let store = Store::open(&data_root.join("orbit.db")).expect("open store");
    store
        .with_transaction(|tx| {
            let old_time = Utc::now() - ChronoDuration::hours(2);
            let run = tx.insert_scheduler_run(scheduler_id, 1, old_time)?;
            let changed = tx.mark_scheduler_run_running(&run.run_id, old_time)?;
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

    add_job(&runtime, "spec-success");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-success",
        &agent_cli,
        0,
        SchedulerRetryBackoffStrategy::None,
        0,
    );

    let due_at = runtime
        .show_scheduler(&scheduler_id)
        .expect("show scheduler")
        .next_run_at;
    let ran = runtime.run_due_schedulers(due_at).expect("run schedulers");
    assert_eq!(ran, 1);

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, SchedulerRunState::Success);
    assert_eq!(history[0].attempt, 1);
    assert!(history[0].agent_response_json.is_some());

    let args_raw = std::fs::read_to_string(args_capture).expect("args capture");
    assert!(args_raw.contains("--output"));
    assert!(args_raw.contains("json"));
    assert!(args_raw.contains("--target-type"));

    let stdin_raw = std::fs::read_to_string(stdin_capture).expect("stdin capture");
    assert!(stdin_raw.contains("\"schemaVersion\":1"));
    assert!(stdin_raw.contains("\"job\""));
    assert!(stdin_raw.contains("\"skills\""));
    assert!(stdin_raw.contains("\"input\""));
    assert!(stdin_raw.contains("\"memory\""));
}

#[test]
fn invalid_agent_json_marks_run_failed_with_protocol_violation() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(&script_path, "#!/bin/sh\nprintf 'not-json'\n");

    add_job(&runtime, "spec-protocol");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-protocol",
        &agent_cli,
        0,
        SchedulerRetryBackoffStrategy::None,
        0,
    );

    let due_at = runtime
        .show_scheduler(&scheduler_id)
        .expect("show scheduler")
        .next_run_at;
    let ran = runtime.run_due_schedulers(due_at).expect("run schedulers");
    assert_eq!(ran, 1);

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, SchedulerRunState::Failed);
    assert_eq!(
        history[0].error_code.as_deref(),
        Some("AGENT_PROTOCOL_VIOLATION")
    );

    let audits = runtime.list_audits(25).expect("audits");
    assert!(
        audits
            .iter()
            .any(|audit| audit.event_type == "SchedulerProtocolViolation"),
        "protocol violations must be auditable"
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

    add_job(&runtime, "spec-invocation");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-invocation",
        &agent_cli,
        0,
        SchedulerRetryBackoffStrategy::None,
        0,
    );

    let due_at = runtime
        .show_scheduler(&scheduler_id)
        .expect("show scheduler")
        .next_run_at;
    let ran = runtime.run_due_schedulers(due_at).expect("run schedulers");
    assert_eq!(ran, 1);

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, SchedulerRunState::Failed);
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
fn codex_scheduler_run_fails_fast_when_required_env_var_is_not_allowlisted() {
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

    add_job(&runtime, "spec-codex-missing-env");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-codex-missing-env",
        &agent_cli,
        0,
        SchedulerRetryBackoffStrategy::None,
        0,
    );

    let run = runtime
        .run_scheduler_now(&scheduler_id)
        .expect("run scheduler");
    assert_eq!(run.state, SchedulerRunState::Failed);

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
    assert_eq!(
        history[0].error_code.as_deref(),
        Some("AGENT_INVOCATION_FAILED")
    );
    let message = history[0].error_message.as_deref().unwrap_or_default();
    assert!(message.contains("HOME"));
    assert!(message.contains("config.toml"));
}

#[test]
fn claude_scheduler_run_fails_fast_when_required_env_var_is_not_allowlisted() {
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

    add_job(&runtime, "spec-claude-missing-env");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-claude-missing-env",
        &agent_cli,
        0,
        SchedulerRetryBackoffStrategy::None,
        0,
    );

    let run = runtime
        .run_scheduler_now(&scheduler_id)
        .expect("run scheduler");
    assert_eq!(run.state, SchedulerRunState::Failed);

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
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

    add_job(&runtime, "spec-provider-env-present");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-provider-env-present",
        &agent_cli,
        0,
        SchedulerRetryBackoffStrategy::None,
        0,
    );

    let run = runtime
        .run_scheduler_now(&scheduler_id)
        .expect("run scheduler");
    assert_eq!(run.state, SchedulerRunState::Failed);

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
    assert_eq!(
        history[0].error_code.as_deref(),
        Some("AGENT_PROTOCOL_VIOLATION")
    );
}

#[test]
fn run_scheduler_now_applies_retry_policy_and_second_attempt_can_succeed() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let marker = dir.path().join("retry.marker");
    let script_path = dir.path().join("mock-agent");
    let script = format!(
        "#!/bin/sh\nif [ -f \"{marker}\" ]; then\n  printf '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null,\"durationMs\":1}}'\n  exit 0\nfi\ntouch \"{marker}\"\nprintf '{{\"schemaVersion\":1,\"status\":\"failed\",\"result\":null,\"error\":{{\"code\":\"FIRST_FAIL\",\"message\":\"first attempt fails\",\"details\":{{}}}},\"durationMs\":1}}'\nexit 1\n",
        marker = marker.to_string_lossy()
    );
    let agent_cli = write_agent_script(&script_path, &script);

    add_job(&runtime, "spec-retry");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-retry",
        &agent_cli,
        1,
        SchedulerRetryBackoffStrategy::Fixed,
        0,
    );

    let result = runtime.run_scheduler_now(&scheduler_id).expect("run now");
    assert_eq!(result.state, SchedulerRunState::Success);
    assert_eq!(result.attempt, 2);

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].attempt, 2);
    assert_eq!(history[0].state, SchedulerRunState::Success);
    assert_eq!(history[1].attempt, 1);
    assert_eq!(history[1].state, SchedulerRunState::Failed);
}

#[test]
fn run_scheduler_now_rejects_when_active_run_exists() {
    let dir = tempdir().expect("tempdir");
    let runtime = Arc::new(OrbitRuntime::from_data_root(dir.path()).expect("runtime"));
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nsleep 0.5\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_job(&runtime, "spec-active-lock");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-active-lock",
        &agent_cli,
        0,
        SchedulerRetryBackoffStrategy::None,
        0,
    );

    let r1 = Arc::clone(&runtime);
    let scheduler_id_thread = scheduler_id.clone();
    let handle = thread::spawn(move || r1.run_scheduler_now(&scheduler_id_thread));
    thread::sleep(std::time::Duration::from_millis(100));

    let err = runtime
        .run_scheduler_now(&scheduler_id)
        .expect_err("second run should be rejected while first is active");
    assert!(matches!(err, OrbitError::SchedulerValidation(_)));
    assert!(err.to_string().contains("already has an active run"));

    let first = handle.join().expect("join");
    assert!(first.is_ok(), "first run should complete successfully");

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
    assert_eq!(
        history.len(),
        1,
        "second invocation must not insert a pending row"
    );
    assert_eq!(history[0].state, SchedulerRunState::Success);
}

#[test]
fn scheduler_history_recovers_stale_running_run_to_failed() {
    let dir = tempdir().expect("tempdir");
    write_sqlite_scheduler_config(dir.path());
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_job(&runtime, "spec-history-stale");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-history-stale",
        &agent_cli,
        0,
        SchedulerRetryBackoffStrategy::None,
        0,
    );
    let stale_run_id = insert_stale_running_run(dir.path(), &scheduler_id);

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
    let stale = history
        .iter()
        .find(|run| run.run_id == stale_run_id)
        .expect("stale run should exist");
    assert_eq!(stale.state, SchedulerRunState::Failed);
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
fn run_scheduler_now_recovers_stale_running_run_and_executes_new_attempt() {
    let dir = tempdir().expect("tempdir");
    write_sqlite_scheduler_config(dir.path());
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_job(&runtime, "spec-run-now-stale");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-run-now-stale",
        &agent_cli,
        0,
        SchedulerRetryBackoffStrategy::None,
        0,
    );
    let stale_run_id = insert_stale_running_run(dir.path(), &scheduler_id);

    let result = runtime.run_scheduler_now(&scheduler_id).expect("run now");
    assert_eq!(result.state, SchedulerRunState::Success);

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
    assert!(
        history.iter().any(|run| run.run_id == stale_run_id),
        "stale run should still be present in history"
    );
    let stale = history
        .iter()
        .find(|run| run.run_id == stale_run_id)
        .expect("stale run should exist");
    assert_eq!(stale.state, SchedulerRunState::Failed);
    assert_eq!(stale.error_code.as_deref(), Some("AGENT_INVOCATION_FAILED"));
    assert!(
        history
            .iter()
            .any(|run| run.state == SchedulerRunState::Success),
        "new attempt should complete successfully"
    );
}

#[test]
fn run_due_schedulers_recovers_stale_running_run_and_reclaims_scheduler() {
    let dir = tempdir().expect("tempdir");
    write_sqlite_scheduler_config(dir.path());
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_job(&runtime, "spec-due-stale");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-due-stale",
        &agent_cli,
        0,
        SchedulerRetryBackoffStrategy::None,
        0,
    );
    let stale_run_id = insert_stale_running_run(dir.path(), &scheduler_id);

    let due_at = runtime
        .show_scheduler(&scheduler_id)
        .expect("show scheduler")
        .next_run_at;
    let ran = runtime
        .run_due_schedulers(due_at)
        .expect("run due schedulers");
    assert_eq!(
        ran, 1,
        "scheduler should be reclaimed after stale run recovery"
    );

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
    let stale = history
        .iter()
        .find(|run| run.run_id == stale_run_id)
        .expect("stale run should exist");
    assert_eq!(stale.state, SchedulerRunState::Failed);
    assert!(
        history
            .iter()
            .any(|run| run.state == SchedulerRunState::Success),
        "reclaimed due scheduler should complete successfully"
    );
}

#[test]
fn concurrent_scheduler_run_invocations_do_not_double_run_scheduler() {
    let dir = tempdir().expect("tempdir");
    let runtime = Arc::new(OrbitRuntime::from_data_root(dir.path()).expect("runtime"));
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nsleep 0.2\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_job(&runtime, "spec-concurrent");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-concurrent",
        &agent_cli,
        0,
        SchedulerRetryBackoffStrategy::None,
        0,
    );

    let due_at = runtime
        .show_scheduler(&scheduler_id)
        .expect("show scheduler")
        .next_run_at;
    let barrier = Arc::new(Barrier::new(3));

    let r1 = Arc::clone(&runtime);
    let b1 = Arc::clone(&barrier);
    let due_one = due_at;
    let t1 = thread::spawn(move || {
        b1.wait();
        r1.run_due_schedulers(due_one).expect("thread 1 run")
    });

    let r2 = Arc::clone(&runtime);
    let b2 = Arc::clone(&barrier);
    let due_two = due_at;
    let t2 = thread::spawn(move || {
        b2.wait();
        r2.run_due_schedulers(due_two).expect("thread 2 run")
    });

    barrier.wait();

    let c1 = t1.join().expect("join t1");
    let c2 = t2.join().expect("join t2");
    assert_eq!(c1 + c2, 1, "scheduler should be claimed exactly once");

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, SchedulerRunState::Success);

    let audits = runtime.list_audits(25).expect("audits");
    assert!(
        audits.iter().any(|audit| {
            audit.event_type == "SchedulerRunCompleted"
                && audit.payload["data"]["scheduler_id"].as_str() == Some(scheduler_id.as_str())
        }),
        "scheduler run completion should be recorded in audits"
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
        .add_job(JobAddParams {
            id: "spec-schema".to_string(),
            spec_type: "analysis".to_string(),
            description: "schema validation".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            artifact_path_template: None,
            skill_refs: vec!["strict-schema".to_string()],
            identity_id: None,
            assigned_to: None,
            created_by: None,
        })
        .expect("add job");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-schema",
        &agent_cli,
        0,
        SchedulerRetryBackoffStrategy::None,
        0,
    );

    let due_at = runtime
        .show_scheduler(&scheduler_id)
        .expect("show scheduler")
        .next_run_at;
    let ran = runtime.run_due_schedulers(due_at).expect("run schedulers");
    assert_eq!(ran, 1);

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, SchedulerRunState::Failed);
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
        .add_job(JobAddParams {
            id: "spec-complex-schema".to_string(),
            spec_type: "analysis".to_string(),
            description: "schema validation".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            artifact_path_template: None,
            skill_refs: vec!["strict-complex".to_string()],
            identity_id: None,
            assigned_to: None,
            created_by: None,
        })
        .expect("add job");
    let scheduler_id = add_scheduled_job(
        &runtime,
        "spec-complex-schema",
        &agent_cli,
        0,
        SchedulerRetryBackoffStrategy::None,
        0,
    );

    let due_at = runtime
        .show_scheduler(&scheduler_id)
        .expect("show scheduler")
        .next_run_at;
    let ran = runtime.run_due_schedulers(due_at).expect("run schedulers");
    assert_eq!(ran, 1);

    let history = runtime.scheduler_history(&scheduler_id).expect("history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, SchedulerRunState::Failed);
    assert_eq!(
        history[0].error_code.as_deref(),
        Some("AGENT_PROTOCOL_VIOLATION")
    );
}
