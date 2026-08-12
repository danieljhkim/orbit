use std::collections::BTreeSet;

use chrono::Utc;
use orbit_common::types::{
    McpCapability, McpToolPlacement, McpToolScope, NotFoundKind, OrbitError, Workspace,
    WorkspaceRegistry, WorkspaceStatus,
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
        "List the workspaces this machine owns from its local workspace registry \
         (operator, local-derived placement)."
    );
    // ORB-10727: the registry-wide tool stays operator-only and global, and is
    // now `local-derived` — it reads the machine-local registry, so it never
    // routes. It still does not treat capability as a hierarchy the crew tool
    // extends.
    for definition in &definitions[..1] {
        assert!(definition.schema.builtin);
        assert!(definition.schema.parameters.is_empty());
        assert_eq!(
            definition.policy.placement(),
            McpToolPlacement::LocalDerived
        );
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
    assert_eq!(crew.policy.placement(), McpToolPlacement::Owner);
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
    assert_eq!(canonical.len(), 29, "the frozen production surface changed");
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
fn discovery_handlers_project_only_locally_owned_workspaces() {
    let now = Utc::now();
    let workspace = |id: &str, owner: &str| Workspace {
        id: id.to_string(),
        name: id.to_string(),
        owner_machine_id: Some(owner.to_string()),
        git_remote: None,
        ship_mode: None,
        base_branch: "main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: now,
        updated_at: now,
    };
    let registry = WorkspaceRegistry {
        workspaces: vec![
            workspace("ws_local", "hm_local"),
            workspace("ws_remote", "hm_remote"),
        ],
        ..WorkspaceRegistry::default()
    };

    let listed = execute_discovery_tool("orbit.workspace.list", &registry, "hm_local")
        .expect("workspace projection");
    assert_eq!(listed["machine_id"], "hm_local");
    assert_eq!(listed["workspaces"].as_array().expect("rows").len(), 1);
    assert_eq!(listed["workspaces"][0]["id"], "ws_local");
    assert!(matches!(
        execute_discovery_tool("orbit.host.future", &registry, "hm_local"),
        Err(OrbitError::NotFound {
            kind: NotFoundKind::Tool,
            ..
        })
    ));
}
