use chrono::{DateTime, Utc};
use orbit_core::{AuditEvent, AuditEventStatus};
use serde_json::json;

use super::super::support::audit_event_to_json;

#[test]
fn audit_list_json_projection_shape_is_stable() {
    let event = AuditEvent {
        id: 7,
        execution_id: "exec-1".to_string(),
        timestamp: DateTime::parse_from_rfc3339("2026-05-23T06:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc),
        command: "tool".to_string(),
        subcommand: Some("orbit.task.show".to_string()),
        tool_name: Some("orbit.task.show".to_string()),
        target_type: Some("task".to_string()),
        target_id: Some("ORB-00276".to_string()),
        role: "codex".to_string(),
        status: AuditEventStatus::Success,
        exit_code: 0,
        duration_ms: 42,
        working_directory: "/workspace".to_string(),
        arguments_json: Some(r#"{"id":"ORB-00276"}"#.to_string()),
        stdout_truncated: Some("{}".to_string()),
        stderr_truncated: None,
        error_message: None,
        host: Some("host.local".to_string()),
        pid: 1234,
        session_id: Some("session-1".to_string()),
        task_id: Some("ORB-00276".to_string()),
        job_run_id: Some("jrun-1".to_string()),
        activity_id: Some("implement".to_string()),
        step_index: Some(2),
    };

    assert_eq!(
        audit_event_to_json(&event),
        json!({
            "id": 7,
            "execution_id": "exec-1",
            "timestamp": "2026-05-23T06:00:00+00:00",
            "command": "tool",
            "subcommand": "orbit.task.show",
            "tool_name": "orbit.task.show",
            "target_type": "task",
            "target_id": "ORB-00276",
            "role": "codex",
            "status": "success",
            "exit_code": 0,
            "duration_ms": 42,
            "working_directory": "/workspace",
            "arguments_json": "{\"id\":\"ORB-00276\"}",
            "stdout_truncated": "{}",
            "stderr_truncated": null,
            "error_message": null,
            "host": "host.local",
            "pid": 1234,
            "session_id": "session-1",
            "task_id": "ORB-00276",
            "job_run_id": "jrun-1",
            "activity_id": "implement",
            "step_index": 2,
        })
    );
}
