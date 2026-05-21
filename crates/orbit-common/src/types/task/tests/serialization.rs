use super::super::{Task, TaskStatus, normalize_task_tags};

#[test]
fn task_deserializes_missing_tags_as_empty_vec() {
    let task = serde_yaml::from_str::<Task>(
        r#"id: T20260101-1
title: Legacy task
description: Existing task record.
acceptance_criteria: []
dependencies: []
plan: ""
execution_summary: ""
context_files: []
status: backlog
priority: medium
task_type: chore
created_at: 2026-01-01T00:00:00Z
updated_at: 2026-01-01T00:00:00Z
"#,
    )
    .expect("task without tags deserializes");

    assert_eq!(task.tags, Vec::<String>::new());
    assert_eq!(task.crew, None);
}

#[test]
fn task_round_trips_with_crew_set() {
    let task = serde_yaml::from_str::<Task>(
        r#"id: T20260101-1
title: Crew task
description: Existing task record.
acceptance_criteria: []
dependencies: []
plan: ""
execution_summary: ""
context_files: []
status: backlog
priority: medium
task_type: chore
crew: opus-codex
created_at: 2026-01-01T00:00:00Z
updated_at: 2026-01-01T00:00:00Z
"#,
    )
    .expect("task with crew deserializes");

    let serialized = serde_yaml::to_string(&task).expect("serialize task");
    let reparsed = serde_yaml::from_str::<Task>(&serialized).expect("reparse task");

    assert_eq!(reparsed, task);
    assert_eq!(reparsed.crew.as_deref(), Some("opus-codex"));
}

#[test]
fn normalize_task_tags_trims_lowercases_and_dedupes() {
    let tags = normalize_task_tags(vec![
        "  Perf ".to_string(),
        "BENCH".to_string(),
        "perf".to_string(),
        "   ".to_string(),
    ]);

    assert_eq!(tags, vec!["perf", "bench"]);
}

#[test]
fn task_status_deserializes_both_hyphen_and_snake_for_in_progress() {
    let snake: TaskStatus = serde_json::from_str("\"in_progress\"").expect("snake de");
    let hyphen: TaskStatus = serde_json::from_str("\"in-progress\"").expect("hyphen de");
    assert_eq!(snake, TaskStatus::InProgress);
    assert_eq!(hyphen, TaskStatus::InProgress);
    // serialize remains snake_case for persisted history/events compat with prior records
    assert_eq!(
        serde_json::to_string(&TaskStatus::InProgress).expect("ser"),
        "\"in_progress\""
    );
}
