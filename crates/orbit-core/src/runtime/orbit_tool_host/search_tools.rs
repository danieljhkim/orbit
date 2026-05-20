use std::str::FromStr;

use orbit_common::types::{OrbitError, optional_string_alias, optional_u32_alias};
use serde_json::Value;

use crate::{GlobalSearchKind, GlobalSearchParams, OrbitRuntime};

use super::input::optional_bool_alias;

pub(super) fn search(runtime: &OrbitRuntime, input: Value) -> Result<Value, OrbitError> {
    let related = optional_string_alias(&input, &["related", "id", "task_id", "taskId"])?;
    let semantic =
        optional_bool_alias(&input, &["semantic"])?.unwrap_or(false) || related.is_some();
    let kind = optional_string_alias(&input, &["kind"])?
        .map(|kind| GlobalSearchKind::from_str(&kind).map_err(OrbitError::InvalidInput))
        .transpose()?
        .unwrap_or_default();

    let result = runtime.global_search(GlobalSearchParams {
        query: optional_string_alias(&input, &["query"])?,
        semantic,
        related,
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
    })?;
    serde_json::to_value(result)
        .map_err(|error| OrbitError::Execution(format!("serialize search result: {error}")))
}
