use std::str::FromStr;

use orbit_common::types::{
    OrbitError, optional_string_alias, optional_string_list_alias, optional_u32_alias,
};
use serde_json::Value;

use crate::{GlobalSearchKind, GlobalSearchParams, OrbitRuntime};

use super::input::optional_bool_alias;

pub(super) fn search(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    // ADR-0175: hard-break the retired neighbor parameter; no compatibility shim.
    if input.get("related").is_some() {
        return Err(OrbitError::InvalidInput(
            "unknown parameter `related`; use `semantic` for task-neighbor lookup".to_string(),
        ));
    }

    let semantic = optional_string_alias(&input, &["semantic", "id", "task_id", "taskId"])?;
    let hybrid = optional_bool_alias(&input, &["hybrid"])?.unwrap_or(false);
    let kind = optional_string_alias(&input, &["kind"])?
        .map(|kind| GlobalSearchKind::from_str(&kind).map_err(OrbitError::InvalidInput))
        .transpose()?
        .unwrap_or_default();

    let result = runtime.global_search(GlobalSearchParams {
        query: optional_string_alias(&input, &["query"])?,
        hybrid,
        semantic,
        kind,
        limit: optional_u32_alias(&input, &["limit"])?
            .map(|limit| limit as usize)
            .unwrap_or(10),
        field: optional_string_alias(&input, &["field"])?,
        // L-0006: `model` is tool-run provenance; embedding selection uses a separate field.
        model: optional_string_alias(
            &input,
            &[
                "embedding_model",
                "embeddingModel",
                "embedding-model",
                "semantic_model",
                "semanticModel",
            ],
        )?,
        tags: optional_string_list_alias(&input, &["tag", "tags"])?.unwrap_or_default(),
        all: optional_bool_alias(&input, &["all"])?.unwrap_or(false),
        status: optional_string_list_alias(&input, &["status", "statuses"])?.unwrap_or_default(),
        path: optional_string_alias(&input, &["path"])?,
    })?;
    serde_json::to_value(result)
        .map_err(|error| OrbitError::Execution(format!("serialize search result: {error}")))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn search_tool_rejects_legacy_related_param() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let error = search(&runtime, json!({ "related": "ORB-00001" }))
            .expect_err("legacy related parameter should be rejected");

        assert!(error.to_string().contains("unknown parameter `related`"));
    }

    #[test]
    fn search_tool_rejects_boolean_semantic_param() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let mut input = serde_json::Map::new();
        input.insert("query".to_string(), json!("anything"));
        input.insert("semantic".to_string(), json!(true));
        let error = search(&runtime, Value::Object(input))
            .expect_err("semantic parameter should require a task ID string");

        assert!(error.to_string().contains("`semantic` must be a string"));
    }
}
