use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use orbit_common::types::{
    ExecutionProfileCrewV1, ExecutionProfileShipV1, ExecutionProfileV1, ProjectionFreshness,
    Workspace, WorkspaceRegistry, WorkspaceStatus,
};

use crate::execution_profile_projection::{ExecutionProfileProjection, FixedProfileClock};
use crate::host_identity::{HOST_IDENTITY_SCHEMA_VERSION, HostIdentity, HostMode};
use crate::host_registry::HostRegistryService;
use crate::persistence::RemoteStore;

const TTL: Duration = Duration::minutes(10);

fn base_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 18, 9, 0, 0)
        .single()
        .expect("timestamp")
}

fn identity(machine_id: &str, host_id: &str) -> HostIdentity {
    HostIdentity {
        schema_version: HOST_IDENTITY_SCHEMA_VERSION,
        machine_id: machine_id.to_string(),
        host_id: host_id.to_string(),
        task_prefix: "ORB".to_string(),
        mode: HostMode::Hub,
    }
}

fn workspace(id: &str, owner_machine_id: &str) -> Workspace {
    Workspace {
        id: id.to_string(),
        name: id.to_string(),
        owner_machine_id: Some(owner_machine_id.to_string()),
        git_remote: Some("git@github.com:example/repo.git".to_string()),
        ship_mode: Some("pr".to_string()),
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: base_time(),
        updated_at: base_time(),
    }
}

fn crew(name: &str, model: &str) -> ExecutionProfileCrewV1 {
    ExecutionProfileCrewV1 {
        name: name.to_string(),
        provider: "codex".to_string(),
        model: model.to_string(),
        backend: "cli".to_string(),
        description: Some(format!("{name} crew")),
        tags: vec!["execution".to_string()],
    }
}

fn profile(
    workspace_id: &str,
    owner_machine_id: &str,
    default_crew: &str,
    crews: Vec<ExecutionProfileCrewV1>,
    observed_at: DateTime<Utc>,
) -> ExecutionProfileV1 {
    let mut profile = ExecutionProfileV1 {
        schema_version: 1,
        workspace_id: workspace_id.to_string(),
        owner_machine_id: owner_machine_id.to_string(),
        observed_at,
        config_digest: String::new(),
        default_crew: default_crew.to_string(),
        crews,
        ship: ExecutionProfileShipV1 {
            mode: "pr".to_string(),
            base_branch: "agent-main".to_string(),
            ship_closure_digest: "a".repeat(64),
        },
    };
    profile.config_digest = profile.compute_config_digest().expect("config digest");
    profile
}

/// Build a store with two workspaces whose owner profiles deliberately differ,
/// plus one owned workspace that never publishes a profile.
fn two_workspace_store() -> RemoteStore {
    let store = RemoteStore::open_in_memory().expect("store");
    let service = HostRegistryService::new(store.clone());
    for (machine, host) in [
        ("hm_owner_a", "owner-a"),
        ("hm_owner_b", "owner-b"),
        ("hm_owner_c", "owner-c"),
    ] {
        service
            .register_identity(&identity(machine, host), Default::default())
            .expect("register owner");
    }
    let registry = WorkspaceRegistry {
        schema_version: 1,
        workspaces: vec![
            workspace("ws_alpha", "hm_owner_a"),
            workspace("ws_beta", "hm_owner_b"),
            workspace("ws_gamma", "hm_owner_c"),
        ],
        checkouts: Vec::new(),
    };
    for (workspace_id, owner) in [
        ("ws_alpha", "hm_owner_a"),
        ("ws_beta", "hm_owner_b"),
        ("ws_gamma", "hm_owner_c"),
    ] {
        service
            .bind_workspace_owner(&registry, workspace_id, owner)
            .expect("bind owner");
    }
    // ws_alpha owner publishes {sol}; ws_beta owner publishes {qa}. ws_gamma
    // owner never publishes: it stays a "missing" workspace.
    service
        .publish_execution_profile_at(
            "hm_owner_a",
            0,
            &profile(
                "ws_alpha",
                "hm_owner_a",
                "sol",
                vec![crew("sol", "gpt-alpha")],
                base_time(),
            ),
            base_time(),
        )
        .expect("publish alpha");
    service
        .publish_execution_profile_at(
            "hm_owner_b",
            0,
            &profile(
                "ws_beta",
                "hm_owner_b",
                "qa",
                vec![crew("qa", "claude-beta")],
                base_time(),
            ),
            base_time(),
        )
        .expect("publish beta");
    store
}

fn projection_at(store: RemoteStore, now: DateTime<Utc>) -> ExecutionProfileProjection {
    ExecutionProfileProjection::with_clock(store, Arc::new(FixedProfileClock(now)), TTL)
}

#[test]
fn discovery_and_validation_never_leak_one_workspace_crews_into_another() {
    let projection = projection_at(two_workspace_store(), base_time());

    let alpha = projection
        .crew_discovery("ws_alpha")
        .expect("alpha discovery");
    assert_eq!(alpha.owner_machine_id.as_deref(), Some("hm_owner_a"));
    assert_eq!(alpha.default_crew.as_deref(), Some("sol"));
    assert_eq!(
        alpha
            .crews
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["sol"]
    );

    let beta = projection
        .crew_discovery("ws_beta")
        .expect("beta discovery");
    assert_eq!(
        beta.crews
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["qa"]
    );

    // A crew valid in one workspace is unknown in the other; validation reads
    // only the resolved workspace's owner profile.
    projection
        .validate_task_crew("ws_alpha", "sol")
        .expect("sol valid in alpha");
    let cross = projection
        .validate_task_crew("ws_alpha", "qa")
        .expect_err("qa is not published for alpha")
        .to_string();
    assert!(cross.contains("qa"), "unexpected: {cross}");
    assert!(cross.contains("ws_alpha"), "unexpected: {cross}");
}

#[test]
fn discovery_and_validation_observe_the_same_generation_and_track_republication() {
    let store = two_workspace_store();
    let service = HostRegistryService::new(store.clone());
    let projection = projection_at(store.clone(), base_time());

    let first = projection.crew_discovery("ws_alpha").expect("discovery");
    assert_eq!(first.profile.freshness, ProjectionFreshness::Current);
    assert_eq!(first.profile.generation, Some(1));
    let validated = projection
        .validate_task_crew("ws_alpha", "sol")
        .expect("valid");
    assert_eq!(validated.generation, 1);
    assert_eq!(first.profile.generation, Some(validated.generation));

    // Publishing a newer *semantic* profile advances the generation for both
    // paths together.
    service
        .publish_execution_profile_at(
            "hm_owner_a",
            1,
            &profile(
                "ws_alpha",
                "hm_owner_a",
                "sol",
                vec![crew("sol", "gpt-alpha-2")],
                base_time() + Duration::minutes(1),
            ),
            base_time() + Duration::minutes(1),
        )
        .expect("republish semantic change");
    let after = projection_at(store.clone(), base_time() + Duration::minutes(1));
    assert_eq!(
        after
            .crew_discovery("ws_alpha")
            .expect("discovery")
            .profile
            .generation,
        Some(2)
    );
    assert_eq!(
        after
            .validate_task_crew("ws_alpha", "sol")
            .expect("valid")
            .generation,
        2
    );

    // An identical refresh renews freshness without inventing a generation.
    service
        .publish_execution_profile_at(
            "hm_owner_a",
            2,
            &profile(
                "ws_alpha",
                "hm_owner_a",
                "sol",
                vec![crew("sol", "gpt-alpha-2")],
                base_time() + Duration::minutes(2),
            ),
            base_time() + Duration::minutes(2),
        )
        .expect("republish identical");
    let refreshed = projection_at(store, base_time() + Duration::minutes(2));
    let discovery = refreshed.crew_discovery("ws_alpha").expect("discovery");
    assert_eq!(discovery.profile.generation, Some(2));
    assert_eq!(discovery.profile.freshness, ProjectionFreshness::Current);
}

#[test]
fn missing_profile_is_inspectable_discovery_but_blocks_validation() {
    let projection = projection_at(two_workspace_store(), base_time());
    let discovery = projection.crew_discovery("ws_gamma").expect("discovery");
    assert_eq!(discovery.profile.freshness, ProjectionFreshness::Missing);
    assert_eq!(discovery.profile.generation, None);
    assert!(discovery.crews.is_empty());
    assert!(discovery.default_crew.is_none());

    let error = projection
        .validate_task_crew("ws_gamma", "sol")
        .expect_err("missing profile blocks crew")
        .to_string();
    assert!(error.contains("missing"), "unexpected: {error}");
    assert!(error.contains("ws_gamma"), "unexpected: {error}");
    assert!(error.contains("hm_owner_c"), "unexpected: {error}");
}

#[test]
fn stale_profile_returns_crews_bound_to_stale_generation_but_never_validates() {
    // A clock past the TTL makes the alpha profile stale.
    let projection = projection_at(two_workspace_store(), base_time() + Duration::minutes(11));
    let discovery = projection.crew_discovery("ws_alpha").expect("discovery");
    assert_eq!(discovery.profile.freshness, ProjectionFreshness::Stale);
    // Crews remain inspectable but stay tied to the stale generation.
    assert_eq!(discovery.profile.generation, Some(1));
    assert_eq!(
        discovery
            .crews
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["sol"]
    );

    let error = projection
        .validate_task_crew("ws_alpha", "sol")
        .expect_err("stale profile is never dispatch-eligible")
        .to_string();
    assert!(error.contains("stale"), "unexpected: {error}");
    assert!(error.contains("age="), "unexpected: {error}");
}

#[test]
fn discovery_is_sanitized_and_exposes_no_config_or_ship_material() {
    let projection = projection_at(two_workspace_store(), base_time());
    let discovery = projection.crew_discovery("ws_alpha").expect("discovery");
    let serialized = serde_json::to_string(&discovery).expect("serialize");

    // The crew projection intentionally exposes model/provider/backend.
    assert!(serialized.contains("gpt-alpha"));
    assert!(serialized.contains("\"provider\":\"codex\""));

    for forbidden in [
        "config_digest",
        "ship_closure_digest",
        "ship",
        "base_branch",
        "root",
        "repo_root",
        "orbit_dir",
        "secret",
        "token",
        "ssh",
        "observed_at\":\"/", // no path smuggled through a timestamp field
    ] {
        assert!(
            !serialized.contains(forbidden),
            "crew discovery leaked '{forbidden}': {serialized}"
        );
    }
}
