use std::collections::BTreeSet;

use chrono::{TimeZone, Utc};
use orbit_common::types::{
    HostNameResolution, HostStatus, Workspace, WorkspaceRegistry, WorkspaceStatus,
};

use crate::HOST_IDENTITY_SCHEMA_VERSION;
use crate::host_identity::{HostIdentity, HostMode};
use crate::host_registry::HostRegistryService;
use crate::persistence::RegistryStore;

fn identity(machine_id: &str, host_id: &str, mode: HostMode) -> HostIdentity {
    HostIdentity {
        schema_version: HOST_IDENTITY_SCHEMA_VERSION,
        machine_id: machine_id.to_string(),
        host_id: host_id.to_string(),
        task_prefix: "ORB".to_string(),
        mode,
    }
}

#[test]
fn hub_administration_preflight_rejects_unstamped_and_shadow_stores() {
    let service = HostRegistryService::new(RegistryStore::open_in_memory().expect("store"));
    let local = identity("hm_local", "local", HostMode::Hub);
    let unconfigured = service
        .require_configured_local_hub(&local)
        .expect_err("unstamped store must fail")
        .to_string();
    assert!(
        unconfigured.contains("no configured hub"),
        "unexpected: {unconfigured}"
    );

    service
        .register_hub_identity(
            &identity("hm_other", "other", HostMode::Hub),
            BTreeSet::new(),
        )
        .expect("configure other hub");
    let shadow = service
        .require_configured_local_hub(&local)
        .expect_err("shadow store must fail")
        .to_string();
    assert!(
        shadow.contains("shadow coordination store"),
        "unexpected: {shadow}"
    );
    assert!(shadow.contains("hm_local"), "unexpected: {shadow}");
    assert!(shadow.contains("hm_other"), "unexpected: {shadow}");
}

#[test]
fn service_registers_stable_identity_and_preserves_typed_lifecycle_results() {
    let service = HostRegistryService::new(RegistryStore::open_in_memory().expect("store"));
    let hub = identity("hm_hub", "hub", HostMode::Hub);
    let spoke = identity("hm_spoke", "spoke", HostMode::Spoke);

    let registered = service
        .register_identity(&hub, BTreeSet::from(["codex".to_string()]))
        .expect("register hub");
    assert_eq!(registered.machine_id, hub.machine_id);
    assert_eq!(registered.host_id, hub.host_id);
    assert_eq!(registered.status, HostStatus::Active);
    assert_eq!(
        service
            .register_identity(&hub, BTreeSet::from(["codex".to_string()]))
            .expect("idempotent registration"),
        registered
    );
    service
        .register_identity(&spoke, BTreeSet::new())
        .expect("register spoke");

    service.rename("hm_spoke", "worker").expect("rename");
    match service.resolve("spoke").expect("resolve alias") {
        HostNameResolution::Alias { host, alias } => {
            assert_eq!(host.host_id, "worker");
            assert_eq!(alias.alias_host_id, "spoke");
        }
        other => panic!("expected alias, got {other:?}"),
    }
    service.retire("hm_spoke").expect("retire");
    assert_eq!(
        service
            .active_hosts()
            .expect("active hosts")
            .iter()
            .map(|host| host.host_id.as_str())
            .collect::<Vec<_>>(),
        vec!["hub"]
    );
    assert_eq!(service.aliases("hm_spoke").expect("aliases").len(), 1);
}

fn workspace(id: &str, owner_machine_id: Option<&str>) -> Workspace {
    let now = Utc
        .with_ymd_and_hms(2026, 7, 18, 8, 0, 0)
        .single()
        .expect("timestamp");
    Workspace {
        id: id.to_string(),
        name: id.to_string(),
        owner_machine_id: owner_machine_id.map(ToOwned::to_owned),
        git_remote: Some("git@github.com:example/repo.git".to_string()),
        ship_mode: Some("pr".to_string()),
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: now,
        updated_at: now,
    }
}

fn workspace_registry(workspaces: Vec<Workspace>) -> WorkspaceRegistry {
    WorkspaceRegistry {
        schema_version: 1,
        owner_host_ids: Default::default(),
        workspaces,
        checkouts: Vec::new(),
    }
}

#[test]
fn service_requires_explicit_existing_workspace_and_consistent_local_owner_mirror() {
    let store = RegistryStore::open_in_memory().expect("store");
    let service = HostRegistryService::new(store);
    service
        .register_hub_identity(&identity("hm_hub", "hub", HostMode::Hub), BTreeSet::new())
        .expect("register hub");
    service
        .register_identity(
            &identity("hm_spoke", "spoke", HostMode::Spoke),
            BTreeSet::new(),
        )
        .expect("register spoke");
    let registry = workspace_registry(vec![workspace("ws_alpha", Some("hm_hub"))]);

    let missing = service
        .bind_workspace_owner(&registry, "ws_missing", "hm_hub")
        .expect_err("missing workspace fails")
        .to_string();
    assert!(missing.contains("unknown logical workspace_id"));
    let mirror = service
        .bind_workspace_owner(&registry, "ws_alpha", "hm_spoke")
        .expect_err("mirror mismatch fails")
        .to_string();
    assert!(mirror.contains("local owner mirror"));
    let bound = service
        .bind_workspace_owner(&registry, "ws_alpha", "hm_hub")
        .expect("bind owner");
    assert_eq!(bound.owner_machine_id, "hm_hub");
}

#[test]
fn link_workspace_owner_binds_active_warns_on_alias_and_rejects_bad_resolutions() {
    let store = RegistryStore::open_in_memory().expect("store");
    let service = HostRegistryService::new(store);
    service
        .register_identity(
            &identity("hm_owner", "owner", HostMode::Spoke),
            BTreeSet::new(),
        )
        .expect("register owner");
    // A tombstone alias for the owner's previous name.
    service.rename("hm_owner", "owner2").expect("rename owner");
    service
        .register_identity(
            &identity("hm_gone", "gone", HostMode::Spoke),
            BTreeSet::new(),
        )
        .expect("register gone");
    service.retire("hm_gone").expect("retire gone");
    let registry = workspace_registry(vec![
        workspace("ws_active", None),
        workspace("ws_alias", None),
    ]);

    // Active name binds with no warning.
    let link = service
        .link_workspace_owner(&registry, "ws_active", "owner2")
        .expect("link active");
    assert_eq!(link.ownership.owner_machine_id, "hm_owner");
    assert!(link.warning.is_none());

    // Tombstone alias resolves to the active owner but warns.
    let aliased = service
        .link_workspace_owner(&registry, "ws_alias", "owner")
        .expect("link via alias");
    assert_eq!(aliased.ownership.owner_machine_id, "hm_owner");
    assert!(aliased.warning.is_some(), "alias link must warn");

    // Unknown, retired, and collision-style failures reject before mutation.
    assert!(
        service
            .link_workspace_owner(&registry, "ws_active", "nope")
            .expect_err("unknown owner")
            .to_string()
            .contains("not a registered host")
    );
    assert!(
        service
            .link_workspace_owner(&registry, "ws_active", "gone")
            .expect_err("retired owner")
            .to_string()
            .contains("retired")
    );
}

#[test]
fn retire_guarding_hub_rejects_self_retirement_before_mutation() {
    let store = RegistryStore::open_in_memory().expect("store");
    let service = HostRegistryService::new(store);
    service
        .register_hub_identity(&identity("hm_hub", "hub", HostMode::Hub), BTreeSet::new())
        .expect("register hub");
    service
        .register_identity(
            &identity("hm_spoke", "spoke", HostMode::Spoke),
            BTreeSet::new(),
        )
        .expect("register spoke");
    let error = service
        .retire_guarding_hub("hm_hub")
        .expect_err("hub cannot retire itself")
        .to_string();
    assert!(error.contains("hub"), "unexpected: {error}");
    // The hub is still active — no mutation happened.
    assert!(matches!(
        service.resolve("hub").expect("resolve hub"),
        HostNameResolution::Active { .. }
    ));
    // A non-hub machine retires normally.
    service
        .retire_guarding_hub("hm_spoke")
        .expect("retire spoke");
}
