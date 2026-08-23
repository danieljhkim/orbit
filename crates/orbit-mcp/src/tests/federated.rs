use super::*;

#[test]
fn host_qualified_selector_accepts_only_machine_and_workspace_ids() {
    let selector: HostQualifiedSelector = "hm_alpha-1/ws_orbit"
        .parse()
        .expect("host-qualified selector");

    assert_eq!(selector.machine_id(), "hm_alpha-1");
    assert_eq!(selector.workspace_id(), "ws_orbit");
    assert_eq!(selector.to_string(), "hm_alpha-1/ws_orbit");
}

#[test]
fn host_qualified_selector_rejects_bare_workspace_id() {
    assert!(matches!(
        "ws_orbit".parse::<HostQualifiedSelector>(),
        Err(OrbitError::UnknownSelector(_))
    ));
}

#[test]
fn host_qualified_selector_rejects_display_host_name() {
    assert!(matches!(
        "orbit-linux/ws_orbit".parse::<HostQualifiedSelector>(),
        Err(OrbitError::UnknownSelector(_))
    ));
}

#[test]
fn host_qualified_selector_rejects_paths() {
    for token in [
        "/srv/orbit/ws_orbit",
        "hm_alpha/../ws_orbit",
        "hm_alpha/C:\\orbit",
    ] {
        assert!(
            matches!(
                token.parse::<HostQualifiedSelector>(),
                Err(OrbitError::UnknownSelector(_))
            ),
            "unexpectedly accepted {token}"
        );
    }
}

#[test]
fn distinct_machine_destinations_load() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = destinations_path(root.path());
    std::fs::write(
        &path,
        r#"
[[destinations]]
ssh = "orbit-a"
machine_id = "hm_alpha"

[[destinations]]
ssh = "operator@orbit-b"
machine_id = "hm_beta"
"#,
    )
    .expect("write destinations");

    let loaded = load_destinations(&path).expect("load destinations");
    assert_eq!(loaded.destinations.len(), 2);
    assert_eq!(loaded.destinations[0].ssh, "orbit-a");
    assert_eq!(loaded.destinations[0].machine_id, "hm_alpha");
    assert_eq!(loaded.destinations[1].machine_id, "hm_beta");
}

#[test]
fn duplicate_machine_destinations_fail_at_load() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = destinations_path(root.path());
    std::fs::write(
        &path,
        r#"
[[destinations]]
ssh = "orbit-a"
machine_id = "hm_duplicate"

[[destinations]]
ssh = "orbit-b"
machine_id = "hm_duplicate"
"#,
    )
    .expect("write destinations");

    let error = load_destinations(&path).expect_err("duplicate must fail at config load");
    assert!(matches!(error, OrbitError::AmbiguousDestination(_)));
}

#[test]
fn destinations_require_machine_id() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = destinations_path(root.path());
    std::fs::write(
        &path,
        r#"
[[destinations]]
ssh = "orbit-a"
"#,
    )
    .expect("write destinations");

    let error = load_destinations(&path).expect_err("machine_id is required");
    assert!(matches!(error, OrbitError::InvalidInput(_)));
}

/// The behavior rule from the federated spec, locked tool by tool over the
/// whole advertised surface [ORB-11012].
const ADVERTISED_TOOL_CLASSES: &[(&str, McpToolClass)] = &[
    ("orbit.auto_task.list", McpToolClass::ControlPlane),
    ("orbit.auto_task.mint", McpToolClass::ControlPlane),
    ("orbit.command.exec", McpToolClass::Execute),
    ("orbit.crew.list", McpToolClass::Unclassified),
    ("orbit.friction.add", McpToolClass::ControlPlane),
    ("orbit.friction.list", McpToolClass::ControlPlane),
    ("orbit.friction.update", McpToolClass::ControlPlane),
    ("orbit.search", McpToolClass::ControlPlane),
    ("orbit.session_log.append", McpToolClass::Execute),
    ("orbit.session_log.list", McpToolClass::Execute),
    ("orbit.session_log.resolve", McpToolClass::Execute),
    ("orbit.task.add", McpToolClass::ControlPlane),
    ("orbit.task.approve", McpToolClass::ControlPlane),
    ("orbit.task.artifact.put", McpToolClass::ControlPlane),
    ("orbit.task.list", McpToolClass::ControlPlane),
    ("orbit.task.show", McpToolClass::ControlPlane),
    ("orbit.task.start", McpToolClass::ControlPlane),
    ("orbit.task.update", McpToolClass::ControlPlane),
    ("orbit.workflow.run.list", McpToolClass::Execute),
    ("orbit.workflow.run.resume", McpToolClass::Execute),
    ("orbit.workflow.run.show", McpToolClass::Execute),
    ("orbit.workflow.ship", McpToolClass::ControlPlane),
    ("orbit.workspace.list", McpToolClass::Unclassified),
];

#[test]
fn every_advertised_tool_carries_its_locked_capability_class() {
    for (name, expected) in ADVERTISED_TOOL_CLASSES {
        assert_eq!(mcp_tool_class(name), *expected, "{name}");
    }
}

/// The classifier is a function over the live surface, so a tool added to the
/// registry without a class assignment fails here instead of silently becoming
/// unclassified and unrefusable.
#[test]
fn the_locked_mapping_covers_exactly_the_advertised_surface() {
    let advertised = crate::canonical_mcp_tool_definitions()
        .expect("canonical MCP definitions")
        .into_iter()
        .map(|definition| definition.schema.name)
        .collect::<std::collections::BTreeSet<_>>();
    let locked = ADVERTISED_TOOL_CLASSES
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect::<std::collections::BTreeSet<_>>();

    assert_eq!(advertised, locked);
    assert_eq!(advertised.len(), 23);
}

#[test]
fn classification_accepts_the_advertised_spelling() {
    assert_eq!(mcp_tool_class("orbit_task_add"), McpToolClass::ControlPlane);
    assert_eq!(
        mcp_tool_class("orbit_workflow_run_show"),
        McpToolClass::Execute
    );
}

#[test]
fn a_replica_refuses_control_plane_and_runs_execute_class_tools() {
    let held = CapabilityClasses::for_checkout(
        &workspace_record(Some("hm_owner")),
        &checkout_record(Some(WorkspaceCheckoutRole::Replica)),
    );

    let refused = ensure_tool_class_held("orbit.task.add", held)
        .expect_err("a replica is not the control plane");
    assert!(
        matches!(&refused, OrbitError::CapabilityRefused(message) if message.contains("control_plane")),
        "{refused}"
    );

    for allowed in ["orbit.workflow.run.show", "orbit.command.exec"] {
        ensure_tool_class_held(allowed, held).expect(allowed);
    }
}

#[test]
fn an_owner_checkout_holds_the_control_plane() {
    let held = CapabilityClasses::for_checkout(
        &workspace_record(Some("hm_owner")),
        &checkout_record(Some(WorkspaceCheckoutRole::Owner)),
    );

    ensure_tool_class_held("orbit.task.add", held).expect("owner holds control_plane");
    ensure_tool_class_held("orbit.command.exec", held).expect("owner runs work locally today");
}

/// A standalone registry predating host identity is not a control-plane
/// authority, whatever role its checkout claims.
#[test]
fn a_workspace_without_an_owner_machine_id_does_not_advertise_control_plane() {
    for role in [None, Some(WorkspaceCheckoutRole::Owner)] {
        let held = CapabilityClasses::for_checkout(&workspace_record(None), &checkout_record(role));

        assert!(!held.holds(McpToolClass::ControlPlane), "{role:?}");
        assert!(held.holds(McpToolClass::Execute), "{role:?}");
        assert!(
            ensure_tool_class_held("orbit.task.update", held).is_err(),
            "{role:?}"
        );
    }
}

/// A legacy checkout with no recorded role is the standalone owner shape that
/// registry validation canonicalizes to `owner`.
#[test]
fn an_absent_checkout_role_is_treated_as_owner() {
    let held = CapabilityClasses::for_checkout(
        &workspace_record(Some("hm_owner")),
        &checkout_record(None),
    );

    assert!(held.holds(McpToolClass::ControlPlane));
}

/// No such checkout exists today, but the class is what refuses, not the role,
/// so a future control-plane-only store refuses execute-class work.
#[test]
fn a_control_plane_that_does_not_run_work_refuses_execute_class_tools() {
    let held = CapabilityClasses::new(true, false);

    let refused =
        ensure_tool_class_held("orbit.workflow.run.resume", held).expect_err("execute is not held");
    assert!(
        matches!(&refused, OrbitError::CapabilityRefused(message) if message.contains("execute")),
        "{refused}"
    );
    ensure_tool_class_held("orbit.task.add", held).expect("control_plane is held");
}

#[test]
fn unclassified_discovery_tools_are_never_refused() {
    for held in [
        CapabilityClasses::new(false, false),
        CapabilityClasses::new(true, true),
    ] {
        for discovery in ["orbit.workspace.list", "orbit.crew.list"] {
            ensure_tool_class_held(discovery, held).expect(discovery);
        }
    }
}

fn workspace_record(owner_machine_id: Option<&str>) -> Workspace {
    let now = chrono::Utc::now();
    Workspace {
        id: "ws_orbit".to_string(),
        name: "orbit".to_string(),
        owner_machine_id: owner_machine_id.map(str::to_string),
        git_remote: None,
        ship_mode: None,
        base_branch: "main".to_string(),
        status: orbit_types::workspace::WorkspaceStatus::Active,
        created_at: now,
        updated_at: now,
    }
}

fn checkout_record(role: Option<WorkspaceCheckoutRole>) -> WorkspaceCheckout {
    WorkspaceCheckout {
        workspace_id: "ws_orbit".to_string(),
        repo_root: PathBuf::from("/srv/orbit"),
        orbit_dir: PathBuf::from("/srv/orbit/.orbit"),
        role,
        owner_machine_id: role
            .filter(|role| *role == WorkspaceCheckoutRole::Replica)
            .map(|_| "hm_owner".to_string()),
        path_overrides: Vec::new(),
    }
}
