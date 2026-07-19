//! Sibling tests for `task/output.rs` (docs/design-patterns/test_layout.md).

use orbit_common::types::TaskHistoryEntry;

use crate::command::task::output::is_human_visible_history_event;

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
