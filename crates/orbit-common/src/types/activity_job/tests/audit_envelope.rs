use serde_json::json;

use super::super::audit_envelope::*;

#[test]
fn step_finished_error_message_round_trips_and_absence_defaults_to_none() {
    let encoded = serde_json::to_value(V2AuditEventKind::StepFinished {
        step_id: "plan".to_string(),
        outcome: "error".to_string(),
        error_message: Some("dispatch failed".to_string()),
    })
    .expect("serialize step finished");

    assert_eq!(encoded["error_message"], "dispatch failed");
    let decoded: V2AuditEventKind =
        serde_json::from_value(encoded).expect("deserialize step finished");
    assert!(matches!(
        decoded,
        V2AuditEventKind::StepFinished {
            step_id,
            outcome,
            error_message: Some(message)
        } if step_id == "plan" && outcome == "error" && message == "dispatch failed"
    ));

    let decoded: V2AuditEventKind = serde_json::from_value(json!({
        "body_kind": "step_finished",
        "step_id": "plan",
        "outcome": "error"
    }))
    .expect("deserialize legacy step finished");
    assert!(matches!(
        decoded,
        V2AuditEventKind::StepFinished {
            error_message: None,
            ..
        }
    ));
}

#[test]
fn cli_invocation_process_round_trips_with_and_without_identity_token() {
    let encoded = serde_json::to_value(V2AuditEventKind::CliInvocationProcess {
        provider: "codex".to_string(),
        pid: 4242,
        pid_start_time: Some("ps-lstart-utc-v1:Mon Jul 27 02:41:00 2026".to_string()),
    })
    .expect("serialize cli invocation process");

    assert_eq!(encoded["body_kind"], "cli_invocation_process");
    assert_eq!(encoded["pid"], 4242);
    assert_eq!(
        encoded["pid_start_time"],
        "ps-lstart-utc-v1:Mon Jul 27 02:41:00 2026"
    );

    let decoded: V2AuditEventKind =
        serde_json::from_value(encoded).expect("deserialize cli invocation process");
    assert_eq!(decoded.event_type(), "cli.invocation.process");
    assert!(matches!(
        decoded,
        V2AuditEventKind::CliInvocationProcess {
            provider,
            pid: 4242,
            pid_start_time: Some(_)
        } if provider == "codex"
    ));

    // A host that cannot probe process start identity (non-Unix, or a sandbox
    // that blocks `ps`) still records a usable PID.
    let encoded = serde_json::to_value(V2AuditEventKind::CliInvocationProcess {
        provider: "claude".to_string(),
        pid: 7,
        pid_start_time: None,
    })
    .expect("serialize unprobed cli invocation process");
    assert!(encoded.get("pid_start_time").is_none());

    let decoded: V2AuditEventKind = serde_json::from_value(json!({
        "body_kind": "cli_invocation_process",
        "provider": "claude",
        "pid": 7
    }))
    .expect("deserialize cli invocation process without token");
    assert!(matches!(
        decoded,
        V2AuditEventKind::CliInvocationProcess {
            pid: 7,
            pid_start_time: None,
            ..
        }
    ));
}

#[test]
fn run_finished_error_message_round_trips_and_absence_defaults_to_none() {
    let encoded = serde_json::to_value(V2AuditEventKind::RunFinished {
        outcome: "error".to_string(),
        error_message: Some("job failed".to_string()),
    })
    .expect("serialize run finished");

    assert_eq!(encoded["error_message"], "job failed");
    let decoded: V2AuditEventKind =
        serde_json::from_value(encoded).expect("deserialize run finished");
    assert!(matches!(
        decoded,
        V2AuditEventKind::RunFinished {
            outcome,
            error_message: Some(message)
        } if outcome == "error" && message == "job failed"
    ));

    let encoded = serde_json::to_value(V2AuditEventKind::RunFinished {
        outcome: "success".to_string(),
        error_message: None,
    })
    .expect("serialize successful run finished");
    assert!(encoded.get("error_message").is_none());

    let decoded: V2AuditEventKind = serde_json::from_value(json!({
        "body_kind": "run_finished",
        "outcome": "success"
    }))
    .expect("deserialize legacy run finished");
    assert!(matches!(
        decoded,
        V2AuditEventKind::RunFinished {
            error_message: None,
            ..
        }
    ));
}
