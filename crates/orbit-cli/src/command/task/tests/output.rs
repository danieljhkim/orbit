//! Sibling tests for `task/output.rs` (docs/design-patterns/test_layout.md).

use orbit_types::task::TaskHistoryEntry;
use serde_json::json;

use crate::command::task::output::{format_task_locks, is_human_visible_history_event};

fn entry(event: &str) -> TaskHistoryEntry {
    TaskHistoryEntry {
        at: chrono::Utc::now(),
        by: "human".to_string(),
        event: event.to_string(),
        note: None,
        from_status: None,
        to_status: None,
    }
}

#[test]
fn commented_events_are_suppressed_from_human_history() {
    // ORB-10311: legacy bare `commented` stubs are hidden from human-facing
    // history, while every operational lifecycle event stays visible.
    assert!(!is_human_visible_history_event("commented"));
    for event in [
        "status_changed",
        "renamed",
        "updated",
        "started",
        "context_pruned",
        "approved",
        "rejected",
    ] {
        assert!(
            is_human_visible_history_event(event),
            "operational event `{event}` must remain visible"
        );
    }
}

#[test]
fn human_history_filter_drops_only_commented_entries_and_preserves_order() {
    let history = [
        entry("status_changed"),
        entry("commented"),
        entry("renamed"),
        entry("commented"),
        entry("updated"),
    ];
    // Mirror the rendering-site filter: keep everything except legacy stubs.
    let visible: Vec<&str> = history
        .iter()
        .filter(|entry| is_human_visible_history_event(&entry.event))
        .map(|entry| entry.event.as_str())
        .collect();
    assert_eq!(visible, vec!["status_changed", "renamed", "updated"]);
}

#[test]
fn format_task_locks_reports_no_locks_when_projection_is_empty() {
    let locks = json!({
        "locked_files": [],
        "by_task": [],
        "by_reservation": [],
        "total_locked": 0,
        "total_tasks": 0,
        "total_reservations": 0,
    });
    assert_eq!(format_task_locks(&locks), "No files currently locked.\n");
}

#[test]
fn format_task_locks_agrees_with_the_json_projection_on_reservation_content() {
    // ORB-10651: `--json` prints this same `orbit.task.locks` document
    // verbatim, so asserting against its fields here is exactly the
    // human/JSON agreement the CLI must not lose.
    let locks = json!({
        "locked_files": ["file:src/lib.rs"],
        "by_task": [{
            "id": "ORB-1",
            "title": "Do the thing",
            "status": "in-progress",
            "job_run_id": "jrun-1",
            "crew": "sonnet",
            "orchestrator": null,
            "context_files": ["file:src/lib.rs"],
        }],
        "by_reservation": [{
            "reservation_id": "reservation-123",
            "workspace_id": "repo-abc",
            "task_ids": ["ORB-1"],
            "files": ["file:src/lib.rs"],
            "actor": "codex",
            "created_at": "2026-08-09T00:00:00Z",
            "expires_at": "2026-08-09T01:00:00Z",
            "owner_run_id": null,
            "owner_metadata_json": null,
        }],
        "total_locked": 1,
        "total_tasks": 1,
        "total_reservations": 1,
    });

    let human = format_task_locks(&locks);
    assert!(human.contains("[ORB-1] Do the thing (in-progress, job_run=jrun-1)"));
    assert!(human.contains("  - file:src/lib.rs"));
    assert!(human.contains("1 file(s) locked across 1 task(s)."));

    let reservation = &locks["by_reservation"][0];
    let reservation_id = reservation["reservation_id"].as_str().unwrap();
    assert!(
        human.contains(reservation_id),
        "human listing must surface the same reservation id the JSON document exposes: {human}"
    );
    assert!(human.contains("tasks=[ORB-1]"));
    assert!(human.contains("files=[file:src/lib.rs]"));
}
