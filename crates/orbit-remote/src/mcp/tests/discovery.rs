use std::collections::BTreeSet;

use orbit_common::types::{
    McpCapability, McpToolPlacement, McpToolScope, NotFoundKind, OrbitError,
    REGISTRY_SNAPSHOT_SCHEMA_VERSION, RegistrySnapshotV1,
};
use serde_json::json;

use super::super::discovery::{discovery_tool_definitions, execute_discovery_tool};
use super::super::host::canonical_mcp_tool_definitions;
use super::super::schema::remote_input_schema;

#[test]
fn remote_owns_the_exact_global_discovery_definitions() {
    let definitions = discovery_tool_definitions().expect("discovery definitions");
    assert_eq!(definitions.len(), 2);
    assert_eq!(
        definitions
            .iter()
            .map(|definition| definition.schema.name.as_str())
            .collect::<Vec<_>>(),
        ["orbit.workspace.list", "orbit.crew.list"]
    );
    assert_eq!(
        definitions[0].schema.description,
        "List workspaces with declared owner and sanitized execution-profile freshness \
         (operator, hub placement)."
    );
    // The registry-wide tool stays operator-only and global; it does not
    // treat capability as a hierarchy that the crew tool extends.
    for definition in &definitions[..1] {
        assert!(definition.schema.builtin);
        assert!(definition.schema.parameters.is_empty());
        assert_eq!(definition.policy.placement(), McpToolPlacement::Hub);
        assert_eq!(definition.policy.scope(), McpToolScope::Global);
        assert_eq!(
            definition.policy.allowed_capabilities(),
            &BTreeSet::from([McpCapability::Operator])
        );
        assert_eq!(
            remote_input_schema(definition).expect("discovery wire schema"),
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": true,
            })
            .as_object()
            .expect("object schema")
            .clone()
        );
    }

    // Crew discovery is workspace-scoped with exactly {agent, operator}; runner
    // is never in its set.
    let crew = &definitions[1];
    assert_eq!(crew.schema.name, "orbit.crew.list");
    assert!(crew.schema.builtin);
    assert_eq!(crew.policy.placement(), McpToolPlacement::Hub);
    assert_eq!(crew.policy.scope(), McpToolScope::WorkspaceRequired);
    assert_eq!(
        crew.policy.allowed_capabilities(),
        &BTreeSet::from([McpCapability::Agent, McpCapability::Operator])
    );
    assert!(
        !crew
            .policy
            .allowed_capabilities()
            .contains(&McpCapability::Runner)
    );

    let canonical = canonical_mcp_tool_definitions().expect("canonical definitions");
    assert_eq!(canonical.len(), 27, "the frozen production surface changed");
    assert_eq!(
        canonical
            .iter()
            .filter(|definition| definition.policy.scope() == McpToolScope::Global)
            .map(|definition| definition.schema.name.as_str())
            .collect::<Vec<_>>(),
        ["orbit.workspace.list"]
    );
}

#[test]
fn discovery_handlers_project_only_the_requested_snapshot_partition() {
    let snapshot = RegistrySnapshotV1 {
        schema_version: REGISTRY_SNAPSHOT_SCHEMA_VERSION,
        hub_machine_id: Some("hm_hub".to_string()),
        registry_revision: 7,
        hosts: Vec::new(),
        workspaces: Vec::new(),
    };

    assert_eq!(
        execute_discovery_tool("orbit.workspace.list", snapshot.clone())
            .expect("workspace projection"),
        json!({
            "hub_machine_id": "hm_hub",
            "registry_revision": 7,
            "workspaces": [],
        })
    );
    assert!(matches!(
        execute_discovery_tool("orbit.host.future", snapshot),
        Err(OrbitError::NotFound {
            kind: NotFoundKind::Tool,
            ..
        })
    ));
}
