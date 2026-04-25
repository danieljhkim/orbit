//! Thin adapter that dispatches model tool calls through the canonical
//! `orbit_tools::ToolRegistry`.
//!
//! The loop deliberately does not implement its own tool registry. Tool
//! invocations originating from the model are routed through the same
//! `ToolRegistry::execute` entry point that the rest of Orbit uses, so tool
//! behavior, policy, and attribution stay in a single source of truth.

use std::time::Instant;

use orbit_common::types::{OrbitError, ToolSchema};
use orbit_tools::{ToolContext, ToolRegistry};
use serde_json::{Value, json};

use super::transport::ToolSpec;

pub fn build_tool_specs(registry: &ToolRegistry, allowlist: &[String]) -> Vec<ToolSpec> {
    allowlist
        .iter()
        .filter_map(|name| {
            registry
                .get_schema(name)
                .map(|schema| schema_to_tool_spec(&schema))
        })
        .collect()
}

pub fn schema_to_tool_spec(schema: &ToolSchema) -> ToolSpec {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for param in &schema.parameters {
        let mut property = schema_for_param_type(&param.param_type);
        property
            .as_object_mut()
            .expect("parameter schema")
            .insert("description".to_string(), json!(param.description.clone()));
        properties.insert(param.name.clone(), property);
        if param.required {
            required.push(param.name.clone());
        }
    }
    let mut input_schema = json!({
        "type": "object",
        "properties": Value::Object(properties),
    });
    if !required.is_empty() {
        input_schema
            .as_object_mut()
            .expect("object")
            .insert("required".to_string(), json!(required));
    }
    ToolSpec {
        name: schema.name.clone(),
        description: schema.description.clone(),
        input_schema,
    }
}

fn schema_for_param_type(raw: &str) -> Value {
    if matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "string_list" | "string[]" | "strings"
    ) {
        return json!({
            "anyOf": [
                { "type": "string" },
                { "type": "array", "items": { "type": "string" } }
            ]
        });
    }

    json!({ "type": map_param_type(raw) })
}

fn map_param_type(raw: &str) -> &'static str {
    match raw.to_ascii_lowercase().as_str() {
        "string" | "str" | "path" | "url" => "string",
        "bool" | "boolean" => "boolean",
        "u8" | "u16" | "u32" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize"
        | "integer" | "int" => "integer",
        "f32" | "f64" | "number" | "float" => "number",
        "array" | "list" => "array",
        "object" | "json" => "object",
        _ => "string",
    }
}

pub struct DispatchOutcome {
    pub output: Value,
    pub is_error: bool,
    pub duration_ms: u128,
}

pub fn dispatch(
    registry: &ToolRegistry,
    ctx: &ToolContext,
    name: &str,
    input: Value,
) -> DispatchOutcome {
    let started = Instant::now();
    let result = registry.execute(name, ctx, input);
    let duration_ms = started.elapsed().as_millis();
    match result {
        Ok(output) => DispatchOutcome {
            output,
            is_error: false,
            duration_ms,
        },
        Err(err) => DispatchOutcome {
            output: tool_error_value(&err),
            is_error: true,
            duration_ms,
        },
    }
}

fn tool_error_value(err: &OrbitError) -> Value {
    json!({
        "error": err.to_string(),
    })
}
