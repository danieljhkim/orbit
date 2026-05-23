use std::str::FromStr;

use orbit_common::types::{
    OrbitError, optional_csv_or_string_list_alias, optional_string_alias,
    optional_string_list_alias, optional_u32_alias,
};
use serde_json::Value;

use crate::{GlobalSearchKind, GlobalSearchParams, OrbitRuntime};

use super::input::optional_bool_alias;

pub(super) fn search(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    // ADR-0179: hard-break retired search parameters; no compatibility shim.
    if input.get("related").is_some() {
        return Err(OrbitError::InvalidInput(
            "unknown parameter `related`; use `semantic` for task-neighbor lookup".to_string(),
        ));
    }
    for retired in [
        "field",
        "embedding_model",
        "embeddingModel",
        "embedding-model",
        "semantic_model",
        "semanticModel",
    ] {
        if input.get(retired).is_some() {
            return Err(OrbitError::InvalidInput(format!(
                "unknown parameter `{retired}`; search no longer exposes field or embedding-model selection"
            )));
        }
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
        tags: optional_string_list_alias(&input, &["tag", "tags"])?.unwrap_or_default(),
        all: optional_bool_alias(&input, &["all"])?.unwrap_or(false),
        status: optional_csv_or_string_list_alias(&input, &["status", "statuses"])?
            .unwrap_or_default(),
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

    #[test]
    fn search_tool_rejects_retired_field_and_embedding_model_params() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let field_error = search(&runtime, json!({ "query": "anything", "field": "title" }))
            .expect_err("field parameter should be retired");
        assert!(
            field_error
                .to_string()
                .contains("unknown parameter `field`")
        );

        let model_error = search(
            &runtime,
            json!({ "query": "anything", "embedding_model": "bge-small" }),
        )
        .expect_err("embedding_model parameter should be retired");
        assert!(
            model_error
                .to_string()
                .contains("unknown parameter `embedding_model`")
        );
    }

    #[test]
    fn search_tool_splits_comma_delimited_status_tokens() {
        let runtime = OrbitRuntime::in_memory().expect("build runtime");
        let error = search(
            &runtime,
            json!({ "query": "anything", "status": "task:not-a-status,doc:active" }),
        )
        .expect_err("invalid task status should be parsed out of CSV");

        assert!(error.to_string().contains("`not-a-status`"));
        assert!(error.to_string().contains("`task`"));
    }
}
