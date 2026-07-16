#![allow(missing_docs)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use orbit_agent::loop_engine::audit::AuditSink;
use orbit_common::test_fixtures::{TEST_CLAUDE_MODEL, TEST_CODEX_MODEL};
use orbit_common::types::activity_job::{AgentRole, V2AuditEventKind};
use orbit_store::Store;
use tempfile::tempdir;

use super::super::super::agent_role::{apply_resolved_settings, resolve_agent_settings};
use super::super::super::audit_writer::V2AuditWriter;
use super::super::super::dispatcher::DispatchError;
use super::super::super::sqlite_sink::V2SqliteSink;
use super::super::run_cli_backend;
use super::test_support::{
    RecordingSink, TestHost, capture_events, test_agent_loop_spec, test_agent_loop_spec_for,
    write_executable,
};

#[test]
fn run_cli_backend_finished_audit_event_keeps_stdout_stderr_blob_refs() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\nprintf '%s\\n' '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null}'\nprintf '%s\\n' 'plain stderr' >&2\n",
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink.clone();
    let audit = Arc::new(V2AuditWriter::new(
        "job-audit",
        "codex:gpt-5.5",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let spec = test_agent_loop_spec(Duration::from_secs(5));
    let input = serde_json::json!({
        "prompt": "do it",
        "task_id": "TAUDIT"
    });

    let outcome = run_cli_backend(&host, &spec, "job-audit", audit.clone(), &input, None)
        .expect("run succeeds");

    assert!(outcome.success);
    let stdout = "{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null}\n";
    assert_eq!(outcome.output["stdout_text"], stdout);
    assert_eq!(outcome.output["stdout_text_truncated"], false);
    assert_eq!(outcome.output["stdout_text_original_bytes"], stdout.len());
    let events = audit.events_snapshot().expect("events snapshot");
    let finished = events
        .iter()
        .find_map(|event| match &event.kind {
            V2AuditEventKind::CliInvocationFinished {
                provider,
                exit_code,
                stdout_blob_ref,
                stderr_blob_ref,
                timed_out,
                ..
            } => Some((
                provider.as_str(),
                *exit_code,
                stdout_blob_ref.as_deref(),
                stderr_blob_ref.as_deref(),
                *timed_out,
            )),
            _ => None,
        })
        .expect("finished event");

    assert_eq!(finished.0, "codex");
    assert_eq!(finished.1, Some(0));
    assert_eq!(finished.2, Some("blob-2"));
    assert_eq!(finished.3, Some("blob-3"));
    assert!(!finished.4);
    assert_eq!(sink.blob("blob-2"), Some(stdout.as_bytes().to_vec()));
    assert_eq!(sink.blob("blob-3"), Some(b"plain stderr\n".to_vec()));
}

#[test]
fn run_cli_backend_projects_prose_prefixed_claude_envelope_result() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("claude");
    let response = serde_json::json!({
        "type": "result",
        "subtype": "success",
        "result": format!(
            "I classified both failed runs.\n{}",
            serde_json::json!({
                "schemaVersion": 1,
                "status": "success",
                "result": {
                    "dispositions": [
                        {
                            "task_id": "ORB-A",
                            "classification": "environmental",
                            "disposition": "rebacklog",
                            "diagnosis": "stale worktree removed"
                        },
                        {
                            "task_id": "ORB-B",
                            "classification": "code_defect",
                            "disposition": "stay_blocked",
                            "diagnosis": "tests remain red"
                        }
                    ],
                    "summary": "one recovery and one human follow-up"
                },
                "error": null
            })
        ),
        "usage": {
            "input_tokens": 11,
            "output_tokens": 7
        }
    })
    .to_string();
    write_executable(
        &script,
        &format!("#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{response}'\n"),
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-claude-envelope-result",
        "claude:sonnet",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let spec = test_agent_loop_spec_for("claude", Duration::from_secs(5));

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-claude-envelope-result",
        audit,
        &serde_json::json!({"prompt": "triage failed runs"}),
        None,
    )
    .expect("run cli backend");

    assert!(outcome.success);
    assert_eq!(
        outcome.output["dispositions"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(
        outcome.output["dispositions"][0]["disposition"],
        serde_json::json!("rebacklog")
    );
    assert_eq!(
        outcome.output["dispositions"][1]["disposition"],
        serde_json::json!("stay_blocked")
    );
    assert_eq!(
        outcome.output["summary"],
        serde_json::json!("one recovery and one human follow-up")
    );
    assert_eq!(outcome.output["provider"], serde_json::json!("claude"));
}

#[test]
fn run_cli_backend_rejects_schema_invalid_success_envelope() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"schemaVersion\":2,\"status\":\"success\",\"result\":{},\"error\":null}'\n",
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-invalid-envelope",
        "codex:gpt-5.5",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let mut spec = test_agent_loop_spec(Duration::from_secs(5));
    spec.require_response_envelope = true;

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-invalid-envelope",
        audit,
        &serde_json::json!({"prompt": "hi"}),
        None,
    )
    .expect("run cli backend");

    assert!(!outcome.success);
    let message = outcome.message.expect("invalid envelope message");
    assert!(
        message.contains("cli response envelope invalid"),
        "{message}"
    );
    assert!(
        message.contains("unsupported schemaVersion: 2"),
        "{message}"
    );
}

#[test]
fn run_cli_backend_accepts_claude_success_prose_without_envelope_for_artifact_activity() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("claude");
    let stdout = serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "result": "All changes are complete and validated. Execution summary persisted to the task.",
        "stop_reason": "end_turn"
    })
    .to_string();
    write_executable(
        &script,
        &format!("#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{stdout}'\n"),
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-artifact-response",
        "claude:sonnet",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let spec = test_agent_loop_spec_for("claude", Duration::from_secs(5));

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-artifact-response",
        audit,
        &serde_json::json!({"task_id": "ORB-10230"}),
        None,
    )
    .expect("run cli backend");

    assert!(outcome.success);
    assert!(outcome.message.is_none());
    assert_eq!(outcome.output["exit_code"], 0);
    assert_eq!(outcome.output["response_envelope_required"], false);
    assert_eq!(outcome.output["response_envelope_valid"], false);
    assert!(
        outcome.output["response_envelope_error"]
            .as_str()
            .is_some_and(|message| message.contains("does not contain an Orbit response envelope"))
    );
}

#[test]
fn run_cli_backend_requires_envelope_when_activity_opts_in() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("claude");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"result\":\"completed\"}'\n",
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-required-response",
        "claude:sonnet",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let mut spec = test_agent_loop_spec_for("claude", Duration::from_secs(5));
    spec.require_response_envelope = true;

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-required-response",
        audit,
        &serde_json::json!({"prompt": "return structured data"}),
        None,
    )
    .expect("run cli backend");

    assert!(!outcome.success);
    assert_eq!(outcome.output["response_envelope_required"], true);
    assert_eq!(outcome.output["response_envelope_valid"], false);
    assert!(
        outcome
            .message
            .as_deref()
            .is_some_and(|message| message.contains("does not contain an Orbit response envelope"))
    );
}

#[test]
fn run_cli_backend_keeps_verbose_provider_result_usable_after_capture_truncation() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    write_executable(
        &script,
        r#"#!/bin/sh
cat > /dev/null
printf '%s' '{"type":"item.completed","item":{"type":"reasoning","text":"'
i=0
while [ "$i" -lt 18000 ]; do
  printf '%s' 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'
  i=$((i + 1))
done
printf '%s\n' '"}}'
printf '%s\n' '{"schemaVersion":1,"status":"success","result":{"workflow":"usable"},"error":null}'
"#,
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink.clone();
    let audit = Arc::new(V2AuditWriter::new(
        "job-verbose-output",
        "codex:gpt-5.5",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let spec = test_agent_loop_spec(Duration::from_secs(10));

    let (outcome, _events) = capture_events(|| {
        run_cli_backend(
            &host,
            &spec,
            "job-verbose-output",
            audit,
            &serde_json::json!({"prompt": "perform verbose work"}),
            None,
        )
    });
    let outcome = outcome.expect("verbose run succeeds");

    assert!(outcome.success, "capture truncation must not fail the run");
    assert!(outcome.message.is_none());
    assert_eq!(outcome.output["exit_code"], 0);
    assert_eq!(outcome.output["timed_out"], false);
    assert_eq!(outcome.output["stdout_capture_truncated"], true);
    let observed = outcome.output["stdout_text_original_bytes"]
        .as_u64()
        .expect("observed byte count");
    let limit = outcome.output["stdout_capture_limit_bytes"]
        .as_u64()
        .expect("capture limit");
    let captured = outcome.output["stdout_text_captured_bytes"]
        .as_u64()
        .expect("captured byte count");
    assert!(observed > limit);
    assert!(captured < observed);

    let preview = outcome.output["stdout_text"]
        .as_str()
        .expect("stdout protocol tail");
    let documents = serde_json::Deserializer::from_str(preview)
        .into_iter::<serde_json::Value>()
        .collect::<Result<Vec<_>, _>>()
        .expect("retained stdout text remains valid JSONL");
    assert_eq!(
        documents
            .last()
            .and_then(|value| value.get("status"))
            .and_then(serde_json::Value::as_str),
        Some("success")
    );

    let stdout_blob_ref = outcome.output["stdout_blob_ref"]
        .as_str()
        .expect("stdout blob ref");
    let stored = sink.blob(stdout_blob_ref).expect("stored bounded stdout");
    assert!(stored.len() < observed as usize);
    assert!(
        String::from_utf8_lossy(&stored).contains("observed_bytes="),
        "bounded blob should record why and where capture was truncated"
    );
}

#[test]
fn run_cli_backend_bounds_stdout_text_preview_and_keeps_envelope_status_from_full_stdout() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    let embedded_envelope = r#"{"schemaVersion":1,"status":"failed","error":{"code":"workspace_unavailable","message":"worktree missing","details":null}}"#;
    let stdout = serde_json::json!({
        "type": "result",
        "subtype": "success",
        "result": format!("{}{}", "x".repeat(70 * 1024), embedded_envelope),
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1
        }
    })
    .to_string();
    write_executable(
        &script,
        &format!("#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{stdout}'\n"),
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-stdout-preview",
        "codex:gpt-5.5",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let mut spec = test_agent_loop_spec(Duration::from_secs(5));
    spec.require_response_envelope = true;

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-stdout-preview",
        audit,
        &serde_json::json!({"prompt": "hi"}),
        None,
    )
    .expect("run succeeds");

    assert!(
        !outcome.success,
        "status=failed after the preview limit must still demote success"
    );
    let preview = outcome.output["stdout_text"]
        .as_str()
        .expect("stdout_text preview");
    assert!(preview.len() <= 64 * 1024);
    assert!(!preview.contains("workspace_unavailable"));
    assert_eq!(outcome.output["stdout_text_truncated"], true);
    assert_eq!(
        outcome.output["stdout_text_preview_bytes"].as_u64(),
        Some(preview.len() as u64)
    );
    assert_eq!(
        outcome.output["stdout_text_preview_limit_bytes"].as_u64(),
        Some((64 * 1024) as u64)
    );
    let message = outcome.message.expect("expected demote message");
    assert!(
        message.contains("envelope status") && message.contains("failed"),
        "demote message should explain envelope status; got {message:?}"
    );
}

#[test]
fn run_cli_backend_redacts_secret_like_stdout_text_preview() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    write_executable(
        &script,
        r#"#!/bin/sh
cat > /dev/null
printf '%s\n' '{"log":"Authorization: Bearer stdout-secret-token"}'
printf '%s\n' '{"x-api-key":"stdout-header-key"}'
printf '%s\n' '{"log":"sk-stdoutsecret123"}'
printf '%s\n' '{"api_key":"stdout-json-key"}'
printf '%s\n' '{"schemaVersion":1,"status":"success","result":{},"error":null}'
"#,
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-stdout-redaction",
        "codex:gpt-5.5",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let spec = test_agent_loop_spec(Duration::from_secs(5));

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-stdout-redaction",
        audit,
        &serde_json::json!({"prompt": "hi"}),
        None,
    )
    .expect("run succeeds");

    assert!(outcome.success);
    let preview = outcome.output["stdout_text"]
        .as_str()
        .expect("stdout_text preview");
    assert!(!preview.contains("stdout-secret-token"));
    assert!(!preview.contains("stdout-header-key"));
    assert!(!preview.contains("stdout-json-key"));
    assert!(!preview.contains("sk-stdoutsecret123"));
    assert!(preview.contains("[REDACTED_AUTH]"));
    assert!(preview.contains("[REDACTED_API_KEY]"));
    assert_eq!(outcome.output["stdout_text_truncated"], false);
    assert_eq!(
        outcome.output["stdout_blob_ref"].as_str(),
        Some("blob-2"),
        "full stdout should remain available via blob ref"
    );
}

#[test]
fn run_cli_backend_redacts_live_env_values_in_stored_blobs() {
    let temp = tempdir().expect("tempdir");
    let secret = "live-cli-blob-secret-value";
    let _guard = EnvVarGuard::set("ORBIT_CLI_BLOB_TEST_TOKEN", secret);
    let script = temp.path().join("codex");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{{\"log\":\"stdout leak {secret}\"}}'\nprintf '%s\\n' '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null}}'\nprintf '%s\\n' 'stderr leak {secret}' >&2\n"
        ),
    );

    let loop_sink = Arc::new(V2SqliteSink::new(
        Store::open_in_memory().expect("open sqlite store"),
        "ws-test",
        "job-cli-blob-redaction",
        "codex:gpt-5.5",
        None,
        temp.path().join("audit").join("blobs"),
    ));
    let sink_for_writer: Arc<dyn AuditSink> = loop_sink.clone();
    let audit = Arc::new(V2AuditWriter::new(
        "job-cli-blob-redaction",
        "codex:gpt-5.5",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let spec = test_agent_loop_spec(Duration::from_secs(5));

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-cli-blob-redaction",
        audit,
        &serde_json::json!({"prompt": format!("provider stdin contains {secret}")}),
        None,
    )
    .expect("run succeeds");

    assert!(outcome.success);
    for key in ["stdin_blob_ref", "stdout_blob_ref", "stderr_blob_ref"] {
        let blob_ref = outcome.output[key].as_str().expect("blob ref");
        let text = String::from_utf8(
            loop_sink
                .blob_store()
                .read(blob_ref)
                .expect("read stored blob"),
        )
        .expect("stored blob utf8");
        assert!(
            !text.contains(secret),
            "{key} should not contain raw live env value: {text}"
        );
        assert!(
            text.contains("[REDACTED_ENV]"),
            "{key} should include env redaction marker: {text}"
        );
    }
}

#[test]
fn run_cli_backend_returns_error_when_declared_workspace_path_missing() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null}'\n",
    );
    let missing = temp.path().join("missing-worktree");

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-missing-cwd",
        "codex:gpt-5.5",
        sink_for_writer,
    ));
    let host = TestHost {
        command: script.display().to_string(),
        executor_args: Vec::new(),
        provider_config: HashMap::new(),
        sandbox: None,
        task_context: Some(serde_json::json!({
            "workspace_path": missing.display().to_string()
        })),
    };
    let spec = test_agent_loop_spec(Duration::from_secs(5));
    let input = serde_json::json!({
        "prompt": "do it",
        "task_id": "TMISSING"
    });

    let err = run_cli_backend(&host, &spec, "job-missing-cwd", audit.clone(), &input, None)
        .expect_err("missing declared workspace should fail");
    match err {
        DispatchError::CliInvocationFailed(message) => {
            assert!(
                message.contains(&missing.display().to_string()),
                "error should name missing path: {message}"
            );
        }
        other => panic!("expected CliInvocationFailed, got {other:?}"),
    }

    let events = audit.events_snapshot().expect("events snapshot");
    assert!(
        !events
            .iter()
            .any(|event| matches!(&event.kind, V2AuditEventKind::CliInvocationStarted { .. })),
        "CliInvocationStarted should not be emitted before cwd validation succeeds"
    );
}

#[test]
fn run_cli_backend_records_resolved_cwd_in_started_event() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null}'\n",
    );
    let workspace_dir = tempdir().expect("workspace tempdir");
    let workspace = workspace_dir
        .path()
        .canonicalize()
        .expect("canonical workspace");
    let workspace_string = workspace.display().to_string();

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-cwd-audit",
        "codex:gpt-5.5",
        sink_for_writer,
    ));
    let host = TestHost {
        command: script.display().to_string(),
        executor_args: Vec::new(),
        provider_config: HashMap::new(),
        sandbox: None,
        task_context: Some(serde_json::json!({
            "workspace_path": workspace_string.clone()
        })),
    };
    let spec = test_agent_loop_spec(Duration::from_secs(5));

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-cwd-audit",
        audit.clone(),
        &serde_json::json!({ "prompt": "do it", "task_id": "TCWD" }),
        None,
    )
    .expect("run succeeds");
    assert!(outcome.success);

    let events = audit.events_snapshot().expect("events snapshot");
    let cwd = events
        .iter()
        .find_map(|event| match &event.kind {
            V2AuditEventKind::CliInvocationStarted { cwd, .. } => cwd.as_deref(),
            _ => None,
        })
        .expect("cli.invocation.started cwd");
    assert_eq!(cwd, workspace_string);
}

#[test]
fn run_cli_backend_passes_provider_config_to_codex_runtime_args() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null}'\n",
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-config",
        "codex:gpt-5.5",
        sink_for_writer,
    ));
    let mut provider_config = HashMap::new();
    provider_config.insert("sandbox".to_string(), "danger-full-access".to_string());
    provider_config.insert("approval_policy".to_string(), "never".to_string());
    provider_config.insert(
        "writable_dirs_json".to_string(),
        r#"["/tmp/orbit-a","/tmp/orbit-b"]"#.to_string(),
    );
    let host = TestHost {
        command: script.display().to_string(),
        executor_args: Vec::new(),
        provider_config,
        sandbox: None,
        task_context: None,
    };
    let spec = test_agent_loop_spec(Duration::from_secs(5));

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-config",
        audit.clone(),
        &serde_json::json!({ "prompt": "do it" }),
        None,
    )
    .expect("run succeeds");

    assert!(outcome.success);
    let events = audit.events_snapshot().expect("events snapshot");
    let argv = events
        .iter()
        .find_map(|event| match &event.kind {
            V2AuditEventKind::CliInvocationStarted { argv_redacted, .. } => Some(argv_redacted),
            _ => None,
        })
        .expect("cli.invocation.started event");

    assert_eq!(
        argv,
        &vec![
            script.display().to_string(),
            "--config".to_string(),
            "approval_policy=\"never\"".to_string(),
            "--sandbox".to_string(),
            "danger-full-access".to_string(),
            "--add-dir".to_string(),
            "/tmp/orbit-a".to_string(),
            "--add-dir".to_string(),
            "/tmp/orbit-b".to_string(),
        ]
    );
}

#[test]
fn run_cli_backend_passes_model_to_grok_and_captures_well_formed_stdout() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("grok");
    let grok_stdout = serde_json::json!({
        "text": "{\"schemaVersion\":1,\"status\":\"success\",\"result\":{\"pong\":\"grok-smoke\"},\"error\":null}",
        "stopReason": "EndTurn"
    })
    .to_string();
    write_executable(
        &script,
        &format!("#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{grok_stdout}'\n"),
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-grok-model",
        "grok:grok-build",
        sink_for_writer,
    ));
    let host = TestHost {
        command: script.display().to_string(),
        executor_args: vec![
            "--output-format".to_string(),
            "json".to_string(),
            "--prompt-file".to_string(),
            "/dev/stdin".to_string(),
        ],
        provider_config: HashMap::new(),
        sandbox: None,
        task_context: None,
    };
    let mut spec = test_agent_loop_spec_for("grok", Duration::from_secs(5));
    spec.model = Some("grok-build".to_string());

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-grok-model",
        audit.clone(),
        &serde_json::json!({"prompt": "hi"}),
        None,
    )
    .expect("run succeeds");

    assert!(outcome.success);
    assert!(outcome.invocation.is_some());
    assert_eq!(outcome.output["provider"], "grok");
    assert_eq!(outcome.output["stdout_blob_ref"].as_str(), Some("blob-2"));
    assert!(
        outcome
            .output
            .get("stdout_text")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| text.contains("grok-smoke")),
        "stdout preview should include the grok response"
    );

    let events = audit.events_snapshot().expect("events snapshot");
    let argv = events
        .iter()
        .find_map(|event| match &event.kind {
            V2AuditEventKind::CliInvocationStarted { argv_redacted, .. } => Some(argv_redacted),
            _ => None,
        })
        .expect("cli.invocation.started event");
    let model_idx = argv
        .iter()
        .position(|arg| arg == "--model")
        .expect("grok argv should include --model");
    assert_eq!(
        argv.get(model_idx + 1).map(String::as_str),
        Some("grok-build")
    );
}

#[test]
fn run_cli_backend_exports_runtime_identity_for_subprocess_tools() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("grok");
    write_executable(
        &script,
        r#"#!/bin/sh
cat > /dev/null
if [ "$ORBIT_AGENT_NAME" = "grok" ] && [ "$ORBIT_AGENT_MODEL" = "grok-build" ]; then
  printf '%s\n' '{"schemaVersion":1,"status":"success","result":{"identity":"ok"},"error":null}'
else
  printf '%s\n' '{"schemaVersion":1,"status":"failed","error":{"code":"identity_env_missing","message":"runtime identity env was not propagated","details":null}}'
  exit 1
fi
"#,
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-grok-identity-env",
        "grok:grok-build",
        sink_for_writer,
    ));
    let host = TestHost {
        command: script.display().to_string(),
        executor_args: Vec::new(),
        provider_config: HashMap::new(),
        sandbox: None,
        task_context: None,
    };
    let mut spec = test_agent_loop_spec_for("grok", Duration::from_secs(5));
    spec.model = Some("grok-build".to_string());

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-grok-identity-env",
        audit,
        &serde_json::json!({"prompt": "hi"}),
        None,
    )
    .expect("run succeeds");

    assert!(outcome.success);
    assert_eq!(outcome.output["provider"], "grok");
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: this test uses a dedicated variable name and restores the
        // previous value on drop.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: see EnvVarGuard::set.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

/// Regression for T20260508-17: a structured-output activity that opts into
/// strict response validation must demote an exit-0 subprocess whose embedded
/// Orbit response reports `status: "failed"`.
#[test]
fn run_cli_backend_demotes_success_when_envelope_reports_failed_despite_exit_zero() {
    let temp = tempdir().expect("tempdir");
    // The agent config layer infers provider from the command basename, so
    // the script name must match a known provider. The demotion logic is
    // provider-agnostic — codex exercises the same code path as claude.
    let script = temp.path().join("codex");
    // Stdout shape mirrors the observed Claude CLI failure: a wrapping JSON
    // whose `result` string starts with prose before embedding an Orbit
    // envelope with status="failed". Exit 0.
    let stdout = serde_json::json!({
        "type": "result",
        "subtype": "success",
        "result": concat!(
            "I could not continue after the workspace disappeared.\n",
            r#"{"schemaVersion":1,"status":"failed","error":{"code":"workspace_unavailable","message":"worktree missing","details":null}}"#
        ),
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1
        }
    })
    .to_string();
    write_executable(
        &script,
        &format!("#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{stdout}'\n"),
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-success-demote",
        "claude:s",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let mut spec = test_agent_loop_spec(Duration::from_secs(5));
    spec.require_response_envelope = true;

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-success-demote",
        audit,
        &serde_json::json!({"prompt": "hi"}),
        None,
    )
    .expect("run cli backend");

    assert!(
        !outcome.success,
        "envelope status=failed must demote dispatch success even on exit 0"
    );
    let message = outcome.message.expect("expected demote message");
    assert!(
        message.contains("envelope status") && message.contains("failed"),
        "demote message should explain envelope status; got {message:?}"
    );
}

/// Sanity check that the demotion does not regress the happy path: an exit-0
/// run with a `status: "success"` envelope must still be classified as
/// success. Without this, the demotion logic could silently flip every
/// claude run to failed.
#[test]
fn run_cli_backend_keeps_success_when_envelope_reports_success() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{}}'\n",
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-success-keep",
        "claude:s",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let spec = test_agent_loop_spec(Duration::from_secs(5));

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-success-keep",
        audit,
        &serde_json::json!({"prompt": "hi"}),
        None,
    )
    .expect("run cli backend");
    assert!(
        outcome.success,
        "envelope status=success must keep dispatch success on exit 0"
    );
}

/// Crew-driven regression test for ORB-00080 AC #15: a mixed fixture crew must
/// produce `--model claude-opus-4-7` for planner and `--model gpt-5.5` for
/// implementer (identity attribution stays family; no leakage of family name
/// into the --model flag that reaches the CLI).
#[test]
fn mixed_crew_drives_exact_models_to_planner_and_implementer() {
    let temp = tempdir().expect("tempdir");
    let claude_script = temp.path().join("claude");
    let codex_script = temp.path().join("codex");
    write_executable(
        &claude_script,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null}\\n'\n",
    );
    write_executable(
        &codex_script,
        "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null}\\n'\n",
    );

    // planner leg via mixed fixture crew
    let sink_for_writer_p: Arc<dyn AuditSink> = Arc::new(RecordingSink::default());
    let audit_p = Arc::new(V2AuditWriter::new(
        "job-crew-planner",
        format!("claude:{TEST_CLAUDE_MODEL}"),
        sink_for_writer_p,
    ));
    let host_p = TestHost::with_command(claude_script.display().to_string());
    let mut spec_p = test_agent_loop_spec_for("claude", Duration::from_secs(5));
    spec_p.role = Some(AgentRole::Planner);
    let input_p = serde_json::json!({
        "prompt": "draft plan",
        "crew": "mixed-fixture",
        "task_id": "T-crew"
    });
    let resolved_p = resolve_agent_settings(AgentRole::Planner, &host_p, &spec_p, &input_p);
    assert_eq!(resolved_p.model.as_deref(), Some(TEST_CLAUDE_MODEL));
    let mut spec_p_run = spec_p.clone();
    apply_resolved_settings(&mut spec_p_run, &resolved_p);
    let _ = run_cli_backend(
        &host_p,
        &spec_p_run,
        "job-crew-planner",
        audit_p.clone(),
        &input_p,
        None,
    )
    .expect("planner cli run");

    let events_p = audit_p.events_snapshot().expect("planner events");
    let argv_p = events_p
        .iter()
        .find_map(|e| match &e.kind {
            V2AuditEventKind::CliInvocationStarted {
                argv_redacted,
                provider,
                ..
            } => {
                assert_eq!(
                    provider, "claude",
                    "identity attribution must be claude family"
                );
                Some(argv_redacted.clone())
            }
            _ => None,
        })
        .expect("planner started event");
    let model_idx_p = argv_p
        .iter()
        .position(|a| a == "--model")
        .expect("planner argv has --model");
    assert_eq!(
        argv_p.get(model_idx_p + 1).map(String::as_str),
        Some(TEST_CLAUDE_MODEL),
        "planner --model must be exact {TEST_CLAUDE_MODEL}, not family"
    );

    // implementer leg via same crew
    let sink_for_writer_i: Arc<dyn AuditSink> = Arc::new(RecordingSink::default());
    let audit_i = Arc::new(V2AuditWriter::new(
        "job-crew-impl",
        format!("codex:{TEST_CODEX_MODEL}"),
        sink_for_writer_i,
    ));
    let host_i = TestHost::with_command(codex_script.display().to_string());
    let mut spec_i = test_agent_loop_spec_for("codex", Duration::from_secs(5));
    spec_i.role = Some(AgentRole::Implementer);
    let input_i = serde_json::json!({
        "prompt": "implement",
        "crew": "mixed-fixture",
        "task_id": "T-crew"
    });
    let resolved_i = resolve_agent_settings(AgentRole::Implementer, &host_i, &spec_i, &input_i);
    assert_eq!(resolved_i.model.as_deref(), Some(TEST_CODEX_MODEL));
    let mut spec_i_run = spec_i.clone();
    apply_resolved_settings(&mut spec_i_run, &resolved_i);
    let _ = run_cli_backend(
        &host_i,
        &spec_i_run,
        "job-crew-impl",
        audit_i.clone(),
        &input_i,
        None,
    )
    .expect("implementer cli run");

    let events_i = audit_i.events_snapshot().expect("impl events");
    let argv_i = events_i
        .iter()
        .find_map(|e| match &e.kind {
            V2AuditEventKind::CliInvocationStarted {
                argv_redacted,
                provider,
                ..
            } => {
                assert_eq!(
                    provider, "codex",
                    "identity attribution must be codex family"
                );
                Some(argv_redacted.clone())
            }
            _ => None,
        })
        .expect("impl started event");
    let model_idx_i = argv_i
        .iter()
        .position(|a| a == "--model")
        .expect("impl argv has --model");
    assert_eq!(
        argv_i.get(model_idx_i + 1).map(String::as_str),
        Some(TEST_CODEX_MODEL),
        "implementer --model must be exact {TEST_CODEX_MODEL}, not family"
    );
}

#[test]
fn run_cli_backend_redacts_token_shaped_argv_in_audit() {
    // [ORB-00417] A token-shaped provider-CLI flag value must be redacted in
    // the persisted run record / audit event, not recorded verbatim.
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null}\\n'\n",
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-argv-redaction",
        "codex:gpt-5.5",
        sink_for_writer,
    ));
    let audit_for_assert = Arc::clone(&audit);

    let mut host = TestHost::with_command(script.display().to_string());
    host.executor_args = vec![
        "--api-key".to_string(),
        "sk-secretargvtoken1234567890abcdef".to_string(),
    ];
    let spec = test_agent_loop_spec(Duration::from_secs(5));

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-argv-redaction",
        audit,
        &serde_json::json!({"prompt": "hi"}),
        None,
    )
    .expect("run succeeds");
    assert!(outcome.success);

    let events = audit_for_assert.events_snapshot().expect("audit snapshot");
    let argv = events
        .iter()
        .find_map(|event| match &event.kind {
            V2AuditEventKind::CliInvocationStarted { argv_redacted, .. } => {
                Some(argv_redacted.clone())
            }
            _ => None,
        })
        .expect("a CliInvocationStarted audit event should be present");
    let joined = argv.join(" ");
    assert!(
        !joined.contains("sk-secretargvtoken1234567890abcdef"),
        "argv leaked the token: {joined}"
    );
    assert!(
        joined.contains("[REDACTED"),
        "argv should carry a redaction placeholder: {joined}"
    );
}
