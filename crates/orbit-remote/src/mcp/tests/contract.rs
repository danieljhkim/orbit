#![allow(missing_docs)]

use orbit_common::types::{
    McpCapability, McpToolDefinition, McpToolPlacement, McpToolPolicy, ToolParam, ToolSchema,
};

use super::super::contract::{canonical_hub_schema_bytes, hub_schema_digest};

fn fixture_definitions() -> Vec<McpToolDefinition> {
    vec![
        McpToolDefinition::new(
            ToolSchema {
                name: "orbit.task.show".to_string(),
                description: "Show one task".to_string(),
                parameters: vec![ToolParam {
                    name: "id".to_string(),
                    description: "Task ID".to_string(),
                    param_type: "string".to_string(),
                    required: true,
                }],
                builtin: true,
            },
            McpToolPolicy::agent_and_operator(McpToolPlacement::Hub),
        )
        .expect("definition"),
    ]
}

#[test]
fn frozen_hub_schema_golden_vector() {
    let bytes = canonical_hub_schema_bytes(&fixture_definitions(), McpCapability::Agent)
        .expect("canonical bytes");
    assert_eq!(
        String::from_utf8(bytes).expect("utf8"),
        concat!(
            "orbit.mcp.hub-schema.v1\0",
            r#"{"canonical_registry_revision":1,"capability":"agent","tools":[{"advertised_name":"orbit_task_show","canonical_name":"orbit.task.show","description":"Show one task","input_schema":{"additionalProperties":true,"properties":{"id":{"description":"Task ID","type":"string"}},"required":["id"],"type":"object"}}]}"#,
        )
    );
    assert_eq!(
        hub_schema_digest(&fixture_definitions(), McpCapability::Agent).expect("digest"),
        "ec8ef56c153562d0f4125cee1b3932c33ed30eb8509601aa5652a351a7b6a8f7"
    );
}
