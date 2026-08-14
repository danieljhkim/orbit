use std::sync::Arc;

use orbit_common::types::{McpToolDefinition, McpToolScope, ToolParam, ToolSchema};
use rmcp::model::{JsonObject, Tool};
use serde_json::{Map, Value, json};

use super::name_map::sanitize_tool_name;

pub(super) fn schema_to_tool(schema: ToolSchema, input_schema: JsonObject) -> Tool {
    let description = schema.description.clone();
    let advertised_name = sanitize_tool_name(&schema.name);
    Tool::new(advertised_name, description, Arc::new(input_schema))
}

/// Canonical name of the broker's workspace-routing argument.
pub(crate) const WORKSPACE_SELECTOR_PARAM: &str = "workspace";

const WORKSPACE_SELECTOR_DESCRIPTION: &str = "Workspace selector for broker routing: a registered workspace name, a logical workspace \
     ID (`ws_*`), or an absolute path to a local checkout (a linked Git worktree resolves to \
     its registered checkout). Optional when the MCP session announced `_meta.orbit.workspace` \
     at initialize; never inferred from process cwd.";

/// Advertise the workspace selector on every workspace-scoped tool.
///
/// The broker rejects a [`McpToolScope::WorkspaceRequired`] call that carries
/// no selector, taking it from either the `workspace` argument or the trusted
/// session context announced through initialize `_meta`. General-purpose MCP
/// clients cannot inject custom initialize metadata, so the argument is the
/// only selector a managed executor can actually supply — and a tool that
/// requires it without advertising it is uncallable from a schema-following
/// caller (F2026-07-099, ORB-10448). Tools that already declare their own
/// `workspace` parameter keep their own description.
pub(super) fn ensure_workspace_selector(schema: &mut JsonObject, definition: &McpToolDefinition) {
    if definition.policy.scope() != McpToolScope::WorkspaceRequired {
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
    build_input_schema_with_enum_values(tool_name, params, no_enum_values)
}

fn no_enum_values(_tool_name: &str, _param_name: &str) -> Option<&'static [&'static str]> {
    None
}

pub(crate) fn build_input_schema_with_enum_values<F>(
    tool_name: &str,
    params: &[ToolParam],
    enum_values: F,
) -> JsonObject
where
    F: Fn(&str, &str) -> Option<&'static [&'static str]>,
{
    let mut properties = Map::new();
    let mut required: Vec<Value> = Vec::new();

    for param in params {
        let mut prop = property_for(&param.param_type);
        if let Some(values) = enum_values(tool_name, &param.name) {
            prop.insert(
                "enum".to_string(),
                Value::Array(
                    values
                        .iter()
                        .map(|value| Value::String((*value).to_string()))
                        .collect(),
                ),
            );
        }
        if !param.description.is_empty() {
            prop.insert(
                "description".to_string(),
                Value::String(param.description.clone()),
            );
        }
        properties.insert(param.name.clone(), Value::Object(prop));

        if param.required {
            required.push(Value::String(param.name.clone()));
        }
    }

    let mut schema = Map::new();
    schema.insert("type".to_string(), Value::String("object".to_string()));
    schema.insert("properties".to_string(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".to_string(), Value::Array(required));
    }
    // Orbit tools accept identity aliases (`agent`, `model`) and other
    // convenience kwargs not enumerated in their static param list. Permit
    // extra properties so MCP clients aren't blocked by a client-side
    // schema validator.
    schema.insert("additionalProperties".to_string(), Value::Bool(true));
    schema
}

/// Build the JSON-Schema fragment for a single parameter.
///
/// String-list and object-map parameters are emitted as `anyOf` unions because
/// Orbit tool input handlers normalize those specific shapes. Generic arrays
/// stay arrays so arrays of objects are not advertised as string lists.
pub(super) fn property_for(param_type: &str) -> Map<String, Value> {
    let mut m = Map::new();
    let key = param_type.trim().to_ascii_lowercase();
    match key.as_str() {
        "string" | "text" | "enum" => {
            m.insert("type".to_string(), Value::String("string".to_string()));
        }
        "integer" | "int" => {
            m.insert("type".to_string(), Value::String("integer".to_string()));
        }
        "number" | "float" => {
            m.insert("type".to_string(), Value::String("number".to_string()));
        }
        "boolean" | "bool" => {
            m.insert("type".to_string(), Value::String("boolean".to_string()));
        }
        "string_list" | "string[]" | "strings" => {
            m.insert(
                "anyOf".to_string(),
                json!([
                    { "type": "array", "items": { "type": "string" } },
                    { "type": "string" },
                ]),
            );
        }
        "array" | "list" => {
            m.insert("type".to_string(), Value::String("array".to_string()));
        }
        "object" | "map" | "json" => {
            m.insert(
                "anyOf".to_string(),
                json!([
                    { "type": "object" },
                    { "type": "array", "items": { "type": "object" } },
                ]),
            );
        }
        "object_list" | "object[]" | "objects" => {
            m.insert(
                "anyOf".to_string(),
                json!([
                    { "type": "array", "items": { "type": "object" } },
                    { "type": "string" },
                ]),
            );
        }
        _ => {
            tracing::warn!(
                target: "orbit.mcp.adapter",
                param_type = %param_type,
                "unknown ToolParam type degrading to string"
            );
            m.insert("type".to_string(), Value::String("string".to_string()));
        }
    }
    m
}
