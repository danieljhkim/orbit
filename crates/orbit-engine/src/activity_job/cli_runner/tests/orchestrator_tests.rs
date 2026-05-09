use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use orbit_agent::loop_engine::audit::AuditSink;
use orbit_common::types::activity_job::V2AuditEventKind;
use tempfile::tempdir;

use super::super::super::audit_writer::V2AuditWriter;
use super::super::super::dispatcher::DispatchError;
use super::super::run_cli_backend;
use super::test_support::{RecordingSink, TestHost, test_agent_loop_spec, write_executable};

#[test]
fn run_cli_backend_finished_audit_event_keeps_stdout_stderr_blob_refs() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\nprintf '%s\\n' 'plain stdout'\nprintf '%s\\n' 'plain stderr' >&2\n",
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
    assert_eq!(outcome.output["stdout_text"], "plain stdout\n");
    assert_eq!(outcome.output["stdout_text_truncated"], false);
    assert_eq!(outcome.output["stdout_text_original_bytes"], 13);
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
    assert_eq!(sink.blob("blob-2"), Some(b"plain stdout\n".to_vec()));
    assert_eq!(sink.blob("blob-3"), Some(b"plain stderr\n".to_vec()));
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
    let spec = test_agent_loop_spec(Duration::from_secs(5));

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
printf '%s\n' 'Authorization: Bearer stdout-secret-token'
printf '%s\n' 'x-api-key: stdout-header-key'
printf '%s\n' 'sk-stdoutsecret123'
printf '%s\n' '{"api_key":"stdout-json-key"}'
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
fn run_cli_backend_returns_error_when_declared_workspace_path_missing() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"status\":\"ok\"}'\n",
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
        "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"status\":\"ok\"}'\n",
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
        "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"status\":\"ok\"}'\n",
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

/// Regression for T20260508-17: a CLI subprocess that exits 0 but emits an
/// embedded Orbit response envelope reporting `status: "failed"` must NOT
/// be classified as success. Pre-fix, dispatch returned `success: true`
/// because the dispatcher only consulted exit code, leaving the planning
/// pipeline to push an empty branch and open an empty PR.
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
    let spec = test_agent_loop_spec(Duration::from_secs(5));

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
