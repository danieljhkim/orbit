use std::sync::Arc;

use orbit_common::types::{ToolParam, ToolSchema};
use rmcp::model::{JsonObject, Tool};
use serde_json::{Map, Value, json};

use super::name_map::sanitize_tool_name;

pub(super) fn schema_to_tool(schema: ToolSchema) -> Tool {
    let description = schema.description.clone();
    let input_schema = build_input_schema(&schema.name, &schema.parameters);
    let advertised_name = sanitize_tool_name(&schema.name);
    Tool::new(advertised_name, description, Arc::new(input_schema))
}

pub(super) fn build_input_schema(tool_name: &str, params: &[ToolParam]) -> JsonObject {
    let mut properties = Map::new();
    let mut required: Vec<Value> = Vec::new();

    for param in params {
        let mut prop = property_for(&param.param_type);
        if let Some(values) = enum_values_for(tool_name, &param.name) {
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

const TASK_TYPE_ENUM: &[&str] = &["feature", "bug", "refactor", "chore"];

const TASK_ADD_STATUS_ENUM: &[&str] = &[
    "proposed",
    "backlog",
    "someday",
    "in-progress",
    "review",
    "done",
    "blocked",
    "rejected",
];

const TASK_UPDATE_STATUS_ENUM: &[&str] = &[
    "proposed",
    "friction",
    "backlog",
    "someday",
    "in-progress",
    "review",
    "done",
    "blocked",
    "rejected",
];

pub(super) fn enum_values_for(
    tool_name: &str,
    param_name: &str,
) -> Option<&'static [&'static str]> {
    match (tool_name, param_name) {
        ("orbit.task.add", "type") => Some(TASK_TYPE_ENUM),
        ("orbit.task.update", "type") => Some(TASK_TYPE_ENUM),
        ("orbit.task.add", "status") => Some(TASK_ADD_STATUS_ENUM),
        ("orbit.task.update", "status") => Some(TASK_UPDATE_STATUS_ENUM),
        _ => None,
    }
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
