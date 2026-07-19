use chrono::{Duration, TimeZone, Utc};
use orbit_common::types::{
    REGISTRY_SNAPSHOT_SCHEMA_VERSION, RegistrySnapshotV1, Workspace, WorkspaceCheckout,
    WorkspaceRegistry, WorkspaceStatus,
};
use orbit_core::routines::{
    RoutinePlacementProvider, RoutineRegistryCacheView, RoutineRegistryView,
    RoutineWorkspaceProvider,
};
use orbit_store::sqlite::task_registry::{WorkspaceConfig, write_workspace_config};

use super::RemoteRoutineEnvironment;
use crate::RegistryCacheService;
use crate::workspace_registry;

fn write_spoke_identity(root: &std::path::Path) {
    std::fs::create_dir_all(root).expect("global root");
    std::fs::write(
        root.join("host.toml"),
        "schema_version = 1\nmachine_id = \"hm_spoke\"\nhost_id = \"spoke\"\nmode = \"spoke\"\n",
    )
    .expect("host identity");
}

fn empty_snapshot() -> RegistrySnapshotV1 {
    RegistrySnapshotV1 {
        schema_version: REGISTRY_SNAPSHOT_SCHEMA_VERSION,
        hub_machine_id: Some("hm_hub".to_string()),
        registry_revision: 1,
        hosts: Vec::new(),
        workspaces: Vec::new(),
    }
}

#[test]
fn spoke_cache_projection_preserves_strict_freshness_boundary() {
    let root = tempfile::tempdir().expect("root");
    write_spoke_identity(root.path());
    let now = Utc
        .with_ymd_and_hms(2026, 7, 18, 12, 10, 0)
        .single()
        .expect("timestamp");
    RegistryCacheService::new(root.path())
        .refresh(empty_snapshot(), now)
        .expect("cache");
    let environment = RemoteRoutineEnvironment::load(root.path()).expect("environment");

    let exact = environment
        .load_routine_placement(now + Duration::minutes(5), Duration::minutes(5))
        .expect("exact boundary");
    assert!(matches!(
        exact.registry,
        RoutineRegistryView::Spoke {
            cache: RoutineRegistryCacheView::Current { .. }
        }
    ));

    let stale = environment
        .load_routine_placement(
            now + Duration::minutes(5) + Duration::seconds(1),
            Duration::minutes(5),
        )
        .expect("stale boundary");
    assert!(matches!(
        stale.registry,
        RoutineRegistryView::Spoke {
            cache: RoutineRegistryCacheView::Stale { .. }
        }
    ));
}

#[test]
fn workspace_discovery_builds_bound_runtimes() {
    let root = tempfile::tempdir().expect("root");
    let global = root.path().join("global");
    let repo = root.path().join("repo");
    let orbit_dir = repo.join(".orbit");
    std::fs::create_dir_all(global.join("state")).expect("global");
    std::fs::create_dir_all(&orbit_dir).expect("orbit");
    std::fs::write(
        global.join("host.toml"),
        "schema_version = 1\nmachine_id = \"hm_local\"\nhost_id = \"local\"\nmode = \"standalone\"\n",
    )
    .expect("host identity");
    write_workspace_config(
        &orbit_dir,
        &WorkspaceConfig {
            schema_version: 1,
            workspace_id: "ws_runtime".to_string(),
        },
    )
    .expect("workspace config");
    let workspace = Workspace {
        id: "logical-abc123".to_string(),
        name: "orbit".to_string(),
        owner_machine_id: None,
        git_remote: None,
        ship_mode: Some("pr".to_string()),
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let registry = WorkspaceRegistry {
        workspaces: vec![workspace.clone()],
        checkouts: vec![WorkspaceCheckout::owner(
            workspace.id.clone(),
            repo,
            orbit_dir,
        )],
        ..WorkspaceRegistry::default()
    };
    workspace_registry::save_registry_to(
        &registry,
        &workspace_registry::registry_path_for(&global),
    )
    .expect("registry");

    let environment = RemoteRoutineEnvironment::load(&global).expect("environment");
    let discovered = environment.discover_workspaces(&global).expect("discovery");
    assert!(discovered.errors.is_empty());
    assert_eq!(discovered.entries.len(), 1);
    let binding = discovered.entries[0]
        .1
        .workspace_runtime_binding()
        .expect("binding");
    assert_eq!(binding.workspace_id, "ws_runtime");
    assert_eq!(binding.ship_mode.as_input_value(), "pr");
}
