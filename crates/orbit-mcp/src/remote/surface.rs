//! Canonical MCP definitions assembled from their owning registries.

use orbit_types::tool::{McpToolDefinition, McpToolDefinitionError};

pub fn canonical_mcp_tool_definitions() -> Result<Vec<McpToolDefinition>, McpToolDefinitionError> {
    let mut definitions = orbit_tools::canonical_builtin_mcp_tool_definitions()?;
    definitions.extend(super::discovery::discovery_tool_definitions()?);
    definitions.sort_by(|left, right| left.schema.name.cmp(&right.schema.name));
    orbit_types::tool::validate_mcp_tool_definitions(&definitions)?;
    Ok(definitions)
}

pub fn safe_mcp_tool_names() -> Vec<String> {
    canonical_mcp_tool_definitions()
        .map(|definitions| {
            definitions
                .into_iter()
                .map(|definition| definition.schema.name)
                .collect()
        })
        .unwrap_or_default()
}
