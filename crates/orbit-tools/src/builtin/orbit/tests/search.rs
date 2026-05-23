//! Schema tests for orbit.search tool.
//
// Migrated from nested `search/tests/` (anti-pattern child of source)
// to sibling layout under `orbit/tests/` per ORB-00243 and
// docs/design-patterns/test_layout.md.

use super::super::search::*;
use crate::Tool;

#[test]
fn search_schema_uses_hybrid_and_semantic_task_id_params() {
    let schema = OrbitSearchTool.schema();
    let params = schema
        .parameters
        .iter()
        .map(|param| (param.name.as_str(), param.param_type.as_str()))
        .collect::<Vec<_>>();

    assert!(params.contains(&("hybrid", "boolean")));
    assert!(params.contains(&("semantic", "string")));
    assert!(!params.iter().any(|(name, _)| *name == "related"));
    assert!(!params.iter().any(|(name, _)| *name == "field"));
    assert!(!params.iter().any(|(name, _)| *name == "embedding_model"));
}
