//! Schema tests for orbit.search tool.

use super::super::*;

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
