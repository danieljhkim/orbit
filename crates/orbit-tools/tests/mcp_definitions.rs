//! Conformance tests for schema-adjacent builtin MCP definitions.
#![allow(missing_docs)]
#![allow(clippy::expect_used)]

use orbit_common::types::{
    McpToolPlacement, McpToolPolicy, McpToolPolicyError, McpToolScope, OrbitError, ToolSchema,
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
    assert_eq!(definitions.len(), 31);
    assert!(
        definitions
            .iter()
            .all(|definition| definition.schema.builtin)
    );
}

#[test]
fn command_exec_is_local_derived_and_operator_only() {
    let definitions =
        canonical_builtin_mcp_tool_definitions().expect("builtin MCP definitions are valid");

    let definition = definitions
        .iter()
        .find(|definition| definition.schema.name == "orbit.command.exec")
        .expect("missing orbit.command.exec definition");
    assert_eq!(
        definition.policy.placement(),
        McpToolPlacement::LocalDerived
    );
    assert_eq!(
        definition.policy.allowed_capabilities(),
        &std::collections::BTreeSet::from([orbit_common::types::McpCapability::Operator])
    );
}

#[test]
fn workflow_family_is_hub_placed_and_operator_only() {
    let definitions =
        canonical_builtin_mcp_tool_definitions().expect("builtin MCP definitions are valid");

    for name in [
        "orbit.workflow.ship",
        "orbit.workflow.run.show",
        "orbit.workflow.run.list",
        "orbit.workflow.run.resume",
    ] {
        let definition = definitions
            .iter()
            .find(|definition| definition.schema.name == name)
            .unwrap_or_else(|| panic!("missing workflow definition {name}"));
        assert_eq!(definition.policy.placement(), McpToolPlacement::Owner);
        assert_eq!(
            definition.policy.allowed_capabilities(),
            &std::collections::BTreeSet::from([orbit_common::types::McpCapability::Operator])
        );
    }
}

#[test]
fn generic_builtin_definitions_are_workspace_scoped_and_exclude_remote_discovery() {
    let definitions =
        canonical_builtin_mcp_tool_definitions().expect("builtin MCP definitions are valid");
    assert!(
        definitions
            .iter()
            .all(|definition| definition.policy.scope() == McpToolScope::WorkspaceRequired)
    );
    assert!(
        definitions.iter().all(|definition| {
            !matches!(definition.schema.name.as_str(), "orbit.workspace.list")
        })
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
        "placement": "owner",
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
    let policy = || McpToolPolicy::agent_and_operator(McpToolPlacement::Owner);

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
