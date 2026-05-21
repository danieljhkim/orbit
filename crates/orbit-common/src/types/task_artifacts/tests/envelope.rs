use chrono::{TimeZone, Utc};

use super::super::*;

fn valid_envelope_yaml(id: &str) -> String {
    format!(
        r#"schema_version: 1
id: {id}
title: Build the thing
status: backlog
type: feature
priority: medium
created_at: 2026-05-10T12:00:00Z
updated_at: 2026-05-10T12:00:00Z
"#
    )
}

fn valid_envelope(id: &str) -> TaskEnvelopeV2 {
    TaskEnvelopeV2 {
        schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
        id: id.to_string(),
        title: "Build the thing".to_string(),
        status: TaskStatus::Backlog,
        task_type: TaskType::Feature,
        priority: TaskPriority::Medium,
        complexity: None,
        job_run_id: None,
        crew: None,
        relations: Vec::new(),
        tags: Vec::new(),
        context_files: Vec::new(),
        external_refs: Vec::new(),
        created_by: None,
        planned_by: None,
        implemented_by: None,
        created_at: Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap(),
        updated_at: Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap(),
    }
}

#[test]
fn envelope_rejects_old_inline_document_fields() {
    let yaml = format!(
        "{}\ndescription: old inline body\n",
        valid_envelope_yaml("ORB-00001")
    );
    let error = serde_yaml::from_str::<TaskEnvelopeV2>(&yaml).unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn envelope_requires_schema_version() {
    let yaml = r#"
id: ORB-00001
title: Build the thing
status: backlog
type: feature
priority: medium
created_at: 2026-05-10T12:00:00Z
updated_at: 2026-05-10T12:00:00Z
"#;
    let error = serde_yaml::from_str::<TaskEnvelopeV2>(yaml).unwrap_err();
    assert!(error.to_string().contains("schema_version"));
}

#[test]
fn envelope_validate_rejects_wrong_schema_version() {
    let mut envelope = valid_envelope("ORB-00001");
    envelope.schema_version = 2;
    assert!(envelope.validate().is_err());
}

#[test]
fn jsonl_rows_validate_schema_and_required_ids() {
    let event = TaskEventRowV2 {
        schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
        event_id: "EV-0001".to_string(),
        at: Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap(),
        by: "codex:gpt-5.5".to_string(),
        event_type: "created".to_string(),
        note: None,
        from_status: None,
        to_status: Some(TaskStatus::Backlog),
    };
    assert!(event.validate().is_ok());

    let mut invalid_event = event;
    invalid_event.event_id = " ".to_string();
    assert!(invalid_event.validate().is_err());

    let comment = TaskCommentRowV2 {
        schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
        comment_id: "C-0001".to_string(),
        at: Utc.with_ymd_and_hms(2026, 5, 10, 12, 0, 0).unwrap(),
        by: "daniel".to_string(),
        body: "Looks good.".to_string(),
    };
    assert!(comment.validate().is_ok());

    let mut invalid_comment = comment;
    invalid_comment.comment_id = String::new();
    assert!(invalid_comment.validate().is_err());
}
