use crate::task::{
    TASK_SHOW_PROJECTION_FIELDS, TASK_SHOW_PROJECTION_FIELDS_CSV, Task,
    is_task_show_projection_field, task_show_record_field_json, unknown_task_show_field_message,
};
use serde_json::json;

fn fixture_task() -> Task {
    serde_yaml::from_str::<Task>(
        r#"id: ORB-10966
title: Project ordinary fields
description: Fixture.
acceptance_criteria: []
dependencies: []
plan: ""
execution_summary: ""
context_files: []
status: in-progress
priority: high
complexity: medium
task_type: bug
created_at: 2026-08-22T19:39:26.449604717+00:00
updated_at: 2026-08-22T21:36:50.192298986+00:00
"#,
    )
    .expect("fixture task deserializes")
}

#[test]
fn projection_csv_matches_the_field_slice() {
    assert_eq!(
        TASK_SHOW_PROJECTION_FIELDS.join(", "),
        TASK_SHOW_PROJECTION_FIELDS_CSV
    );
}

#[test]
fn vocabulary_includes_ordinary_top_level_fields() {
    for field in [
        "id",
        "title",
        "type",
        "status",
        "priority",
        "complexity",
        "created_at",
        "updated_at",
    ] {
        assert!(
            is_task_show_projection_field(field),
            "{field} must be projectable"
        );
    }
    assert!(!is_task_show_projection_field("not_a_field"));
}

#[test]
fn unknown_field_error_lists_the_vocabulary() {
    let message = unknown_task_show_field_message("not_a_field");
    assert!(message.contains("unknown field selector `not_a_field`"),);
    assert!(message.contains(TASK_SHOW_PROJECTION_FIELDS_CSV));
    assert!(message.contains("status"));
}

#[test]
fn record_field_json_uses_canonical_serialized_types() {
    let task = fixture_task();
    assert_eq!(
        task_show_record_field_json(&task, "status"),
        Some(json!("in-progress"))
    );
    assert_eq!(
        task_show_record_field_json(&task, "id"),
        Some(json!("ORB-10966"))
    );
    assert_eq!(
        task_show_record_field_json(&task, "title"),
        Some(json!("Project ordinary fields"))
    );
    assert_eq!(
        task_show_record_field_json(&task, "type"),
        Some(json!("bug"))
    );
    assert_eq!(
        task_show_record_field_json(&task, "priority"),
        Some(json!("high"))
    );
    assert_eq!(
        task_show_record_field_json(&task, "complexity"),
        Some(json!("medium"))
    );
    assert_eq!(
        task_show_record_field_json(&task, "created_at"),
        Some(json!(task.created_at.to_rfc3339()))
    );
    assert_eq!(
        task_show_record_field_json(&task, "updated_at"),
        Some(json!(task.updated_at.to_rfc3339()))
    );
    assert_eq!(task_show_record_field_json(&task, "comments"), None);
    assert_eq!(task_show_record_field_json(&task, "not_a_field"), None);
}

#[test]
fn unset_complexity_projects_as_json_null() {
    let mut task = fixture_task();
    task.complexity = None;
    assert_eq!(
        task_show_record_field_json(&task, "complexity"),
        Some(json!(null))
    );
}
