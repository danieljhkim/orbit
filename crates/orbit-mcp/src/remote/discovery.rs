//! MCP machine-local discovery definitions and execution.

use std::collections::BTreeSet;

use orbit_common::{NotFoundKind, OrbitError};
use orbit_types::tool::{
    McpToolDefinition, McpToolDefinitionError, McpToolScope, ToolParam, ToolSchema,
    validate_mcp_tool_definitions,
};
use orbit_types::workspace::{WorkspaceRegistry, WorkspaceStatus};
use serde_json::{Value, json};

pub(super) fn discovery_tool_definitions() -> Result<Vec<McpToolDefinition>, McpToolDefinitionError>
{
    let definitions = vec![workspace_list_definition(), crew_list_definition()];
    validate_mcp_tool_definitions(&definitions)?;
    Ok(definitions)
}

/// Registry-wide discovery accepts no workspace selector.
fn workspace_list_definition() -> McpToolDefinition {
    McpToolDefinition::new(
        ToolSchema {
            name: "orbit.workspace.list".to_string(),
            description: "List active workspaces with a checkout registered on this machine."
                .to_string(),
            parameters: Vec::new(),
            builtin: true,
        },
        McpToolScope::Global,
    )
}

/// Crew discovery resolves one workspace on the accepting machine.
fn crew_list_definition() -> McpToolDefinition {
    McpToolDefinition::new(
        ToolSchema {
            name: "orbit.crew.list".to_string(),
            description: "List the effective configured crews for a selected workspace on this \
                          machine."
                .to_string(),
            parameters: vec![ToolParam {
                name: "workspace".to_string(),
                description:
                    "Registered workspace name, logical workspace ID (`ws_*`), or absolute \
                              path registered on the accepting server. Defaults to the MCP session \
                              workspace and is never inferred from process cwd."
                        .to_string(),
                param_type: "string".to_string(),
                required: false,
            }],
            builtin: true,
        },
        McpToolScope::WorkspaceRequired,
    )
}

pub fn execute_discovery_tool(
    name: &str,
    registry: &WorkspaceRegistry,
    local_machine_id: &str,
) -> Result<Value, OrbitError> {
    match name {
        "orbit.workspace.list" => {
            let local_workspace_ids = registry
                .checkouts
                .iter()
                .map(|checkout| checkout.workspace_id.as_str())
                .collect::<BTreeSet<_>>();
            let workspaces = registry
                .workspaces
                .iter()
                .filter(|workspace| {
                    workspace.status == WorkspaceStatus::Active
                        && local_workspace_ids.contains(workspace.id.as_str())
                })
                .collect::<Vec<_>>();
            Ok(json!({
                "machine_id": local_machine_id,
                "workspaces": workspaces,
            }))
        }
        _ => Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string())),
    }
}
