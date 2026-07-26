#![allow(missing_docs)]

use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use orbit_agent::loop_engine::audit::AuditSink;
use orbit_common::test_fixtures::{TEST_CLAUDE_MODEL, TEST_CODEX_MODEL};
use orbit_common::types::activity_job::{
    AgentRole, JobV2StepBody, V2AuditEventKind, load_job_asset,
};
use orbit_store::Store;
use tempfile::{TempDir, tempdir};

use crate::context::{ProvenanceEnv, provenance_env};
use crate::template::{self, TemplateContext};

use super::super::super::agent_role::{apply_resolved_settings, resolve_agent_settings};
use super::super::super::audit_writer::V2AuditWriter;
use super::super::super::dispatcher::DispatchError;
use super::super::super::sqlite_sink::V2SqliteSink;
use super::super::super::workspace::{WorktreeBoundaryGuard, validate_declared_worktree_pair};
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

/// [ORB-10449] The regression this task exists for: an agent that exits 0 with
/// prose and no envelope did not finish its turn, and must not checkpoint as
/// success just because the activity never opted into the *content* contract.
#[test]
fn run_cli_backend_fails_artifact_activity_when_exit_zero_carries_no_envelope() {
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

    assert!(!outcome.success);
    assert_eq!(outcome.output["exit_code"], 0);
    // The content contract is still opt-in and still off here — the step failed
    // on the completion protocol alone.
    assert_eq!(outcome.output["response_envelope_required"], false);
    assert_eq!(outcome.output["completion_envelope_required"], true);
    assert_eq!(outcome.output["completion_envelope_satisfied"], false);
    let message = outcome.message.expect("completion protocol message");
    assert!(message.contains("agent step did not complete"), "{message}");
    assert!(
        message.contains("does not contain an Orbit response envelope"),
        "{message}"
    );
}

/// The declared exception (`dispatch_agent`): an activity whose work is
/// decorative keeps the pre-ORB-10449 behaviour, and the invalid envelope is
/// still recorded as a diagnostic rather than acted on.
#[test]
fn run_cli_backend_keeps_advisory_activity_successful_without_an_envelope() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("claude");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' 'advisory grouping notes, no envelope'\n",
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-advisory-response",
        "claude:sonnet",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let mut spec = test_agent_loop_spec_for("claude", Duration::from_secs(5));
    spec.require_completion_envelope = false;

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-advisory-response",
        audit,
        &serde_json::json!({"prompt": "group the backlog"}),
        None,
    )
    .expect("run cli backend");

    assert!(outcome.success);
    assert!(outcome.message.is_none());
    assert_eq!(outcome.output["completion_envelope_required"], false);
    assert_eq!(outcome.output["completion_envelope_satisfied"], false);
    assert!(
        outcome.output["completion_envelope_error"]
            .as_str()
            .is_some_and(|message| message.contains("agent step did not complete"))
    );
}

/// The completion check is content-blind. An agent that declares
/// `status: "failed"` ran its contract to the end, so it satisfies the protocol
/// gate; whether that declared failure means anything is a question for durable
/// state, not for this step's success. Guards the doctrine boundary: no
/// activity decision may read an agent-loop response's content.
#[test]
fn run_cli_backend_completion_check_accepts_a_declared_failure_envelope() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{\"schemaVersion\":1,\"status\":\"failed\",\"result\":{},\"error\":{\"code\":\"blocked\",\"message\":\"cannot proceed\"}}'\n",
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-declared-failure",
        "codex:gpt-5.5",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let spec = test_agent_loop_spec(Duration::from_secs(5));

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-declared-failure",
        audit,
        &serde_json::json!({"task_id": "ORB-10449"}),
        None,
    )
    .expect("run cli backend");

    assert!(outcome.success, "content must not decide step completion");
    assert_eq!(outcome.output["completion_envelope_required"], true);
    assert_eq!(outcome.output["completion_envelope_satisfied"], true);
    assert!(outcome.output["completion_envelope_error"].is_null());
}

/// A provider that interleaves a wrapped tool's stdout with its own protocol
/// output still terminated properly. The completion gate must key on the
/// termination signal, not on the tidiness of the stream around it — a false
/// positive here would fail completed work.
#[test]
fn run_cli_backend_completion_check_tolerates_interleaved_non_json_stdout() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '[main abc1234] some commit'\nprintf '%s\\n' '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null}'\n",
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-interleaved-stdout",
        "codex:gpt-5.5",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    let spec = test_agent_loop_spec(Duration::from_secs(5));

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-interleaved-stdout",
        audit,
        &serde_json::json!({"task_id": "ORB-10449"}),
        None,
    )
    .expect("run cli backend");

    assert!(outcome.success);
    assert_eq!(outcome.output["completion_envelope_satisfied"], true);
}

/// [ORB-10449] `jrun-20260726-1758-5` replayed as a fixture: claude exits 0
/// with `stop_reason: end_turn` after parking itself on a background process,
/// having emitted no envelope. Before this change the step checkpointed as
/// success and the run failed three steps later at the delivery gate.
#[test]
fn run_cli_backend_fails_on_the_jrun_20260726_1758_5_stall_shape() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("claude");
    let stdout = serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "stop_reason": "end_turn",
        "result": "The nextest run is still executing in the background (no failures through \
                   1782/2693 tests so far). I'll wait for the scheduled wakeup or task \
                   notification before analyzing results and continuing the ORB-10436 audit."
    })
    .to_string();
    // The captured prose contains an apostrophe, so route it through a file
    // rather than a single-quoted shell literal.
    let stdout_file = temp.path().join("stdout.json");
    fs::write(&stdout_file, &stdout).expect("write stall stdout fixture");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\ncat > /dev/null\ncat '{}'\n",
            stdout_file.display()
        ),
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "jrun-20260726-1758-5",
        "claude:sonnet",
        sink_for_writer,
    ));
    let host = TestHost::with_command(script.display().to_string());
    // `agent_implement`'s shipped shape: artifact-backed, so the content
    // contract is off. Only the completion protocol stands between a stalled
    // implementer and a green checkpoint.
    let spec = test_agent_loop_spec_for("claude", Duration::from_secs(5));
    assert!(!spec.require_response_envelope);

    let outcome = run_cli_backend(
        &host,
        &spec,
        "jrun-20260726-1758-5",
        audit,
        &serde_json::json!({"task_id": "ORB-10436"}),
        None,
    )
    .expect("run cli backend");

    assert!(
        !outcome.success,
        "a stalled implementer must not checkpoint"
    );
    assert_eq!(outcome.output["exit_code"], 0);
    assert_eq!(outcome.output["timed_out"], false);
    let message = outcome.message.expect("stall message");
    assert!(message.contains("agent step did not complete"), "{message}");
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
        workspace_root: None,
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
        workspace_root: None,
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

const TASK_LOCAL_PIPELINE_YAML: &str =
    include_str!("../../../../../../.orbit/resources/jobs/task_local_pipeline.yaml");
const TASK_PR_PIPELINE_YAML: &str =
    include_str!("../../../../../../.orbit/resources/jobs/task_pr_pipeline.yaml");

#[test]
fn actual_ship_pipeline_implementers_run_for_each_provider_and_stay_in_the_worktree() {
    let mut rendered_assets = BTreeSet::new();
    for pipeline_yaml in [TASK_LOCAL_PIPELINE_YAML, TASK_PR_PIPELINE_YAML] {
        for provider in ["claude", "codex"] {
            let fixture = linked_worktree_fixture();
            let script = fixture.root().join(provider);
            let assigned = fixture.assigned.display().to_string();
            let task_id = format!("ORB-ASSET-{provider}");
            let (pipeline, input) =
                rendered_implement_input_from_asset(pipeline_yaml, &fixture.assigned, &task_id);
            rendered_assets.insert(pipeline.clone());
            write_executable(
                &script,
                &format!(
                    r#"#!/bin/sh
cat > /dev/null
test "$(pwd -P)" = '{assigned}' || exit 41
test "$(git rev-parse --show-toplevel)" = '{assigned}' || exit 42
printf '%s\n' '{pipeline}:{provider}' > observed-relative-write.txt
printf '%s\n' '{{"schemaVersion":1,"status":"success","result":{{}},"error":null}}'
"#
                ),
            );
            let mut host = TestHost::with_command(script.display().to_string());
            host.workspace_root = Some(fixture.primary.clone());
            let spec = test_agent_loop_spec_for(provider, Duration::from_secs(5));
            let audit = test_audit(&format!("run-{pipeline}-{provider}"), provider);

            let outcome = run_cli_backend(
                &host,
                &spec,
                &format!("run-{pipeline}-{provider}"),
                audit,
                &input,
                None,
            )
            .unwrap_or_else(|error| panic!("{pipeline}/{provider} invocation: {error}"));

            assert!(outcome.success, "{pipeline}/{provider} should succeed");
            assert_eq!(
                fs::read_to_string(fixture.assigned.join("observed-relative-write.txt"))
                    .expect("assigned relative write"),
                format!("{pipeline}:{provider}\n")
            );
            assert!(
                !fixture.primary.join("observed-relative-write.txt").exists(),
                "{pipeline}/{provider} must not write the registered primary checkout"
            );
        }
    }
    assert_eq!(
        rendered_assets,
        BTreeSet::from([
            "task_local_pipeline".to_string(),
            "task_pr_pipeline".to_string(),
        ]),
        "the matrix must load and render both committed shipment assets"
    );
}

#[test]
fn declared_repo_root_mismatch_fails_typed_before_provider_spawn() {
    let fixture = linked_worktree_fixture();
    let marker = fixture.root().join("provider-started");
    let script = fixture.root().join("codex");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\nprintf started > '{}'\nexit 0\n",
            marker.display()
        ),
    );
    let mut host = TestHost::with_command(script.display().to_string());
    host.workspace_root = Some(fixture.primary.clone());
    let audit = test_audit("run-repo-root-mismatch", "codex");
    let mut input = worktree_input(&fixture, "ORB-PAIR-MISMATCH");
    input["repo_root"] = serde_json::json!(fixture.primary);

    let error = run_cli_backend(
        &host,
        &test_agent_loop_spec(Duration::from_secs(5)),
        "run-repo-root-mismatch",
        audit.clone(),
        &input,
        None,
    )
    .expect_err("mismatched declared pair must fail closed");

    assert_worktree_mismatch(
        &error,
        "ORB-PAIR-MISMATCH",
        "run-repo-root-mismatch",
        "different Git checkouts",
    );
    assert_pre_spawn_failure(&audit, &marker);

    input["repo_root"] = serde_json::Value::Null;
    let null_error = run_cli_backend(
        &host,
        &test_agent_loop_spec(Duration::from_secs(5)),
        "run-null-repo-root",
        audit.clone(),
        &input,
        None,
    )
    .expect_err("a declared null repo_root must not erase the worktree pair");
    assert_worktree_mismatch(
        &null_error,
        "ORB-PAIR-MISMATCH",
        "run-null-repo-root",
        "repo_root must be a non-empty string",
    );
    assert_pre_spawn_failure(&audit, &marker);
}

#[test]
fn declared_non_git_checkout_fails_typed_before_provider_spawn() {
    let fixture = linked_worktree_fixture();
    let non_git = fixture.root().join("not-a-repository");
    fs::create_dir(&non_git).expect("create non-Git directory");
    let marker = fixture.root().join("provider-started");
    let script = fixture.root().join("codex");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\nprintf started > '{}'\nexit 0\n",
            marker.display()
        ),
    );
    let mut host = TestHost::with_command(script.display().to_string());
    host.workspace_root = Some(fixture.primary.clone());
    let audit = test_audit("run-non-git-pair", "codex");
    let input = serde_json::json!({
        "task_id": "ORB-NON-GIT",
        "workspace_path": non_git,
        "repo_root": non_git,
    });

    let error = run_cli_backend(
        &host,
        &test_agent_loop_spec(Duration::from_secs(5)),
        "run-non-git-pair",
        audit.clone(),
        &input,
        None,
    )
    .expect_err("non-Git declared pair must fail closed");

    assert_worktree_mismatch(
        &error,
        "ORB-NON-GIT",
        "run-non-git-pair",
        "not a Git checkout",
    );
    assert_pre_spawn_failure(&audit, &marker);
}

#[test]
fn declared_checkout_from_different_repository_fails_before_provider_spawn() {
    let assigned_fixture = linked_worktree_fixture();
    let registered_fixture = linked_worktree_fixture();
    let marker = assigned_fixture.root().join("provider-started");
    let script = assigned_fixture.root().join("codex");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\nprintf started > '{}'\nexit 0\n",
            marker.display()
        ),
    );
    let mut host = TestHost::with_command(script.display().to_string());
    host.workspace_root = Some(registered_fixture.primary.clone());
    let audit = test_audit("run-different-common-dir", "codex");

    let error = run_cli_backend(
        &host,
        &test_agent_loop_spec(Duration::from_secs(5)),
        "run-different-common-dir",
        audit.clone(),
        &worktree_input(&assigned_fixture, "ORB-DIFFERENT-REPO"),
        None,
    )
    .expect_err("different repositories must fail closed");

    assert_worktree_mismatch(
        &error,
        "ORB-DIFFERENT-REPO",
        "run-different-common-dir",
        "different Git common dirs",
    );
    assert_pre_spawn_failure(&audit, &marker);
}

#[test]
fn declared_checkout_cannot_collapse_to_registered_primary() {
    let fixture = linked_worktree_fixture();
    let marker = fixture.root().join("provider-started");
    let script = fixture.root().join("codex");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\nprintf started > '{}'\nexit 0\n",
            marker.display()
        ),
    );
    let mut host = TestHost::with_command(script.display().to_string());
    host.workspace_root = Some(fixture.primary.clone());
    let audit = test_audit("run-primary-collapse", "codex");
    let input = serde_json::json!({
        "task_id": "ORB-PRIMARY-COLLAPSE",
        "workspace_path": fixture.primary,
        "repo_root": fixture.primary,
    });

    let error = run_cli_backend(
        &host,
        &test_agent_loop_spec(Duration::from_secs(5)),
        "run-primary-collapse",
        audit.clone(),
        &input,
        None,
    )
    .expect_err("assigned checkout must not collapse to primary");

    assert_worktree_mismatch(
        &error,
        "ORB-PRIMARY-COLLAPSE",
        "run-primary-collapse",
        "collapses to the registered primary",
    );
    assert_pre_spawn_failure(&audit, &marker);
}

#[test]
fn unchanged_pre_dirty_primary_does_not_block_valid_worktree_implementation() {
    let fixture = linked_worktree_fixture();
    fs::write(
        fixture.primary.join("README.md"),
        "pre-existing primary dirtiness\n",
    )
    .expect("dirty primary");
    let primary_before = git_bytes(&fixture.primary, &["diff", "--binary", "HEAD", "--"]);
    let script = fixture.root().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf 'assigned only\\n' > assigned.txt\nprintf '%s\\n' '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null}'\n",
    );
    let mut host = TestHost::with_command(script.display().to_string());
    host.workspace_root = Some(fixture.primary.clone());

    let outcome = run_cli_backend(
        &host,
        &test_agent_loop_spec(Duration::from_secs(5)),
        "run-pre-dirty-primary",
        test_audit("run-pre-dirty-primary", "codex"),
        &worktree_input(&fixture, "ORB-PRE-DIRTY"),
        None,
    )
    .expect("unchanged dirty primary is allowed");

    assert!(outcome.success);
    assert!(fixture.assigned.join("assigned.txt").exists());
    assert_eq!(
        git_bytes(&fixture.primary, &["diff", "--binary", "HEAD", "--"]),
        primary_before,
        "the guard must leave pre-existing primary dirtiness byte-for-byte unchanged"
    );
}

#[test]
fn concurrent_primary_fast_forward_does_not_block_disjoint_worktree_changes() {
    let fixture = linked_worktree_fixture();
    let script = fixture.root().join("codex");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\ncat > /dev/null\nprintf 'assigned\\n' > assigned.txt\nprintf 'concurrent\\n' > '{}/concurrent.txt'\ngit -C '{}' add -- concurrent.txt\ngit -C '{}' commit -m concurrent-base\nprintf '%s\\n' '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null}}'\n",
            fixture.primary.display(),
            fixture.primary.display(),
            fixture.primary.display(),
        ),
    );
    let mut host = TestHost::with_command(script.display().to_string());
    host.workspace_root = Some(fixture.primary.clone());

    let outcome = run_cli_backend(
        &host,
        &test_agent_loop_spec(Duration::from_secs(5)),
        "run-concurrent-fast-forward",
        test_audit("run-concurrent-fast-forward", "codex"),
        &worktree_input(&fixture, "ORB-CONCURRENT-FF"),
        None,
    )
    .expect("a disjoint same-branch primary fast-forward is benign");

    assert!(outcome.success);
    assert!(fixture.assigned.join("assigned.txt").exists());
    assert!(fixture.primary.join("concurrent.txt").exists());
}

#[test]
fn two_in_flight_worktrees_survive_one_primary_merge_advance() {
    let fixture = linked_worktree_fixture();
    let assigned_two = fixture.root().join("assigned-two");
    git_ok(
        &fixture.primary,
        &[
            "worktree",
            "add",
            "-b",
            "orbit-integrity-test-two",
            assigned_two.to_str().expect("utf8 second worktree"),
        ],
    );
    let assigned_two = assigned_two
        .canonicalize()
        .expect("canonical second worktree");
    let input_one = worktree_input(&fixture, "ORB-CONCURRENT-ONE");
    let input_two = serde_json::json!({
        "prompt": "implement",
        "task_id": "ORB-CONCURRENT-TWO",
        "workspace_path": assigned_two,
        "repo_root": assigned_two,
    });
    let pair_one = validate_declared_worktree_pair(
        &input_one,
        None,
        "run-concurrent-one",
        "codex",
        Some(&fixture.primary),
    )
    .expect("validate first pair")
    .expect("first linked pair");
    let pair_two = validate_declared_worktree_pair(
        &input_two,
        None,
        "run-concurrent-two",
        "codex",
        Some(&fixture.primary),
    )
    .expect("validate second pair")
    .expect("second linked pair");
    let guard_one = WorktreeBoundaryGuard::capture(
        &input_one,
        None,
        "run-concurrent-one",
        "codex",
        Some(&fixture.assigned),
        Some(&fixture.primary),
        Some(&pair_one),
    )
    .expect("capture first guard")
    .expect("first guard enabled");
    let guard_two = WorktreeBoundaryGuard::capture(
        &input_two,
        None,
        "run-concurrent-two",
        "codex",
        Some(&assigned_two),
        Some(&fixture.primary),
        Some(&pair_two),
    )
    .expect("capture second guard")
    .expect("second guard enabled");

    fs::write(fixture.assigned.join("candidate-one.txt"), "candidate\n")
        .expect("write first candidate");
    fs::write(assigned_two.join("candidate-two.txt"), "candidate\n")
        .expect("write second candidate");
    fs::write(fixture.primary.join("merged-pr.txt"), "merged\n").expect("write merged PR");
    git_ok(&fixture.primary, &["add", "merged-pr.txt"]);
    git_ok(&fixture.primary, &["commit", "-m", "merge concurrent PR"]);

    guard_one
        .verify()
        .expect("first in-flight pipeline accepts the shared base advance");
    guard_two
        .verify()
        .expect("second in-flight pipeline accepts the shared base advance");

    assert!(fixture.assigned.join("candidate-one.txt").exists());
    assert!(assigned_two.join("candidate-two.txt").exists());
}

#[test]
fn concurrent_auto_task_refresh_stays_in_assigned_worktree_during_boundary_checks() {
    let fixture = linked_worktree_fixture();
    let definition_dir = fixture.assigned.join(".orbit/auto_tasks");
    fs::create_dir_all(&definition_dir).expect("definition dir");
    let definition_path = definition_dir.join("doc-duties.yaml");
    fs::write(
        &definition_path,
        "schemaVersion: 1\nname: doc-duties\nrevision: 0\n",
    )
    .expect("seed definition");
    git_ok(
        &fixture.assigned,
        &["add", ".orbit/auto_tasks/doc-duties.yaml"],
    );
    git_ok(
        &fixture.assigned,
        &["commit", "-m", "seed auto-task definition"],
    );
    git_ok(
        &fixture.primary,
        &["merge", "--ff-only", "orbit-integrity-test"],
    );

    let implement_worktree = fixture.root().join("disjoint-implement");
    git_ok(
        &fixture.primary,
        &[
            "worktree",
            "add",
            "-b",
            "orbit-disjoint-implement",
            implement_worktree
                .to_str()
                .expect("utf8 implement worktree"),
        ],
    );
    let implement_worktree = implement_worktree
        .canonicalize()
        .expect("canonical implement worktree");
    let input_one = worktree_input(&fixture, "ORB-AUTO-TASK-REFRESH");
    let input_two = serde_json::json!({
        "prompt": "implement disjoint work",
        "task_id": "ORB-DISJOINT-IMPLEMENT",
        "workspace_path": implement_worktree,
        "repo_root": implement_worktree,
    });
    let pair_one = validate_declared_worktree_pair(
        &input_one,
        None,
        "run-auto-task-refresh",
        "codex",
        Some(&fixture.primary),
    )
    .expect("validate refresh pair")
    .expect("refresh linked pair");
    let pair_two = validate_declared_worktree_pair(
        &input_two,
        None,
        "run-disjoint-implement",
        "claude",
        Some(&fixture.primary),
    )
    .expect("validate implement pair")
    .expect("implement linked pair");
    let refresh_guard = WorktreeBoundaryGuard::capture(
        &input_one,
        None,
        "run-auto-task-refresh",
        "codex",
        Some(&fixture.assigned),
        Some(&fixture.primary),
        Some(&pair_one),
    )
    .expect("capture refresh guard")
    .expect("refresh guard enabled");
    let implement_guard = WorktreeBoundaryGuard::capture(
        &input_two,
        None,
        "run-disjoint-implement",
        "claude",
        Some(&implement_worktree),
        Some(&fixture.primary),
        Some(&pair_two),
    )
    .expect("capture implement guard")
    .expect("implement guard enabled");
    let primary_before = git_bytes(&fixture.primary, &["status", "--porcelain=v2", "-z"]);

    let (staged_tx, staged_rx) = std::sync::mpsc::channel();
    let (continue_tx, continue_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        for revision in 1..=32 {
            let staged = definition_dir.join(format!(".doc-duties.{revision}.tmp"));
            fs::write(
                &staged,
                format!("schemaVersion: 1\nname: doc-duties\nrevision: {revision}\n"),
            )
            .expect("stage definition");
            if revision == 1 {
                staged_tx.send(()).expect("signal staged refresh");
                continue_rx.recv().expect("continue refresh");
            }
            fs::rename(&staged, &definition_path).expect("atomically refresh definition");
        }
    });

    staged_rx.recv().expect("refresh reached staging point");
    implement_guard
        .verify()
        .expect("disjoint boundary snapshot must not report primary checkout drift");
    continue_tx.send(()).expect("release refresh");
    writer.join().expect("refresh thread");
    refresh_guard
        .verify()
        .expect("refresh boundary must remain inside its assigned worktree");

    assert_eq!(
        git_bytes(&fixture.primary, &["status", "--porcelain=v2", "-z"]),
        primary_before,
        "concurrent refresh must leave registered primary byte-identical"
    );
    assert!(
        fixture
            .assigned
            .join(".orbit/auto_tasks/doc-duties.yaml")
            .is_file()
    );
}

#[test]
fn failed_auto_task_refresh_preserves_primary_and_audits_definition_and_run() {
    let fixture = linked_worktree_fixture();
    let primary_before = git_bytes(&fixture.primary, &["status", "--porcelain=v2", "-z"]);
    let script = fixture.root().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nmkdir -p .orbit/auto_tasks\nprintf 'partial\\n' > .orbit/auto_tasks/.doc-duties.tmp\nrm .orbit/auto_tasks/.doc-duties.tmp\nprintf 'auto-task doc-duties refresh failed\\n' >&2\nexit 23\n",
    );
    let mut host = TestHost::with_command(script.display().to_string());
    host.workspace_root = Some(fixture.primary.clone());
    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink.clone();
    let audit = Arc::new(V2AuditWriter::new(
        "run-auto-task-refresh-failed",
        "codex:gpt-5.5",
        sink_for_writer,
    ));

    let outcome = run_cli_backend(
        &host,
        &test_agent_loop_spec(Duration::from_secs(5)),
        "run-auto-task-refresh-failed",
        audit.clone(),
        &worktree_input(&fixture, "ORB-AUTO-TASK-REFRESH"),
        None,
    )
    .expect("failed provider remains an audited dispatch outcome");

    assert!(!outcome.success);
    assert_eq!(
        git_bytes(&fixture.primary, &["status", "--porcelain=v2", "-z"]),
        primary_before,
        "failed refresh must leave registered primary byte-identical"
    );
    let events = audit.events_snapshot().expect("audit events");
    let failed = events
        .iter()
        .find_map(|event| match &event.kind {
            V2AuditEventKind::CliInvocationFinished {
                exit_code,
                stderr_blob_ref,
                ..
            } if *exit_code == Some(23) => Some((
                event.envelope.run_id.as_str(),
                stderr_blob_ref.as_deref().expect("stderr blob"),
            )),
            _ => None,
        })
        .expect("durable failed-refresh event");
    assert_eq!(failed.0, "run-auto-task-refresh-failed");
    assert_eq!(
        sink.blob(failed.1).as_deref(),
        Some(b"auto-task doc-duties refresh failed\n".as_slice()),
        "failure evidence must identify the auto-task"
    );
}

#[test]
fn unchanged_pre_dirty_path_is_excluded_from_escape_diagnostic() {
    let fixture = linked_worktree_fixture();
    fs::write(
        fixture.primary.join("README.md"),
        "pre-existing primary dirtiness\n",
    )
    .expect("dirty primary before capture");
    let escaped = fixture.primary.join("escaped-after-capture.txt");
    let script = fixture.root().join("codex");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\ncat > /dev/null\nprintf 'escaped\\n' > '{}'\nprintf '%s\\n' '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null}}'\n",
            escaped.display()
        ),
    );
    let mut host = TestHost::with_command(script.display().to_string());
    host.workspace_root = Some(fixture.primary.clone());

    let error = run_cli_backend(
        &host,
        &test_agent_loop_spec(Duration::from_secs(5)),
        "run-predirty-attribution",
        test_audit("run-predirty-attribution", "codex"),
        &worktree_input(&fixture, "ORB-PREDIRTY-ATTRIBUTION"),
        None,
    )
    .expect_err("new primary delta must fail closed");

    let diagnostic = worktree_integrity_diagnostic(&error);
    assert_eq!(
        diagnostic["changed_paths"],
        serde_json::json!(["escaped-after-capture.txt"]),
        "unchanged pre-existing dirtiness must not be attributed to this invocation"
    );
    assert_eq!(
        diagnostic["primary_before"]["path_states"]["README.md"],
        diagnostic["primary_after"]["path_states"]["README.md"],
        "the diagnostic must retain identical before/after identity for the pre-dirty path"
    );
    assert!(
        diagnostic["primary_after"]["path_states"]["escaped-after-capture.txt"]
            ["untracked_content_sha256"]
            .is_string(),
        "the new untracked path needs its own content identity"
    );
}

#[test]
fn staged_only_primary_delta_reports_its_path_and_index_identity() {
    let fixture = linked_worktree_fixture();
    let script = fixture.root().join("codex");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\ncat > /dev/null\nprintf 'staged-only\\n' > '{}/README.md'\ngit -C '{}' add -- README.md\nprintf '%s\\n' '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null}}'\n",
            fixture.primary.display(),
            fixture.primary.display()
        ),
    );
    let mut host = TestHost::with_command(script.display().to_string());
    host.workspace_root = Some(fixture.primary.clone());

    let error = run_cli_backend(
        &host,
        &test_agent_loop_spec(Duration::from_secs(5)),
        "run-staged-only-attribution",
        test_audit("run-staged-only-attribution", "codex"),
        &worktree_input(&fixture, "ORB-STAGED-ONLY"),
        None,
    )
    .expect_err("staged primary delta must fail closed");

    let diagnostic = worktree_integrity_diagnostic(&error);
    assert_eq!(
        diagnostic["changed_paths"],
        serde_json::json!(["README.md"])
    );
    let after = &diagnostic["primary_after"]["path_states"]["README.md"];
    assert!(after["index_entry_sha256"].is_string());
    assert!(after["staged_patch_sha256"].is_string());
    assert!(
        after["worktree_patch_sha256"].is_null(),
        "a fully staged edit has no index-to-worktree patch"
    );
    assert_eq!(after["worktree_present"], true);
    assert!(after["untracked_content_sha256"].is_null());
}

#[test]
fn primary_escape_is_typed_non_retryable_and_preserves_both_checkouts() {
    let fixture = linked_worktree_fixture();
    let escaped = fixture.primary.join("escaped.txt");
    let script = fixture.root().join("claude");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\ncat > /dev/null\nprintf 'escaped\\n' > '{}'\ngit -C '{}' add -- escaped.txt\nprintf '%s\\n' '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null}}'\n",
            escaped.display(),
            fixture.primary.display()
        ),
    );
    let mut host = TestHost::with_command(script.display().to_string());
    host.workspace_root = Some(fixture.primary.clone());
    let audit = test_audit("run-deliberate-escape", "claude");

    let error = run_cli_backend(
        &host,
        &test_agent_loop_spec_for("claude", Duration::from_secs(5)),
        "run-deliberate-escape",
        audit.clone(),
        &worktree_input(&fixture, "ORB-ESCAPE"),
        None,
    )
    .expect_err("primary write must fail closed");

    assert_worktree_integrity_error(
        &error,
        "primary_checkout_drift",
        ("ORB-ESCAPE", "run-deliberate-escape", "claude"),
        &fixture,
        "escaped.txt",
    );
    assert!(error.is_non_retryable());
    assert!(
        escaped.exists(),
        "diagnosis must not clean the primary delta"
    );
    assert!(
        !fixture.assigned.join("escaped.txt").exists(),
        "diagnosis must not copy the primary delta"
    );
    assert_eq!(
        String::from_utf8(git_bytes(
            &fixture.primary,
            &["diff", "--cached", "--name-only", "HEAD", "--"]
        ))
        .expect("utf8 staged paths")
        .trim(),
        "escaped.txt",
        "diagnosis must not reset the provider-mutated primary index"
    );
    assert!(
        audit
            .events_snapshot()
            .expect("audit events")
            .iter()
            .any(|event| matches!(event.kind, V2AuditEventKind::CliInvocationFinished { .. })),
        "terminal provider audit must precede the integrity failure"
    );
}

#[test]
fn primary_content_mutation_is_typed_even_when_assigned_content_also_changes() {
    let fixture = linked_worktree_fixture();
    let escaped = fixture.primary.join("ambiguous-primary.txt");
    let script = fixture.root().join("codex");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\ncat > /dev/null\nprintf 'assigned\\n' > ambiguous-assigned.txt\nprintf 'primary\\n' > '{}'\nprintf '%s\\n' '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null}}'\n",
            escaped.display()
        ),
    );
    let mut host = TestHost::with_command(script.display().to_string());
    host.workspace_root = Some(fixture.primary.clone());

    let error = run_cli_backend(
        &host,
        &test_agent_loop_spec(Duration::from_secs(5)),
        "run-ambiguous-integrity",
        test_audit("run-ambiguous-integrity", "codex"),
        &worktree_input(&fixture, "ORB-AMBIGUOUS"),
        None,
    )
    .expect_err("dual-checkout mutation must fail closed");

    assert_worktree_integrity_error(
        &error,
        "primary_checkout_drift",
        ("ORB-AMBIGUOUS", "run-ambiguous-integrity", "codex"),
        &fixture,
        "ambiguous-primary.txt",
    );
    assert!(fixture.assigned.join("ambiguous-assigned.txt").exists());
    assert!(escaped.exists());
}

#[test]
fn assigned_history_divergence_is_a_typed_worktree_content_conflict() {
    let fixture = linked_worktree_fixture();
    let script = fixture.root().join("codex");
    write_executable(
        &script,
        "#!/bin/sh\ncat > /dev/null\nprintf 'committed by provider\\n' > assigned-commit.txt\ngit add -- assigned-commit.txt\ngit commit -m assigned-history-change\nprintf '%s\\n' '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null}'\n",
    );
    let mut host = TestHost::with_command(script.display().to_string());
    host.workspace_root = Some(fixture.primary.clone());

    let error = run_cli_backend(
        &host,
        &test_agent_loop_spec(Duration::from_secs(5)),
        "run-assigned-history-change",
        test_audit("run-assigned-history-change", "codex"),
        &worktree_input(&fixture, "ORB-ASSIGNED-HISTORY"),
        None,
    )
    .expect_err("provider-created worktree history must fail closed");

    assert_worktree_integrity_error(
        &error,
        "worktree_content_conflict",
        (
            "ORB-ASSIGNED-HISTORY",
            "run-assigned-history-change",
            "codex",
        ),
        &fixture,
        "assigned-commit.txt",
    );
    let diagnostic = worktree_integrity_diagnostic(&error);
    assert_eq!(
        diagnostic["changed_paths"],
        serde_json::json!(["assigned-commit.txt"]),
        "worktree conflicts report the run's paths, not primary checkout paths"
    );
}

#[test]
fn non_fast_forward_primary_move_remains_a_typed_drift_failure() {
    let fixture = linked_worktree_fixture();
    fs::write(fixture.primary.join("before-reset.txt"), "before reset\n")
        .expect("write primary commit");
    git_ok(&fixture.primary, &["add", "before-reset.txt"]);
    git_ok(&fixture.primary, &["commit", "-m", "primary before reset"]);
    let script = fixture.root().join("codex");
    write_executable(
        &script,
        &format!(
            "#!/bin/sh\ncat > /dev/null\nprintf 'assigned survives\\n' > assigned-survives.txt\ngit -C '{}' reset --hard HEAD^\nprintf '%s\\n' '{{\"schemaVersion\":1,\"status\":\"success\",\"result\":{{}},\"error\":null}}'\n",
            fixture.primary.display(),
        ),
    );
    let mut host = TestHost::with_command(script.display().to_string());
    host.workspace_root = Some(fixture.primary.clone());

    let error = run_cli_backend(
        &host,
        &test_agent_loop_spec(Duration::from_secs(5)),
        "run-primary-reset",
        test_audit("run-primary-reset", "codex"),
        &worktree_input(&fixture, "ORB-PRIMARY-RESET"),
        None,
    )
    .expect_err("a primary reset must remain fail closed");

    assert_worktree_integrity_error(
        &error,
        "primary_checkout_drift",
        ("ORB-PRIMARY-RESET", "run-primary-reset", "codex"),
        &fixture,
        "before-reset.txt",
    );
    assert!(
        fixture.assigned.join("assigned-survives.txt").exists(),
        "the guard diagnoses without discarding the run candidate"
    );
}

#[test]
fn primary_escape_is_checked_after_nonzero_exit_and_timeout() {
    for (terminal, trailer, timeout) in [
        ("nonzero", "exit 23", Duration::from_secs(5)),
        ("timeout", "sleep 10", Duration::from_secs(2)),
    ] {
        let fixture = linked_worktree_fixture();
        let escaped_name = format!("{terminal}-primary.txt");
        let escaped = fixture.primary.join(&escaped_name);
        let script = fixture.root().join("codex");
        write_executable(
            &script,
            &format!(
                "#!/bin/sh\ncat > /dev/null\nprintf '{terminal}\\n' > '{}'\n{trailer}\n",
                escaped.display()
            ),
        );
        let mut host = TestHost::with_command(script.display().to_string());
        host.workspace_root = Some(fixture.primary.clone());
        let run_id = format!("run-{terminal}-escape");
        let task_id = format!("ORB-{}", terminal.to_ascii_uppercase());

        let error = run_cli_backend(
            &host,
            &test_agent_loop_spec(timeout),
            &run_id,
            test_audit(&run_id, "codex"),
            &worktree_input(&fixture, &task_id),
            None,
        )
        .unwrap_err();

        assert_worktree_integrity_error(
            &error,
            "primary_checkout_drift",
            (&task_id, &run_id, "codex"),
            &fixture,
            &escaped_name,
        );
        assert!(escaped.exists(), "{terminal} delta must remain for rescue");
    }
}

struct LinkedWorktreeFixture {
    temp: TempDir,
    primary: PathBuf,
    assigned: PathBuf,
}

impl LinkedWorktreeFixture {
    fn root(&self) -> &Path {
        self.temp.path()
    }
}

fn linked_worktree_fixture() -> LinkedWorktreeFixture {
    let temp = tempdir().expect("fixture tempdir");
    let primary = temp.path().join("primary");
    let assigned = temp.path().join("assigned");
    fs::create_dir_all(&primary).expect("create primary");
    git_ok(&primary, &["init"]);
    git_ok(&primary, &["config", "user.name", "Orbit Test"]);
    git_ok(
        &primary,
        &["config", "user.email", "orbit-test@example.invalid"],
    );
    fs::write(primary.join("README.md"), "base\n").expect("write initial file");
    git_ok(&primary, &["add", "README.md"]);
    git_ok(&primary, &["commit", "-m", "initial"]);
    git_ok(
        &primary,
        &[
            "worktree",
            "add",
            "-b",
            "orbit-integrity-test",
            assigned.to_str().expect("utf8 assigned path"),
        ],
    );

    LinkedWorktreeFixture {
        primary: primary.canonicalize().expect("canonical primary"),
        assigned: assigned.canonicalize().expect("canonical assigned"),
        temp,
    }
}

fn git_ok(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {} in {} failed: {}",
        args.join(" "),
        repo.display(),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_bytes(repo: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {} in {} failed: {}",
        args.join(" "),
        repo.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn test_audit(run_id: &str, provider: &str) -> Arc<V2AuditWriter> {
    let sink: Arc<dyn AuditSink> = Arc::new(RecordingSink::default());
    Arc::new(V2AuditWriter::new(
        run_id,
        format!("{provider}:test-model"),
        sink,
    ))
}

fn rendered_implement_input_from_asset(
    yaml: &str,
    assigned_root: &Path,
    task_id: &str,
) -> (String, serde_json::Value) {
    let asset = load_job_asset(yaml).expect("committed shipment asset parses");
    assert!(
        matches!(
            asset.name.as_str(),
            "task_local_pipeline" | "task_pr_pipeline"
        ),
        "unexpected shipment asset {}",
        asset.name
    );
    let implement_bundle = asset
        .spec
        .steps
        .iter()
        .find(|step| step.id == "implement_bundle")
        .expect("shipment asset has implement_bundle");
    let JobV2StepBody::Loop { loop_ } = &implement_bundle.body else {
        panic!("{} implement_bundle must be a loop", asset.name);
    };
    let implement_one = loop_
        .steps
        .iter()
        .find(|step| step.id == "implement_one")
        .expect("shipment asset has implement_one");
    let JobV2StepBody::TargetRef(implement) = &implement_one.body else {
        panic!("{} implement_one must reference an activity", asset.name);
    };
    assert_eq!(implement.target, "activity:agent_implement");
    let template_input = implement
        .default_input
        .as_ref()
        .expect("agent_implement default input");
    for field in ["workspace_path", "repo_root"] {
        assert_eq!(
            template_input[field], "{{ steps.worktree.output.workspace_path }}",
            "{} must source {field} from the committed worktree output",
            asset.name
        );
    }

    let assigned = assigned_root.display().to_string();
    let mut steps = HashMap::new();
    steps.insert(
        "worktree".to_string(),
        serde_json::json!({ "output": { "workspace_path": assigned } }),
    );
    let context = TemplateContext {
        item: Some(serde_json::json!(task_id)),
        steps,
        ..TemplateContext::default()
    };
    let rendered = render_asset_value(template_input, &context);
    assert_eq!(rendered["task_id"], task_id);
    assert_eq!(rendered["workspace_path"], assigned);
    assert_eq!(rendered["repo_root"], assigned);
    (asset.name, rendered)
}

fn render_asset_value(value: &serde_json::Value, context: &TemplateContext) -> serde_json::Value {
    match value {
        serde_json::Value::String(template_value) if template_value.contains("{{") => {
            serde_json::Value::String(
                template::render(template_value, context).expect("render committed asset input"),
            )
        }
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .iter()
                .map(|value| render_asset_value(value, context))
                .collect(),
        ),
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), render_asset_value(value, context)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn worktree_input(fixture: &LinkedWorktreeFixture, task_id: &str) -> serde_json::Value {
    serde_json::json!({
        "prompt": "implement",
        "task_id": task_id,
        "workspace_path": fixture.assigned,
        "repo_root": fixture.assigned,
    })
}

fn worktree_integrity_diagnostic(error: &DispatchError) -> serde_json::Value {
    let DispatchError::WorktreeIntegrity { diagnostic, .. } = error else {
        panic!("expected WorktreeIntegrity, got {error:?}");
    };
    serde_json::from_str(diagnostic).expect("worktree integrity diagnostic is JSON")
}

fn assert_worktree_mismatch(
    error: &DispatchError,
    task_id: &str,
    run_id: &str,
    reason_fragment: &str,
) {
    assert!(
        matches!(
            error,
            DispatchError::WorktreeIntegrity {
                code: "worktree_mismatch",
                ..
            }
        ),
        "unexpected mismatch error: {error:?}"
    );
    assert!(error.is_non_retryable());
    let diagnostic = worktree_integrity_diagnostic(error);
    assert_eq!(diagnostic["code"], "worktree_mismatch");
    assert_eq!(diagnostic["task_id"], task_id);
    assert_eq!(diagnostic["run_id"], run_id);
    assert_eq!(diagnostic["provider"], "codex");
    assert!(
        diagnostic["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains(reason_fragment)),
        "unexpected mismatch reason: {}",
        diagnostic["reason"]
    );
}

fn assert_pre_spawn_failure(audit: &V2AuditWriter, marker: &Path) {
    assert!(!marker.exists(), "provider process must not have started");
    assert!(
        !audit
            .events_snapshot()
            .expect("audit events")
            .iter()
            .any(|event| matches!(event.kind, V2AuditEventKind::CliInvocationStarted { .. })),
        "mismatch must be rejected before cli.invocation.started"
    );
}

fn assert_worktree_integrity_error(
    error: &DispatchError,
    expected_code: &str,
    identity: (&str, &str, &str),
    fixture: &LinkedWorktreeFixture,
    changed_path: &str,
) {
    let (task_id, run_id, provider) = identity;
    assert!(
        matches!(
            error,
            DispatchError::WorktreeIntegrity { code, .. } if *code == expected_code
        ),
        "unexpected integrity error: {error:?}"
    );
    let rendered = error.to_string();
    for expected in [
        expected_code,
        task_id,
        run_id,
        provider,
        &fixture.assigned.display().to_string(),
        &fixture.primary.display().to_string(),
        changed_path,
        "assigned_before",
        "assigned_after",
        "primary_before",
        "primary_after",
    ] {
        assert!(
            rendered.contains(expected),
            "integrity diagnostic missing {expected:?}: {rendered}"
        );
    }
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
        workspace_root: None,
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
        workspace_root: None,
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
        workspace_root: None,
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

/// ORB-10342: pipeline-gate provider spawns must carry the same
/// AGENT_RUN_ID/AGENT_MODEL/AGENT_TASK trio the worker path sets
/// (ORB-10340), so the shared prepare-commit-msg injector can stamp commit
/// trailers regardless of which spawner produced the commit.
#[test]
fn run_cli_backend_sets_agent_telemetry_env_vars_for_commit_trailers() {
    let temp = tempdir().expect("tempdir");
    let script = temp.path().join("grok");
    write_executable(
        &script,
        r#"#!/bin/sh
cat > /dev/null
if [ "$AGENT_RUN_ID" = "job-grok-telemetry" ] && [ "$AGENT_MODEL" = "grok-build" ] && [ "$AGENT_TASK" = "ORB-10342" ]; then
  printf '%s\n' '{"schemaVersion":1,"status":"success","result":{"identity":"ok"},"error":null}'
else
  printf '%s\n' '{"schemaVersion":1,"status":"failed","error":{"code":"telemetry_env_missing","message":"agent telemetry env was not propagated","details":null}}'
  exit 1
fi
"#,
    );

    let sink = Arc::new(RecordingSink::default());
    let sink_for_writer: Arc<dyn AuditSink> = sink;
    let audit = Arc::new(V2AuditWriter::new(
        "job-grok-telemetry",
        "grok:grok-build",
        sink_for_writer,
    ));
    let host = TestHost {
        command: script.display().to_string(),
        executor_args: Vec::new(),
        provider_config: HashMap::new(),
        sandbox: None,
        task_context: None,
        workspace_root: None,
    };
    let mut spec = test_agent_loop_spec_for("grok", Duration::from_secs(5));
    spec.model = Some("grok-build".to_string());

    let outcome = run_cli_backend(
        &host,
        &spec,
        "job-grok-telemetry",
        audit,
        &serde_json::json!({"prompt": "hi", "task_id": "ORB-10342"}),
        None,
    )
    .expect("run succeeds");

    assert!(outcome.success);
    assert_eq!(outcome.output["provider"], "grok");
}

/// AGENT_MODEL/AGENT_TASK must be omitted (unset), not set to an empty
/// string, when the model or task id is unknown — mirrors ORB-10340's
/// worker-side semantics. AGENT_RUN_ID is always known for a dispatched run.
// Asserted at the builder boundary, not through a spawned child: provider spawns seed the
// cleared environment with `non_sensitive_env_vars()`, so an Orbit-dispatched test run's own
// AGENT_* telemetry can leak into the child and mask a correct omission. The positive
// propagation case above still covers the wiring end-to-end.
#[test]
fn run_cli_backend_omits_agent_model_and_task_env_vars_when_unknown() {
    let vars = provenance_env(ProvenanceEnv {
        orbit_run_id: Some("job-grok-telemetry-unknown"),
        orbit_managed_run_context: true,
        orbit_agent_name: Some("grok"),
        agent_run_id: Some("job-grok-telemetry-unknown"),
        ..ProvenanceEnv::default()
    });

    assert!(vars.contains(&(
        "AGENT_RUN_ID".to_string(),
        "job-grok-telemetry-unknown".to_string()
    )));
    assert!(!vars.iter().any(|(key, _)| key == "AGENT_MODEL"));
    assert!(!vars.iter().any(|(key, _)| key == "AGENT_TASK"));
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
