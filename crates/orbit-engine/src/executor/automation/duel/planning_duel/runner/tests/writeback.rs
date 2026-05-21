use serde_json::json;

use orbit_common::types::TaskStatus;

use crate::context::TaskReadHost;

use super::{PlanningDuelHost, install_planning_duel_artifacts, run_writeback};

#[test]
fn writeback_populates_context_files_when_section_present() {
    let host = PlanningDuelHost::new(TaskStatus::InProgress);
    install_planning_duel_artifacts(
        &host,
        "## Plan\nDo it.\n\n## Context Files\n\n- `file:src/a.rs`\n- crates/foo/\n",
    );

    let _ = run_writeback(&host);

    assert_eq!(
        host.last_context_files_update(),
        Some(Some(vec![
            "file:src/a.rs".to_string(),
            "dir:crates/foo".to_string(),
        ])),
        "writeback should set context_files to canonical entries"
    );

    let task = host.get_task("T20260430-STATUS").expect("task readable");
    assert_eq!(
        task.context_files,
        vec!["file:src/a.rs".to_string(), "dir:crates/foo".to_string()]
    );
}

#[test]
fn writeback_preserves_context_files_when_section_absent() {
    let host = PlanningDuelHost::new(TaskStatus::InProgress);
    // Pre-populate the task's context_files with curated state.
    host.task.lock().expect("task lock").context_files = vec!["file:pre-existing.rs".to_string()];
    install_planning_duel_artifacts(&host, "## Plan\nNo Context Files section here.\n");

    let _ = run_writeback(&host);

    assert_eq!(
        host.last_context_files_update(),
        Some(None),
        "writeback must leave context_files untouched when no section is present"
    );

    let task = host.get_task("T20260430-STATUS").expect("task readable");
    assert_eq!(
        task.context_files,
        vec!["file:pre-existing.rs".to_string()],
        "pre-existing context_files should be preserved"
    );
}

#[test]
fn writeback_preserves_context_files_when_section_recognized_but_empty() {
    let host = PlanningDuelHost::new(TaskStatus::InProgress);
    host.task.lock().expect("task lock").context_files = vec!["file:pre-existing.rs".to_string()];
    install_planning_duel_artifacts(
        &host,
        "## Plan\nDo it.\n\n## Context Files\n\n## Risks\n- something.\n",
    );

    let _ = run_writeback(&host);

    assert_eq!(
        host.last_context_files_update(),
        Some(None),
        "an empty Context Files section should not clear the field"
    );
}

#[test]
fn writeback_is_idempotent_across_two_resolves() {
    let host = PlanningDuelHost::new(TaskStatus::InProgress);
    install_planning_duel_artifacts(
        &host,
        "## Plan\nDo it.\n\n## Context Files\n- `file:src/a.rs`\n- `dir:src`\n",
    );

    let _ = run_writeback(&host);
    let first = host.last_context_files_update();

    let _ = run_writeback(&host);
    let second = host.last_context_files_update();

    assert!(first.is_some());
    assert_eq!(
        first, second,
        "two consecutive resolves must produce identical context_files"
    );
}
