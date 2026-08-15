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
    tool_input_schema,
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
    ToolSpec {
        name: schema.name.clone(),
        description: schema.description.clone(),
        input_schema: Value::Object(tool_input_schema(schema)),
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
