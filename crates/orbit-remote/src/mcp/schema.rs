//! Remote-owned schema-adjacent enum metadata.

use orbit_common::types::{McpToolDefinition, OrbitError};
use orbit_mcp::{McpInputSchema, McpInputSchemaResolver, encode_mcp_input_schema_with_enum_values};

const TASK_TYPE_ENUM: &[&str] = &["feature", "bug", "refactor", "chore"];
const TASK_UPDATE_STATUS_ENUM: &[&str] = &[
    "proposed",
    "backlog",
    "someday",
    "in-progress",
    "review",
    "done",
    "blocked",
    "rejected",
];
const TASK_COMPLEXITY_ENUM: &[&str] = &["low", "medium", "hard"];
const AGENT_FAMILY_ENUM: &[&str] = &["codex", "claude", "gemini", "grok"];
const GRAPH_SEARCH_KIND_ENUM: &[&str] = &["symbol", "string", "config"];
const GRAPH_REF_CONFIDENCE_ENUM: &[&str] = &["exact", "import", "same_module", "fuzzy"];
const GRAPH_REF_KIND_ENUM: &[&str] = &[
    "call",
    "type",
    "use",
    "trait_bound",
    "impl",
    "extends",
    "implements",
];

pub(super) fn enum_values_for(
    tool_name: &str,
    param_name: &str,
) -> Option<&'static [&'static str]> {
    match (tool_name, param_name) {
        ("orbit.task.add", "type") | ("orbit.task.update", "type") => Some(TASK_TYPE_ENUM),
        ("orbit.task.update", "status") => Some(TASK_UPDATE_STATUS_ENUM),
        ("orbit.task.add", "complexity") => Some(TASK_COMPLEXITY_ENUM),
        ("orbit.graph.search", "kind") => Some(GRAPH_SEARCH_KIND_ENUM),
        ("orbit.graph.refs", "confidence") => Some(GRAPH_REF_CONFIDENCE_ENUM),
        ("orbit.graph.refs", "kind") => Some(GRAPH_REF_KIND_ENUM),
        (_, "model") => Some(AGENT_FAMILY_ENUM),
        _ => None,
    }
}

pub(super) fn remote_input_schema(
    definition: &McpToolDefinition,
) -> Result<McpInputSchema, OrbitError> {
    Ok(encode_mcp_input_schema_with_enum_values(
        &definition.schema.name,
        &definition.schema.parameters,
        enum_values_for,
    ))
}

#[derive(Debug, Default)]
pub(super) struct RemoteInputSchemaResolver;

impl McpInputSchemaResolver for RemoteInputSchemaResolver {
    fn input_schema(&self, definition: &McpToolDefinition) -> Result<McpInputSchema, OrbitError> {
        remote_input_schema(definition)
    }
}
