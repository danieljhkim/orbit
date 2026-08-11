//! Remote-owned global registry discovery definitions and execution.

use orbit_common::types::{
    McpToolDefinition, McpToolPlacement, McpToolPolicy, McpToolPolicyError, McpToolScope,
    NotFoundKind, OrbitError, RegistrySnapshotV1, ToolParam, ToolSchema,
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
            description: "List the workspaces this machine owns, with sanitized \
                          execution-profile freshness (operator, local-derived placement)."
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
    snapshot: RegistrySnapshotV1,
) -> Result<Value, OrbitError> {
    match name {
        // `hub_machine_id` is the host-registry store's own column name for the
        // machine that stamped it; on a machine's own registry that is this
        // machine. Renaming it belongs to the host-registry recomposition
        // (ADR-0358), not to the bridge, so the response key is left stable.
        "orbit.workspace.list" => Ok(json!({
            "hub_machine_id": snapshot.hub_machine_id,
            "registry_revision": snapshot.registry_revision,
            "workspaces": snapshot.workspaces,
        })),
        _ => Err(OrbitError::not_found(NotFoundKind::Tool, name.to_string())),
    }
}
