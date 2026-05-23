//! Thin adapter that dispatches model tool calls through the canonical
//! `orbit_tools::ToolRegistry`.
//!
//! The loop deliberately does not implement its own tool registry. Tool
//! invocations originating from the model are routed through the same
//! `ToolRegistry::execute` entry point that the rest of Orbit uses, so tool
//! behavior, policy, and attribution stay in a single source of truth.

// ORB-00013: Existing expect calls in this module document local invariants; keep the allow scoped while the workspace lint is ratcheted.
#![allow(clippy::expect_used)]

use std::collections::HashSet;
use std::time::Instant;

use orbit_common::types::{
    OrbitError, ToolSchema,
    activity_job::{tool_allowed, validate_tool_allowlist},
};
use orbit_tools::{ToolContext, ToolRegistry};
use serde_json::{Value, json};

use super::transport::ToolSpec;

pub fn build_tool_specs(registry: &ToolRegistry, allowlist: &[String]) -> Vec<ToolSpec> {
    let mut schemas = registry.schemas();
    schemas.sort_unstable_by(|left, right| left.name.cmp(&right.name));

    let mut seen = HashSet::new();
    let mut specs = Vec::new();
    for entry in allowlist {
        if let Some(schema) = registry.get_active_schema(entry) {
            push_tool_spec(&mut specs, &mut seen, &schema);
        }
        if entry.ends_with('*') && validate_tool_allowlist(std::slice::from_ref(entry)).is_ok() {
            for schema in &schemas {
                if tool_allowed(&schema.name, std::slice::from_ref(entry)) {
                    push_tool_spec(&mut specs, &mut seen, schema);
                }
            }
        }
    }
    specs
}

fn push_tool_spec(specs: &mut Vec<ToolSpec>, seen: &mut HashSet<String>, schema: &ToolSchema) {
    if seen.insert(schema.name.clone()) {
        specs.push(schema_to_tool_spec(schema));
    }
}

pub fn schema_to_tool_spec(schema: &ToolSchema) -> ToolSpec {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for param in &schema.parameters {
        let mut property = schema_for_param_type(&param.param_type);
        let property_object = property.as_object_mut().expect("parameter schema");
        if let Some(values) = enum_values_for(&schema.name, &param.name) {
            property_object.insert("enum".to_string(), json!(values));
        }
        property_object.insert("description".to_string(), json!(param.description.clone()));
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

fn enum_values_for(tool_name: &str, param_name: &str) -> Option<&'static [&'static str]> {
    match (tool_name, param_name) {
        ("orbit.task.add", "type") => Some(TASK_TYPE_ENUM),
        ("orbit.task.update", "type") => Some(TASK_TYPE_ENUM),
        ("orbit.task.add", "status") => Some(TASK_ADD_STATUS_ENUM),
        ("orbit.task.update", "status") => Some(TASK_UPDATE_STATUS_ENUM),
        _ => None,
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
