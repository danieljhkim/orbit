//! Tests for refresh diagnostics in pack output.
//
// Migrated from nested `knowledge/pack/tests/` (anti-pattern child of source)
// to sibling under `knowledge/tests/` per ORB-00243 and
// docs/design-patterns/test_layout.md.

use orbit_common::types::OrbitError;
use orbit_knowledge::KnowledgePackResult;
use serde_json::json;

use super::super::pack::{OrbitKnowledgePackTool, add_refresh_diagnostics, parse_selector_strings};
use crate::Tool;

/// ORB-00382: the published schema and the handler must agree on `selectors`.
/// The schema marks it required and the handler enforces it — so a caller that
/// passes exactly the schema's required field succeeds, and one that omits it
/// gets the same `missing selectors` error the MCP surface surfaces.
#[test]
fn pack_schema_marks_selectors_required_and_handler_enforces_it() {
    // Schema side: `selectors` is the sole required parameter.
    let required: Vec<String> = OrbitKnowledgePackTool
        .schema()
        .parameters
        .into_iter()
        .filter(|param| param.required)
        .map(|param| param.name)
        .collect();
    assert_eq!(required, vec!["selectors".to_string()]);

    // Handler side: omitting `selectors` is rejected with `missing selectors`.
    let err = parse_selector_strings(&json!({})).expect_err("missing selectors must be rejected");
    assert!(
        matches!(&err, OrbitError::InvalidInput(message) if message.contains("missing `selectors`")),
        "expected missing-selectors InvalidInput, got {err:?}"
    );

    // Calling with exactly the schema's required field succeeds (no missing error).
    let parsed = parse_selector_strings(&json!({
        "selectors": "symbol:crates/orbit-graph/src/lib.rs#RefResult"
    }))
    .expect("a call carrying `selectors` parses");
    assert_eq!(
        parsed,
        vec!["symbol:crates/orbit-graph/src/lib.rs#RefResult".to_string()]
    );
}

#[test]
fn refresh_diagnostics_only_report_actual_auto_refresh_skips() {
    let mut refreshed_pack = KnowledgePackResult::default();
    add_refresh_diagnostics(&mut refreshed_pack, false, None, false);
    assert!(refreshed_pack.diagnostics.is_none());

    let mut skipped_pack = KnowledgePackResult::default();
    add_refresh_diagnostics(&mut skipped_pack, true, None, false);
    assert_eq!(
        skipped_pack
            .diagnostics
            .and_then(|diagnostics| diagnostics.auto_refresh)
            .map(|diagnostic| diagnostic.status),
        Some("skipped".to_string())
    );
}
