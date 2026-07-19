//! Remote-owned global registry discovery definitions and execution.

use orbit_common::types::{
    McpToolDefinition, McpToolPlacement, McpToolPolicy, McpToolPolicyError, McpToolScope,
    NotFoundKind, OrbitError, RegistrySnapshotV1, ToolParam, ToolSchema,
    validate_mcp_tool_definitions,
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
        crew_list_definition()?,
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

/// `orbit.crew.list` completes the hub discovery surface. Unlike the two
/// registry-wide, operator-only, global tools above, it resolves a stable
/// workspace and reads that workspace owner's stored execution-profile
/// projection, so it is workspace-scoped and its accepted read-only crew
/// discovery contract permits `agent` and `operator` (never `runner`).
fn crew_list_definition() -> Result<McpToolDefinition, McpToolPolicyError> {
    McpToolDefinition::new(
        ToolSchema {
            name: "orbit.crew.list".to_string(),
            description:
                "List a workspace owner's published execution-profile crews with sanitized \
                 profile state, generation, and freshness (agent/operator, hub placement)."
                    .to_string(),
            parameters: vec![ToolParam {
                name: "workspace".to_string(),
                description:
                    "Stable logical workspace ID. Defaults to the trusted MCP session workspace; \
                     never resolved from process cwd or a checkout path."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            }],
            builtin: true,
        },
        McpToolPolicy::agent_and_operator(McpToolPlacement::Hub),
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
