use std::sync::{Arc, Barrier};
use std::thread;

use orbit_core::OrbitRuntime;
use orbit_core::command::job::JobAddParams;
use orbit_core::command::work::WorkAddParams;
use orbit_types::{JobRetryBackoffStrategy, JobRunState, JobTargetType};
use serde_json::json;
use tempfile::tempdir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn add_work(runtime: &OrbitRuntime, id: &str) {
    let _ = runtime
        .add_work(WorkAddParams {
            id: id.to_string(),
            spec_type: "analysis".to_string(),
            description: "runtime test spec".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            artifact_path_template: None,
            skill_refs: Vec::new(),
        })
        .expect("add work");
}

#[test]
fn add_work_rejects_missing_skill_ref() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let result = runtime.add_work(WorkAddParams {
        id: "spec-missing-skill".to_string(),
        spec_type: "analysis".to_string(),
        description: "missing skill".to_string(),
        input_schema_json: json!({}),
        output_schema_json: json!({}),
        artifact_path_template: None,
        skill_refs: vec!["does-not-exist".to_string()],
    });
    assert!(result.is_err());
}

fn add_scheduled_job(
    runtime: &OrbitRuntime,
    target_id: &str,
    agent_cli: &str,
    retry_max_attempts: u32,
    retry_backoff_strategy: JobRetryBackoffStrategy,
    retry_initial_delay_seconds: u64,
) -> String {
    runtime
        .add_job(JobAddParams {
            target_type: JobTargetType::Work,
            target_id: target_id.to_string(),
            schedule: "every 1s".to_string(),
            agent_cli: agent_cli.to_string(),
            timeout_seconds: 10,
            retry_max_attempts,
            retry_backoff_strategy,
            retry_initial_delay_seconds,
        })
        .expect("add job")
        .job_id
}

fn write_agent_script(path: &std::path::Path, body: &str) -> String {
    std::fs::write(path, body).expect("write script");
    #[cfg(unix)]
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod script");
    path.to_string_lossy().to_string()
}

#[test]
fn scheduled_job_run_executes_agent_and_records_success_run() {
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

    add_work(&runtime, "spec-success");
    let job_id = add_scheduled_job(
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
    assert!(stdin_raw.contains("\"work\""));
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

    add_work(&runtime, "spec-protocol");
    let job_id = add_scheduled_job(
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
    assert_eq!(history[0].state, JobRunState::Failed);
    assert_eq!(
        history[0].error_code.as_deref(),
        Some("AGENT_PROTOCOL_VIOLATION")
    );

    let audits = runtime.list_audits(25).expect("audits");
    assert!(
        audits
            .iter()
            .any(|audit| audit.event_type == "JobProtocolViolation"),
        "protocol violations must be auditable"
    );
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

    add_work(&runtime, "spec-retry");
    let job_id = add_scheduled_job(
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
fn concurrent_job_run_invocations_do_not_double_run_job() {
    let dir = tempdir().expect("tempdir");
    let runtime = Arc::new(OrbitRuntime::from_data_root(dir.path()).expect("runtime"));
    let script_path = dir.path().join("mock-agent");
    let agent_cli = write_agent_script(
        &script_path,
        "#!/bin/sh\nsleep 0.2\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
    );

    add_work(&runtime, "spec-concurrent");
    let job_id = add_scheduled_job(
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
        r#"# strict-schema

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
        .add_work(WorkAddParams {
            id: "spec-schema".to_string(),
            spec_type: "analysis".to_string(),
            description: "schema validation".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            artifact_path_template: None,
            skill_refs: vec!["strict-schema".to_string()],
        })
        .expect("add work");
    let job_id = add_scheduled_job(
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
        r#"# strict-complex

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
        .add_work(WorkAddParams {
            id: "spec-complex-schema".to_string(),
            spec_type: "analysis".to_string(),
            description: "schema validation".to_string(),
            input_schema_json: json!({}),
            output_schema_json: json!({}),
            artifact_path_template: None,
            skill_refs: vec!["strict-complex".to_string()],
        })
        .expect("add work");
    let job_id = add_scheduled_job(
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
