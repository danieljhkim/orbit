use std::sync::Arc;

use orbit_common::protocol::tool_schema::tool_input_schema_for;
#[cfg(test)]
use orbit_common::protocol::tool_schema::tool_parameter_schema;
use orbit_types::tool::{McpToolDefinition, McpToolScope, ToolParam, ToolSchema};
use rmcp::model::{JsonObject, Tool};
use serde_json::{Value, json};

use super::name_map::sanitize_tool_name;

pub(super) fn schema_to_tool(schema: ToolSchema, input_schema: JsonObject) -> Tool {
    let description = schema.description.clone();
    let advertised_name = sanitize_tool_name(&schema.name);
    Tool::new(advertised_name, description, Arc::new(input_schema))
}

/// Canonical name of the authoritative server's workspace selector.
pub(crate) const WORKSPACE_SELECTOR_PARAM: &str = "workspace";

const WORKSPACE_SELECTOR_DESCRIPTION: &str = "Workspace selector for the authoritative server: a registered workspace name, a logical \
     workspace ID (`ws_*`), or an absolute path registered on that server. Optional when the \
     MCP session announced `_meta.orbit.workspace` at initialize; never inferred from the \
     server process cwd.";

/// Advertise the workspace selector on every workspace-scoped tool.
pub(super) fn ensure_workspace_selector(schema: &mut JsonObject, definition: &McpToolDefinition) {
    if definition.scope != McpToolScope::WorkspaceRequired {
        return;
    }
    // `orbit.task.show` still opens a workspace runtime, but `id` is globally
    // resolved by default [ORB-10961]. The generic selector text would make
    // clients inject cwd, initialize metadata, or a linked-worktree runtime
    // identity. The tool declares its own optional filter instead.
    if definition.schema.name == "orbit.task.show" {
        return;
    }
    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };
    if properties.contains_key(WORKSPACE_SELECTOR_PARAM) {
        return;
    }
    properties.insert(
        WORKSPACE_SELECTOR_PARAM.to_string(),
        json!({
            "type": "string",
            "description": WORKSPACE_SELECTOR_DESCRIPTION,
        }),
    );
}

pub(crate) fn build_input_schema(tool_name: &str, params: &[ToolParam]) -> JsonObject {
    tool_input_schema_for(tool_name, params)
}

#[cfg(test)]
pub(super) fn property_for(param_type: &str) -> JsonObject {
    tool_parameter_schema(param_type)
}
