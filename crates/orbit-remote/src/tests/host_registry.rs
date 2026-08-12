use std::collections::BTreeSet;
use std::fs;

use chrono::{Duration, TimeZone, Utc};
use orbit_common::types::{
    ExecutionProfileCrewV1, ExecutionProfileShipV1, ExecutionProfileV1, HostNameResolution,
    HostStatus, ProjectionFreshness, Workspace, WorkspaceRegistry, WorkspaceStatus,
};

use crate::host_identity::{HOST_IDENTITY_SCHEMA_VERSION, HostIdentity, HostMode};
use crate::host_registry::{HostRegistryService, require_local_hub_identity};
use crate::persistence::RemoteStore;

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
fn hub_administration_preflight_rejects_spoke_local_execution() {
    let root = tempfile::tempdir().expect("tempdir");
    fs::write(
        root.path().join("host.toml"),
        "schema_version = 1\nmachine_id = \"hm_spoke\"\nhost_id = \"spoke\"\nmode = \"spoke\"\n",
    )
    .expect("write host identity");

    let error = require_local_hub_identity(root.path())
        .expect_err("spoke-local administration must fail")
        .to_string();
    assert!(error.contains("hub-local"), "unexpected: {error}");
    assert!(error.contains("spoke"), "unexpected: {error}");
}

#[test]
fn hub_administration_preflight_rejects_unstamped_and_shadow_stores() {
    let service = HostRegistryService::new(RemoteStore::open_in_memory().expect("store"));
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
    let service = HostRegistryService::new(RemoteStore::open_in_memory().expect("store"));
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

const PROFILE_WORKSPACE_ID: &str = "alpha-abc123";

fn execution_profile(
    workspace_id: &str,
    owner_machine_id: &str,
    observed_at: chrono::DateTime<Utc>,
) -> ExecutionProfileV1 {
    let mut profile = ExecutionProfileV1 {
        schema_version: 1,
        workspace_id: workspace_id.to_string(),
        owner_machine_id: owner_machine_id.to_string(),
        observed_at,
        config_digest: String::new(),
        default_crew: "sol".to_string(),
        crews: vec![ExecutionProfileCrewV1 {
            name: "sol".to_string(),
            provider: "codex".to_string(),
            model: "gpt-test".to_string(),
            backend: "cli".to_string(),
            description: Some("Systems implementation".to_string()),
            tags: vec!["hard".to_string()],
        }],
        ship: ExecutionProfileShipV1 {
            mode: "pr".to_string(),
            base_branch: "agent-main".to_string(),
            ship_closure_digest: "a".repeat(64),
        },
    };
    profile.config_digest = profile.compute_config_digest().expect("config digest");
    profile
}

#[test]
fn service_requires_explicit_existing_workspace_and_consistent_local_owner_mirror() {
    let store = RemoteStore::open_in_memory().expect("store");
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
fn service_profile_publication_uses_hub_receipt_for_freshness() {
    let store = RemoteStore::open_in_memory().expect("store");
    let service = HostRegistryService::new(store.clone());
    service
        .register_identity(
            &identity("hm_owner", "owner", HostMode::Hub),
            BTreeSet::new(),
        )
        .expect("host");
    let registry = workspace_registry(vec![workspace(PROFILE_WORKSPACE_ID, Some("hm_owner"))]);
    service
        .bind_workspace_owner(&registry, PROFILE_WORKSPACE_ID, "hm_owner")
        .expect("ownership");
    let received_at = Utc
        .with_ymd_and_hms(2026, 7, 18, 9, 0, 0)
        .single()
        .expect("timestamp");
    let profile = execution_profile(PROFILE_WORKSPACE_ID, "hm_owner", received_at);
    service
        .publish_execution_profile_at("hm_owner", 0, &profile, received_at)
        .expect("publish");
    let current = store
        .sanitized_execution_profile(
            PROFILE_WORKSPACE_ID,
            received_at + Duration::minutes(9),
            Duration::minutes(10),
        )
        .expect("current");
    assert_eq!(current.freshness, ProjectionFreshness::Current);
    let stale = store
        .sanitized_execution_profile(
            PROFILE_WORKSPACE_ID,
            received_at + Duration::minutes(11),
            Duration::minutes(10),
        )
        .expect("stale");
    assert_eq!(stale.freshness, ProjectionFreshness::Stale);
}

#[test]
fn link_workspace_owner_binds_active_warns_on_alias_and_rejects_bad_resolutions() {
    let store = RemoteStore::open_in_memory().expect("store");
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
    let store = RemoteStore::open_in_memory().expect("store");
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
