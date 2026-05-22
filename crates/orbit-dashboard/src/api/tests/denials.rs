use chrono::{Duration, Utc};
use orbit_core::{AuditEventStatus, OrbitRuntime};
use serde_json::json;

use super::super::denials::{SQLITE_FS_BOUNDARY_PROFILE, collect_denial_rows, denials_payload};
use super::test_support::write_lines;

#[test]
fn denials_payload_combines_v2_and_sqlite_denials() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let audit_dir = runtime.data_root().join("state/audit/v2_loop");
    std::fs::create_dir_all(&audit_dir).expect("create audit dir");
    let now = Utc::now();
    write_lines(
        &audit_dir.join("run-v2-denials.jsonl"),
        &[
            json!({
                "schemaVersion": 1,
                "event_type": "fs.call.denied",
                "event_id": "evt-fs-denied",
                "ts": now.to_rfc3339(),
                "run_id": "run-v2-denials",
                "agent_identity": "codex / gpt-5",
                "body_kind": "fs_call_denied",
                "profile": "restricted",
                "path": "./secret.txt"
            })
            .to_string(),
            json!({
                "schemaVersion": 1,
                "event_type": "tool.denied",
                "event_id": "evt-tool-denied",
                "ts": now.to_rfc3339(),
                "run_id": "run-v2-denials",
                "agent_identity": "codex / gpt-5",
                "body_kind": "tool_denied",
                "tool_name": "github.pr.merge"
            })
            .to_string(),
        ],
    );
    runtime
        .record_audit_event(&orbit_core::AuditEventInsertParams {
            execution_id: "exec-sqlite-fs".to_string(),
            command: "tool".to_string(),
            subcommand: Some("fs.read".to_string()),
            tool_name: Some("fs.read".to_string()),
            target_type: Some("tool".to_string()),
            target_id: Some("fs.read".to_string()),
            role: "codex".to_string(),
            status: AuditEventStatus::Denied,
            exit_code: 1,
            duration_ms: 5,
            working_directory: "/workspace".to_string(),
            arguments_json: None,
            stdout_truncated: None,
            stderr_truncated: None,
            error_message: Some("path is outside workspace: /usr/bin/false".to_string()),
            host: None,
            pid: 123,
            session_id: None,
            task_id: None,
            job_run_id: None,
            activity_id: None,
            step_index: None,
        })
        .expect("record sqlite denial");

    let since = now - Duration::minutes(5);
    let rows = collect_denial_rows(&runtime, Some(since), None, None).expect("collect denials");
    let payload = denials_payload(&rows, None, Some(since));
    assert_eq!(payload["total"], 3);
    assert!(payload["by_target"].to_string().contains("/usr/bin/false"));
    assert!(payload["by_target"].to_string().contains("./secret.txt"));
    assert!(payload["by_target"].to_string().contains("github.pr.merge"));

    let fs_payload = denials_payload(&rows, Some("fs"), Some(since));
    assert_eq!(fs_payload["total"], 2);
    assert!(
        fs_payload["by_profile"]
            .to_string()
            .contains(SQLITE_FS_BOUNDARY_PROFILE)
    );
    assert!(fs_payload["by_profile"].to_string().contains("restricted"));

    let tool_payload = denials_payload(&rows, Some("tool"), Some(since));
    assert_eq!(tool_payload["total"], 1);

    let sqlite_only = collect_denial_rows(
        &runtime,
        Some(since),
        Some(SQLITE_FS_BOUNDARY_PROFILE),
        Some("codex"),
    )
    .expect("collect filtered sqlite denials");
    assert_eq!(sqlite_only.len(), 1);
    assert_eq!(sqlite_only[0].target(), "/usr/bin/false");
}

#[test]
fn denials_payload_distinguishes_job_runs_from_audit_executions() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let since = Utc::now() - Duration::minutes(5);
    runtime
        .record_audit_event(&orbit_core::AuditEventInsertParams {
            execution_id: "exec-linked-to-run".to_string(),
            command: "tool".to_string(),
            subcommand: Some("orbit.task.update".to_string()),
            tool_name: Some("orbit.task.update".to_string()),
            target_type: Some("task".to_string()),
            target_id: Some("ORB-00001".to_string()),
            role: "codex".to_string(),
            status: AuditEventStatus::Denied,
            exit_code: 1,
            duration_ms: 7,
            working_directory: "/workspace".to_string(),
            arguments_json: None,
            stdout_truncated: None,
            stderr_truncated: None,
            error_message: Some("denied by policy".to_string()),
            host: None,
            pid: 123,
            session_id: None,
            task_id: Some("ORB-00001".to_string()),
            job_run_id: Some("jrun-real-policy".to_string()),
            activity_id: Some("agent_implement".to_string()),
            step_index: Some(0),
        })
        .expect("record linked denial");
    runtime
        .record_audit_event(&orbit_core::AuditEventInsertParams {
            execution_id: "audit-task-locks-reserve-denied-test".to_string(),
            command: "task.locks.reserve.denied".to_string(),
            subcommand: None,
            tool_name: Some("orbit.task.locks.reserve".to_string()),
            target_type: Some("task_reservation".to_string()),
            target_id: None,
            role: "admin".to_string(),
            status: AuditEventStatus::Denied,
            exit_code: 1,
            duration_ms: 0,
            working_directory: "/workspace".to_string(),
            arguments_json: Some(
                json!({
                    "actor": "codex / gpt-5.5",
                    "task_ids": ["ORB-00001"],
                    "files": ["file:crates/orbit-cli/src/lib.rs"],
                    "conflicts": [{
                        "file": "file:crates/orbit-cli/src/lib.rs",
                        "held_by": "task",
                        "held_by_id": "ORB-00002"
                    }]
                })
                .to_string(),
            ),
            stdout_truncated: None,
            stderr_truncated: None,
            error_message: None,
            host: None,
            pid: 123,
            session_id: None,
            task_id: None,
            job_run_id: None,
            activity_id: None,
            step_index: None,
        })
        .expect("record task-lock denial");

    let rows = collect_denial_rows(&runtime, Some(since), None, None).expect("collect denials");
    let payload = denials_payload(&rows, None, Some(since));

    let by_run = payload["by_run"].as_array().expect("by_run array");
    assert_eq!(by_run.len(), 1);
    assert_eq!(by_run[0]["run_id"], "jrun-real-policy");
    assert!(
        !payload["by_run"]
            .to_string()
            .contains("audit-task-locks-reserve-denied-test"),
        "audit execution IDs must not be rendered as JobRun IDs"
    );
}
