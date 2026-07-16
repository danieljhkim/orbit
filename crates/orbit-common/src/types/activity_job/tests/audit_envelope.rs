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

#[test]
fn step_recovery_resumed_error_round_trips() {
    let encoded = serde_json::to_value(V2AuditEventKind::StepRecoveryResumed {
        step_id: "pr_open".to_string(),
        attempt: 2,
        outcome: "error".to_string(),
        error_message: Some("push failed after recovery".to_string()),
    })
    .expect("serialize resumed recovery step");

    assert_eq!(encoded["body_kind"], "step_recovery_resumed");
    assert_eq!(encoded["attempt"], 2);
    assert_eq!(encoded["error_message"], "push failed after recovery");
    let decoded: V2AuditEventKind =
        serde_json::from_value(encoded).expect("deserialize resumed recovery step");
    assert!(matches!(
        decoded,
        V2AuditEventKind::StepRecoveryResumed {
            step_id,
            attempt: 2,
            outcome,
            error_message: Some(message),
        } if step_id == "pr_open" && outcome == "error" && message == "push failed after recovery"
    ));
}
