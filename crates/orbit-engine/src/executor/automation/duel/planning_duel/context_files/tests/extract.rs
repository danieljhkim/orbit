use super::super::{PlanContextFilesExtraction, extract_context_files_from_plan};

fn extract(plan: &str) -> Option<PlanContextFilesExtraction> {
    extract_context_files_from_plan(plan)
}

#[test]
fn extract_returns_none_when_no_context_files_section() {
    let plan = "## Plan\n\n- Step 1\n\n## Risks\n\n- Some risk.\n";
    assert!(extract(plan).is_none());
}

#[test]
fn extract_returns_canonical_entries_for_canonical_bullets() {
    let plan = "## Context Files\n\n- `file:src/a.rs`\n- `dir:crates/foo`\n- `symbol:src/lib.rs#bar:function`\n";
    let result = extract(plan).expect("section present");
    assert_eq!(
        result.canonical_entries,
        vec![
            "file:src/a.rs".to_string(),
            "dir:crates/foo".to_string(),
            "symbol:src/lib.rs#bar:function".to_string(),
        ]
    );
    assert!(result.skipped.is_empty());
}

#[test]
fn extract_upgrades_raw_paths_to_file_or_dir() {
    let plan = "## Context Files\n\n- crates/foo/bar.rs\n- crates/foo/\n";
    let result = extract(plan).expect("section present");
    assert_eq!(
        result.canonical_entries,
        vec![
            "file:crates/foo/bar.rs".to_string(),
            "dir:crates/foo".to_string(),
        ]
    );
}

#[test]
fn extract_handles_mixed_canonical_and_raw_entries_in_one_list() {
    let plan = "## Context Files\n- `file:src/a.rs`\n- src/b.rs\n- `dir:crates/x`\n- crates/y/\n";
    let result = extract(plan).expect("section present");
    assert_eq!(
        result.canonical_entries,
        vec![
            "file:src/a.rs".to_string(),
            "file:src/b.rs".to_string(),
            "dir:crates/x".to_string(),
            "dir:crates/y".to_string(),
        ]
    );
}

#[test]
fn extract_strips_em_dash_annotations_after_entry() {
    let plan = "## Context Files\n\n- `file:src/a.rs` — handles the read path.\n- `file:src/b.rs` (helper).\n";
    let result = extract(plan).expect("section present");
    assert_eq!(
        result.canonical_entries,
        vec!["file:src/a.rs".to_string(), "file:src/b.rs".to_string()]
    );
}

#[test]
fn extract_terminates_section_at_next_same_level_heading() {
    let plan =
        "## Context Files\n\n- `file:src/a.rs`\n\n## Risks\n\n- `file:should/not/appear.rs`\n";
    let result = extract(plan).expect("section present");
    assert_eq!(result.canonical_entries, vec!["file:src/a.rs".to_string()]);
}

#[test]
fn extract_terminates_section_at_higher_level_heading() {
    let plan =
        "## Context Files\n\n- `file:src/a.rs`\n\n# Top Level\n\n- `file:should/not/appear.rs`\n";
    let result = extract(plan).expect("section present");
    assert_eq!(result.canonical_entries, vec!["file:src/a.rs".to_string()]);
}

#[test]
fn extract_dedupes_preserving_first_seen_order() {
    let plan =
        "## Context Files\n- `file:src/a.rs`\n- `file:src/b.rs`\n- `file:src/a.rs`\n- src/b.rs\n";
    let result = extract(plan).expect("section present");
    assert_eq!(
        result.canonical_entries,
        vec!["file:src/a.rs".to_string(), "file:src/b.rs".to_string()]
    );
}

#[test]
fn extract_returns_none_when_section_recognized_but_empty() {
    let plan = "## Context Files\n\n## Risks\n\n- `file:should/not/appear.rs`\n";
    assert!(extract(plan).is_none());
}

#[test]
fn extract_skips_unparseable_entries_and_records_skip() {
    // `symbol:foo` is missing the `#name:kind` suffix → canonicalization fails.
    let plan = "## Context Files\n\n- `file:src/a.rs`\n- `symbol:foo`\n";
    let result = extract(plan).expect("section present");
    assert_eq!(result.canonical_entries, vec!["file:src/a.rs".to_string()]);
    assert_eq!(result.skipped.len(), 1, "one entry should be skipped");
    assert_eq!(result.skipped[0].raw_entry, "symbol:foo");
    assert!(
        !result.skipped[0].reason.is_empty(),
        "skip reason should be populated"
    );
}

#[test]
fn extract_skips_subbullets_and_non_bullet_lines() {
    let plan = "## Context Files\n\n- `file:src/a.rs`\n  - `file:nested/should-not-appear.rs`\nDescription line.\n* `file:src/b.rs`\n";
    let result = extract(plan).expect("section present");
    assert_eq!(
        result.canonical_entries,
        vec!["file:src/a.rs".to_string(), "file:src/b.rs".to_string()]
    );
}

#[test]
fn extract_accepts_h3_section() {
    let plan = "### Context Files\n\n- `file:src/a.rs`\n";
    let result = extract(plan).expect("section present");
    assert_eq!(result.canonical_entries, vec!["file:src/a.rs".to_string()]);
}

#[test]
fn extract_rejects_non_exact_heading() {
    let plan = "## Context files referenced by step 3\n\n- `file:src/a.rs`\n";
    assert!(extract(plan).is_none());
}

#[test]
fn extract_accepts_trailing_colon_on_heading() {
    let plan = "## Context Files:\n\n- `file:src/a.rs`\n";
    let result = extract(plan).expect("section present");
    assert_eq!(result.canonical_entries, vec!["file:src/a.rs".to_string()]);
}

#[test]
fn extract_handles_t20260509_7_shape() {
    // Snapshot of a representative T20260509-7 winning plan ## Context Files
    // section as of 2026-05-08. Pinned inline rather than referencing the
    // live task record so the test is stable.
    let plan = r#"## Plan

Do the work.

## Context Files

- `symbol:crates/orbit-engine/src/executor/automation/duel/planning_duel/artifacts.rs#writeback_planning_duel_task:function` — primary insertion point.
- `file:crates/orbit-engine/src/context.rs` — add the `context_files` field.
- `dir:crates/orbit-engine/src/executor/automation/duel/planning_duel` — module folder.
- `file:CLAUDE.md` — doc-update rule.

## Risks

- Heuristics drift.
"#;
    let result = extract(plan).expect("section present");
    assert_eq!(
        result.canonical_entries,
        vec![
            "symbol:crates/orbit-engine/src/executor/automation/duel/planning_duel/artifacts.rs#writeback_planning_duel_task:function".to_string(),
            "file:crates/orbit-engine/src/context.rs".to_string(),
            "dir:crates/orbit-engine/src/executor/automation/duel/planning_duel".to_string(),
            "file:CLAUDE.md".to_string(),
        ]
    );
}
