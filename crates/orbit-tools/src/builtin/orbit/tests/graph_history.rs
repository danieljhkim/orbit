//! Schema tests for the deprecated graph history tool.
//
// Migrated from nested `graph_history/tests/` (anti-pattern child of source)
// to sibling layout under `orbit/tests/` per ORB-00243 and
// docs/design-patterns/test_layout.md.
//
// The structural change ensures tests only reach `pub` / `pub(crate)` / `pub(super)`
// items of `graph_history`.

use super::super::graph_history::*;
use crate::Tool;

#[test]
fn schema_lists_required_selector_without_task_id_pattern() {
    let tool = OrbitGraphHistoryTool;
    let schema = tool.schema();
    assert_eq!(schema.name, "orbit.graph.history");
    let selector = schema
        .parameters
        .iter()
        .find(|p| p.name == "selector")
        .expect("selector param present");
    assert!(selector.required);
    assert!(
        !schema
            .parameters
            .iter()
            .any(|p| p.name == "task_id_pattern")
    );
}

#[test]
fn schema_description_points_to_git_grep() {
    let schema = OrbitGraphHistoryTool.schema();
    assert!(
        schema.description.contains("git log --grep"),
        "description should mention git grep replacement: {}",
        schema.description
    );
}
