//! Remote-owned global registry discovery definitions and execution.

use orbit_common::types::{
    McpToolDefinition, McpToolPlacement, McpToolPolicy, McpToolPolicyError, McpToolScope,
    NotFoundKind, OrbitError, RegistrySnapshotV1, ToolSchema, validate_mcp_tool_definitions,
};
use serde_json::{Value, json};

pub(super) fn discovery_tool_definitions() -> Result<Vec<McpToolDefinition>, McpToolPolicyError> {
    let definitions = vec![
        discovery_definition(
            "orbit.host.list",
            "List registered hub hosts with sanitized lifecycle, aliases, and workspace-presence \
             freshness (operator, hub placement).",
        )?,
        discovery_definition(
            "orbit.workspace.list",
            "List workspaces with declared owner and sanitized execution-profile freshness \
             (operator, hub placement).",
        )?,
    ];
    validate_mcp_tool_definitions(&definitions)?;
    Ok(definitions)
}

fn discovery_definition(
    name: &str,
    description: &str,
) -> Result<McpToolDefinition, McpToolPolicyError> {
    McpToolDefinition::new(
        ToolSchema {
            name: name.to_string(),
            description: description.to_string(),
            // Registry discovery is workspace-unscoped. Its sanitized snapshot
            // projection accepts no workspace selector or checkout path.
            parameters: Vec::new(),
            builtin: true,
        },
        McpToolPolicy::operator_only(McpToolPlacement::Hub).with_scope(McpToolScope::Global),
    )
}

pub(super) fn execute_discovery_tool(
    name: &str,
    snapshot: RegistrySnapshotV1,
) -> Result<Value, OrbitError> {
    match name {
        "orbit.host.list" => Ok(json!({
            "hub_machine_id": snapshot.hub_machine_id,
            "registry_revision": snapshot.registry_revision,
            "hosts": snapshot.hosts,
        })),
        "orbit.workspace.list" => Ok(json!({
            "hub_machine_id": snapshot.hub_machine_id,
            "registry_revision": snapshot.registry_revision,
            "workspaces": snapshot.workspaces,
        })),
        _ => Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string())),
    }
}
