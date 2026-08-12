//! Remote-owned machine-local discovery definitions and execution.

use orbit_common::types::{
    McpToolDefinition, McpToolPlacement, McpToolPolicy, McpToolPolicyError, McpToolScope,
    NotFoundKind, OrbitError, ToolParam, ToolSchema, WorkspaceRegistry, WorkspaceStatus,
    validate_mcp_tool_definitions,
};
use serde_json::{Value, json};

pub(super) fn discovery_tool_definitions() -> Result<Vec<McpToolDefinition>, McpToolPolicyError> {
    let definitions = vec![workspace_list_definition()?, crew_list_definition()?];
    validate_mcp_tool_definitions(&definitions)?;
    Ok(definitions)
}

/// `orbit.workspace.list` is the one `local-derived` entry backed by
/// machine-local registry state rather than checkout-derived state, and it
/// enumerates only the workspaces this machine owns. It is registry-wide, so it
/// is `global`: it accepts no workspace selector or checkout path.
fn workspace_list_definition() -> Result<McpToolDefinition, McpToolPolicyError> {
    McpToolDefinition::new(
        ToolSchema {
            name: "orbit.workspace.list".to_string(),
            description: "List the workspaces this machine owns from its local workspace registry \
                          (operator, local-derived placement)."
                .to_string(),
            parameters: Vec::new(),
            builtin: true,
        },
        McpToolPolicy::operator_only(McpToolPlacement::LocalDerived)
            .with_scope(McpToolScope::Global),
    )
}

/// `orbit.crew.list` completes the discovery surface. Unlike the registry-wide,
/// operator-only, global tool above, it resolves a stable workspace and reads
/// that workspace owner's local crew config, so it is workspace-scoped, `owner`
/// placed, and its read-only discovery contract permits `agent` and `operator`.
fn crew_list_definition() -> Result<McpToolDefinition, McpToolPolicyError> {
    McpToolDefinition::new(
        ToolSchema {
            name: "orbit.crew.list".to_string(),
            description: "List a workspace owner's execution-profile crews with sanitized profile \
                 state, generation, and freshness (agent/operator, owner placement)."
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
        McpToolPolicy::agent_and_operator(McpToolPlacement::Owner),
    )
}

pub(super) fn execute_discovery_tool(
    name: &str,
    registry: &WorkspaceRegistry,
    local_machine_id: &str,
) -> Result<Value, OrbitError> {
    match name {
        "orbit.workspace.list" => {
            let workspaces = registry
                .workspaces
                .iter()
                .filter(|workspace| {
                    workspace.status == WorkspaceStatus::Active
                        && workspace.owner_machine_id.as_deref() == Some(local_machine_id)
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
