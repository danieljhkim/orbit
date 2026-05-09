//! Thin adapter that dispatches model tool calls through the canonical
//! `orbit_tools::ToolRegistry`.
//!
//! The loop deliberately does not implement its own tool registry. Tool
//! invocations originating from the model are routed through the same
//! `ToolRegistry::execute` entry point that the rest of Orbit uses, so tool
//! behavior, policy, and attribution stay in a single source of truth.

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
        if let Some(schema) = registry.get_schema(entry) {
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

const TASK_TYPE_ENUM: &[&str] = &[
    "task", "feature", "epic", "friction", "issue", "bug", "chore", "refactor",
];

const TASK_STATUS_ENUM: &[&str] = &[
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
        ("orbit.task.add" | "orbit.task.update", "status") => Some(TASK_STATUS_ENUM),
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

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_common::types::ToolParam;

    fn param_with_type(name: &str, param_type: &str) -> ToolParam {
        ToolParam {
            name: name.to_string(),
            description: String::new(),
            param_type: param_type.to_string(),
            required: false,
        }
    }

    fn param(name: &str) -> ToolParam {
        param_with_type(name, "string")
    }

    #[test]
    fn task_tool_specs_preserve_friction_enums() {
        let add_schema = ToolSchema {
            name: "orbit.task.add".to_string(),
            description: String::new(),
            parameters: vec![param("type"), param("status")],
            builtin: true,
        };
        let add_spec = schema_to_tool_spec(&add_schema);
        let add_properties = add_spec.input_schema["properties"]
            .as_object()
            .expect("properties");
        assert!(
            add_properties["type"]["enum"]
                .as_array()
                .expect("type enum")
                .iter()
                .any(|value| value == "friction")
        );
        assert!(
            add_properties["status"]["enum"]
                .as_array()
                .expect("status enum")
                .iter()
                .any(|value| value == "friction")
        );

        let update_schema = ToolSchema {
            name: "orbit.task.update".to_string(),
            description: String::new(),
            parameters: vec![param("status")],
            builtin: true,
        };
        let update_spec = schema_to_tool_spec(&update_schema);
        assert!(
            update_spec.input_schema["properties"]["status"]["enum"]
                .as_array()
                .expect("update status enum")
                .iter()
                .any(|value| value == "friction")
        );
    }

    #[test]
    fn task_tool_specs_advertise_dependencies_as_string_list() {
        for tool_name in ["orbit.task.add", "orbit.task.update"] {
            let schema = ToolSchema {
                name: tool_name.to_string(),
                description: String::new(),
                parameters: vec![param_with_type("dependencies", "string_list")],
                builtin: true,
            };
            let spec = schema_to_tool_spec(&schema);
            let any_of = spec.input_schema["properties"]["dependencies"]["anyOf"]
                .as_array()
                .expect("string-list union");

            assert!(
                any_of.iter().any(|schema| {
                    schema["type"] == "array" && schema["items"]["type"] == "string"
                }),
                "{tool_name} dependencies must accept an array of strings"
            );
            assert!(
                any_of.iter().any(|schema| schema["type"] == "string"),
                "{tool_name} dependencies must accept a string"
            );
        }
    }

    #[test]
    fn build_tool_specs_expands_wildcards_to_registered_tools_only() {
        let mut registry = ToolRegistry::new();
        registry.register_builtins();
        assert!(registry.unregister("orbit.task.search"));

        let specs = build_tool_specs(&registry, &["orbit.task.*".to_string()]);
        let names = specs.into_iter().map(|spec| spec.name).collect::<Vec<_>>();

        assert!(names.iter().any(|name| name == "orbit.task.show"));
        assert!(!names.iter().any(|name| name == "orbit.task.search"));
        assert!(!names.iter().any(|name| name == "orbit.task.*"));
    }

    #[test]
    fn build_tool_specs_does_not_expand_unvalidated_wildcard_roots() {
        let mut registry = ToolRegistry::new();
        registry.register_builtins();

        let specs = build_tool_specs(&registry, &["orbit.*".to_string()]);

        assert!(specs.is_empty());
    }
}
