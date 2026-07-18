//! Conformance tests for schema-adjacent builtin MCP definitions.
#![allow(missing_docs)]
#![allow(clippy::expect_used)]

use orbit_common::types::{
    McpToolPlacement, McpToolPolicy, McpToolPolicyError, OrbitError, ToolSchema,
};
use orbit_tools::{Tool, ToolContext, ToolRegistry, canonical_builtin_mcp_tool_definitions};
use serde_json::Value;

struct TestTool(&'static str);

impl Tool for TestTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.0.to_string(),
            description: "test tool".to_string(),
            parameters: Vec::new(),
            builtin: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, _input: Value) -> Result<Value, OrbitError> {
        Ok(Value::Null)
    }
}

#[test]
fn canonical_builtin_definitions_are_workspace_independent() {
    let definitions =
        canonical_builtin_mcp_tool_definitions().expect("builtin MCP definitions are valid");
    assert_eq!(definitions.len(), 27);
    assert!(
        definitions
            .iter()
            .all(|definition| definition.schema.builtin)
    );
}

#[test]
fn missing_and_invalid_policy_fail_closed() {
    let mut missing = ToolRegistry::new();
    missing.register(TestTool("demo.missing"));
    assert!(
        missing
            .mcp_tool_definitions()
            .expect("unclassified tools are valid but unexposed")
            .is_empty()
    );

    let invalid = serde_json::from_value::<McpToolPolicy>(serde_json::json!({
        "placement": "hub",
        "allowed_capabilities": []
    }))
    .expect("deserialize invalid policy for validation coverage");
    let mut registry = ToolRegistry::new();
    registry.register_mcp(TestTool("demo.invalid"), invalid);
    assert_eq!(
        registry.mcp_tool_definitions(),
        Err(McpToolPolicyError::EmptyCapabilities)
    );
}

#[test]
fn duplicate_canonical_and_advertised_names_fail_closed() {
    let policy = || McpToolPolicy::agent_and_operator(McpToolPlacement::Hub);

    let mut canonical = ToolRegistry::new();
    canonical.register_mcp(TestTool("demo.same"), policy());
    canonical.register_mcp(TestTool("demo.same"), policy());
    assert!(matches!(
        canonical.mcp_tool_definitions(),
        Err(McpToolPolicyError::DuplicateCanonicalName(_))
    ));

    let mut advertised = ToolRegistry::new();
    advertised.register_mcp(TestTool("demo.name"), policy());
    advertised.register_mcp(TestTool("demo_name"), policy());
    assert!(matches!(
        advertised.mcp_tool_definitions(),
        Err(McpToolPolicyError::DuplicateAdvertisedName(_))
    ));
}
