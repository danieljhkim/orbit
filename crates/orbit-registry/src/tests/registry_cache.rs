use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;

use chrono::{Duration, TimeZone, Utc};
use orbit_common::types::{
    ExecutionProfileCrewV1, ExecutionProfileShipV1, ExecutionProfileV1, HostRegistration,
    HostStatus, OrbitError, ProjectionFreshness, REGISTRY_CACHE_SCHEMA_VERSION,
    REGISTRY_SNAPSHOT_SCHEMA_VERSION, RegistryCacheV1, RegistryHostV1, RegistryPresenceV1,
    RegistryProfileV1, RegistrySnapshotV1, RegistryWorkspaceV1, WorkspacePresenceDeclaration,
};

use crate::persistence::RegistryStore;
use crate::registry_cache::{RegistryCacheOutcome, RegistryCacheService, RegistryCacheState};

fn snapshot(revision: u64, hub: Option<&str>) -> RegistrySnapshotV1 {
    RegistrySnapshotV1 {
        schema_version: REGISTRY_SNAPSHOT_SCHEMA_VERSION,
        hub_machine_id: hub.map(str::to_string),
        registry_revision: revision,
        hosts: Vec::new(),
        workspaces: Vec::new(),
    }
}

fn populated_snapshot(
    revision: u64,
    freshness: ProjectionFreshness,
    age_seconds: u64,
) -> RegistrySnapshotV1 {
    let observed = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
    let mut value = snapshot(revision, Some("hm_hub"));
    value.hosts.push(RegistryHostV1 {
        machine_id: "hm_hub".to_string(),
        host_id: "hub".to_string(),
        labels: Default::default(),
        status: HostStatus::Active,
        registered_at: observed - Duration::hours(1),
        updated_at: observed,
        retired_at: None,
        last_seen_at: Some(observed),
        aliases: Vec::new(),
        presence: vec![RegistryPresenceV1 {
            workspace_id: "ws_a".to_string(),
            freshness,
            last_verified: Some(observed),
            age_seconds: Some(age_seconds),
        }],
    });
    value.workspaces.push(RegistryWorkspaceV1 {
        workspace_id: "ws_a".to_string(),
        owner_machine_id: "hm_hub".to_string(),
        owner_host_id: Some("hub".to_string()),
        profile: RegistryProfileV1 {
            freshness,
            generation: Some(7),
            observed_at: Some(observed - Duration::seconds(1)),
            received_at: Some(observed),
            age_seconds: Some(age_seconds),
        },
    });
    value
}

fn service() -> (tempfile::TempDir, RegistryCacheService) {
    let dir = tempfile::tempdir().expect("tempdir");
    let service = RegistryCacheService::new(dir.path());
    (dir, service)
}

#[test]
fn missing_cache_is_reported_as_missing() {
    let (_dir, service) = service();
    let state = service
        .load(Utc::now(), Duration::minutes(5))
        .expect("load missing");
    assert_eq!(state, RegistryCacheState::Missing);
}

#[test]
fn refresh_writes_then_load_reports_current_and_stale_by_threshold() {
    let (_dir, service) = service();
    let t0 = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
    let outcome = service
        .refresh(snapshot(3, Some("hm_hub")), t0)
        .expect("refresh");
    assert_eq!(outcome, RegistryCacheOutcome::Written { revision: 3 });

    let current = service
        .load(t0 + Duration::minutes(1), Duration::minutes(5))
        .expect("load current");
    match current {
        RegistryCacheState::Current { cache, age_seconds } => {
            assert_eq!(cache.snapshot.registry_revision, 3);
            assert_eq!(age_seconds, 60);
        }
        other => panic!("expected Current, got {other:?}"),
    }

    let stale = service
        .load(t0 + Duration::minutes(10), Duration::minutes(5))
        .expect("load stale");
    assert!(matches!(stale, RegistryCacheState::Stale { .. }));
}

#[test]
fn malformed_and_future_schema_are_distinguished_and_not_rewritten() {
    let (_dir, service) = service();
    std::fs::write(service.cache_path(), b"{ not json").expect("write malformed");
    let state = service
        .load(Utc::now(), Duration::minutes(5))
        .expect("load");
    assert!(matches!(state, RegistryCacheState::Malformed { .. }));
    // The invalid file is untouched.
    assert_eq!(
        std::fs::read(service.cache_path()).expect("reread"),
        b"{ not json"
    );

    std::fs::write(
        service.cache_path(),
        br#"{"schema_version": 999, "received_at": "2026-07-18T12:00:00Z", "snapshot": {}}"#,
    )
    .expect("write future");
    let state = service
        .load(Utc::now(), Duration::minutes(5))
        .expect("load");
    assert_eq!(
        state,
        RegistryCacheState::UnsupportedFuture {
            schema_version: 999
        }
    );

    let mut nested_future = serde_json::to_value(RegistryCacheV1 {
        schema_version: REGISTRY_CACHE_SCHEMA_VERSION,
        received_at: Utc::now(),
        snapshot: snapshot(7, Some("hm_hub")),
    })
    .expect("serialize cache value");
    nested_future["snapshot"]["schema_version"] = serde_json::json!(999);
    let nested_bytes = serde_json::to_vec(&nested_future).expect("serialize nested future");
    std::fs::write(service.cache_path(), &nested_bytes).expect("write nested future");
    assert_eq!(
        service
            .load(Utc::now(), Duration::minutes(5))
            .expect("load nested future"),
        RegistryCacheState::UnsupportedFuture {
            schema_version: 999
        }
    );
    assert_eq!(
        std::fs::read(service.cache_path()).expect("reread nested future"),
        nested_bytes
    );
}

#[test]
fn refresh_rejects_future_snapshot_schema_before_first_write() {
    let (_dir, service) = service();
    let mut future = snapshot(1, Some("hm_hub"));
    future.schema_version = REGISTRY_SNAPSHOT_SCHEMA_VERSION + 1;
    let error = service
        .refresh(future, Utc::now())
        .expect_err("future snapshot must be rejected")
        .to_string();
    assert!(error.contains("future"), "unexpected: {error}");
    assert!(!service.cache_path().exists());
}

#[test]
fn first_refresh_rejects_missing_or_invalid_hub_identity_before_write() {
    for (hub, expected) in [(None, "omits"), (Some("not-a-machine-id"), "invalid")] {
        let (_dir, service) = service();
        let error = service
            .refresh(snapshot(1, hub), Utc::now())
            .expect_err("an unpinned first refresh must be rejected")
            .to_string();
        assert!(error.contains(expected), "unexpected: {error}");
        assert!(
            !service.cache_path().exists(),
            "invalid first refresh must not create a cache"
        );
    }
}

#[test]
fn persisted_unpinned_cache_is_malformed_on_reload_without_rewrite() {
    let (_dir, service) = service();
    let unpinned = RegistryCacheV1 {
        schema_version: REGISTRY_CACHE_SCHEMA_VERSION,
        received_at: Utc::now(),
        snapshot: snapshot(5, None),
    };
    let bytes = serde_json::to_vec_pretty(&unpinned).expect("serialize unpinned cache");
    std::fs::write(service.cache_path(), &bytes).expect("write unpinned cache");

    match service
        .load(Utc::now(), Duration::minutes(5))
        .expect("classify unpinned cache")
    {
        RegistryCacheState::Malformed { reason } => {
            assert!(reason.contains("hub_machine_id"), "unexpected: {reason}");
        }
        other => panic!("expected Malformed, got {other:?}"),
    }
    assert_eq!(
        std::fs::read(service.cache_path()).expect("reread unpinned cache"),
        bytes,
        "load must not rewrite an unpinned cache"
    );
}

#[test]
fn higher_revision_cannot_switch_hubs_from_a_persisted_unpinned_cache() {
    let (_dir, service) = service();
    let unpinned = RegistryCacheV1 {
        schema_version: REGISTRY_CACHE_SCHEMA_VERSION,
        received_at: Utc::now(),
        snapshot: snapshot(5, None),
    };
    let bytes = serde_json::to_vec_pretty(&unpinned).expect("serialize unpinned cache");
    std::fs::write(service.cache_path(), &bytes).expect("write unpinned cache");

    let error = service
        .refresh(snapshot(6, Some("hm_other")), Utc::now())
        .expect_err("an unpinned cache must not adopt a higher-revision hub")
        .to_string();
    assert!(error.contains("malformed"), "unexpected: {error}");
    assert_eq!(
        std::fs::read(service.cache_path()).expect("reread after rejected switch"),
        bytes,
        "rejected hub switch must preserve the unpinned bytes for diagnosis"
    );
}

#[test]
fn refresh_accepts_higher_revision_and_rejects_lower() {
    let (_dir, service) = service();
    let now = Utc::now();
    service
        .refresh(snapshot(5, Some("hm_hub")), now)
        .expect("first");
    service
        .refresh(snapshot(6, Some("hm_hub")), now)
        .expect("higher revision");

    let error = service
        .refresh(snapshot(4, Some("hm_hub")), now)
        .expect_err("lower revision must be rejected")
        .to_string();
    assert!(error.contains("lower"), "unexpected: {error}");
    // Prior bytes preserved at revision 6.
    match service.load(now, Duration::minutes(5)).expect("load") {
        RegistryCacheState::Current { cache, .. } => {
            assert_eq!(cache.snapshot.registry_revision, 6);
        }
        other => panic!("expected Current, got {other:?}"),
    }
}

#[test]
fn refresh_rejects_different_hub_and_divergent_payload_at_equal_revision() {
    let (_dir, service) = service();
    let now = Utc::now();
    service
        .refresh(snapshot(5, Some("hm_hub")), now)
        .expect("first");
    let prior_bytes = std::fs::read(service.cache_path()).expect("prior bytes");

    let hub_error = service
        .refresh(snapshot(6, Some("hm_other")), now)
        .expect_err("different hub must be rejected")
        .to_string();
    assert!(hub_error.contains("hub"), "unexpected: {hub_error}");

    let mut divergent = snapshot(5, Some("hm_hub"));
    divergent.hosts.push(orbit_common::types::RegistryHostV1 {
        machine_id: "hm_x".to_string(),
        host_id: "x".to_string(),
        labels: Default::default(),
        status: orbit_common::types::HostStatus::Active,
        registered_at: now,
        updated_at: now,
        retired_at: None,
        last_seen_at: None,
        aliases: Vec::new(),
        presence: Vec::new(),
    });
    let payload_error = service
        .refresh(divergent, now)
        .expect_err("divergent payload at equal revision must be rejected")
        .to_string();
    assert!(
        payload_error.contains("payload"),
        "unexpected: {payload_error}"
    );

    let missing_hub_error = service
        .refresh(snapshot(6, None), now)
        .expect_err("a pinned hub identity must not disappear")
        .to_string();
    assert!(
        missing_hub_error.contains("omits"),
        "unexpected: {missing_hub_error}"
    );
    assert_eq!(
        std::fs::read(service.cache_path()).expect("bytes after rejected refreshes"),
        prior_bytes,
        "hub and same-revision payload conflicts must preserve prior bytes"
    );
}

#[test]
fn refresh_renews_receipt_for_identical_payload_at_equal_revision() {
    let (_dir, service) = service();
    let t0 = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
    service
        .refresh(snapshot(5, Some("hm_hub")), t0)
        .expect("first");
    let outcome = service
        .refresh(snapshot(5, Some("hm_hub")), t0 + Duration::minutes(3))
        .expect("renew");
    assert_eq!(
        outcome,
        RegistryCacheOutcome::ReceiptRenewed { revision: 5 }
    );
    match service
        .load(t0 + Duration::minutes(3), Duration::minutes(5))
        .expect("load")
    {
        RegistryCacheState::Current { cache, age_seconds } => {
            assert_eq!(cache.received_at, t0 + Duration::minutes(3));
            assert_eq!(age_seconds, 0);
        }
        other => panic!("expected Current, got {other:?}"),
    }
}

#[test]
fn refresh_renews_only_receipt_when_read_time_freshness_views_change() {
    let (_dir, service) = service();
    let t0 = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
    let original = populated_snapshot(5, ProjectionFreshness::Current, 0);
    service.refresh(original.clone(), t0).expect("first");

    let later_view = populated_snapshot(5, ProjectionFreshness::Stale, 600);
    let outcome = service
        .refresh(later_view, t0 + Duration::minutes(10))
        .expect("derived freshness must not conflict at equal revision");
    assert_eq!(
        outcome,
        RegistryCacheOutcome::ReceiptRenewed { revision: 5 }
    );
    match service
        .load(t0 + Duration::minutes(10), Duration::minutes(5))
        .expect("load renewed")
    {
        RegistryCacheState::Current { cache, age_seconds } => {
            assert_eq!(cache.received_at, t0 + Duration::minutes(10));
            assert_eq!(cache.snapshot, original);
            assert_eq!(age_seconds, 0);
        }
        other => panic!("expected Current, got {other:?}"),
    }
}

#[test]
fn write_failure_preserves_the_prior_valid_snapshot() {
    let (_dir, service) = service();
    let now = Utc::now();
    service
        .refresh(snapshot(5, Some("hm_hub")), now)
        .expect("seed");

    let error = service
        .refresh_with_writer(snapshot(6, Some("hm_hub")), now, |_, _| {
            Err(io::Error::other("injected write failure"))
        })
        .expect_err("injected failure surfaces");
    assert!(
        error.to_string().contains("preserved"),
        "unexpected: {error}"
    );

    // The prior snapshot at revision 5 is still the only readable one.
    match service.load(now, Duration::minutes(5)).expect("load") {
        RegistryCacheState::Current { cache, .. } => {
            assert_eq!(cache.snapshot.registry_revision, 5);
        }
        other => panic!("expected Current, got {other:?}"),
    }
}

#[test]
fn serialization_failure_preserves_prior_bytes_without_calling_writer() {
    let (_dir, service) = service();
    let now = Utc::now();
    service
        .refresh(snapshot(5, Some("hm_hub")), now)
        .expect("seed");
    let prior_bytes = std::fs::read(service.cache_path()).expect("read prior");

    let error = service
        .refresh_with_codec(
            snapshot(6, Some("hm_hub")),
            now,
            |_| {
                Err(OrbitError::Store(
                    "injected serialization failure".to_string(),
                ))
            },
            |_, _| panic!("writer must not run after serialization failure"),
        )
        .expect_err("serialization failure surfaces")
        .to_string();
    assert!(
        error.contains("serialization failure"),
        "unexpected: {error}"
    );
    assert_eq!(
        std::fs::read(service.cache_path()).expect("read preserved"),
        prior_bytes
    );
}

#[test]
fn receipt_renewal_failure_distinguishes_preserved_from_committed_uncertain() {
    let (_dir, service) = service();
    let t0 = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
    let value = populated_snapshot(5, ProjectionFreshness::Current, 0);
    service.refresh(value.clone(), t0).expect("seed");
    let prior_bytes = std::fs::read(service.cache_path()).expect("prior bytes");

    let preserved = service
        .refresh_with_writer(value.clone(), t0 + Duration::minutes(1), |_, _| {
            Err(io::Error::other("injected pre-rename failure"))
        })
        .expect_err("pre-rename failure surfaces")
        .to_string();
    assert!(preserved.contains("preserved"), "unexpected: {preserved}");
    assert_eq!(
        std::fs::read(service.cache_path()).expect("preserved bytes"),
        prior_bytes
    );

    let cache_path = service.cache_path().to_path_buf();
    let uncertain = service
        .refresh_with_writer(value, t0 + Duration::minutes(2), move |path, bytes| {
            assert_eq!(path, cache_path);
            std::fs::write(path, bytes)?;
            Err(io::Error::other("injected post-rename failure"))
        })
        .expect_err("post-rename error surfaces")
        .to_string();
    assert!(
        uncertain.contains("durability is uncertain"),
        "unexpected: {uncertain}"
    );
    match service
        .load(t0 + Duration::minutes(2), Duration::minutes(5))
        .expect("load committed renewal")
    {
        RegistryCacheState::Current { cache, age_seconds } => {
            assert_eq!(cache.received_at, t0 + Duration::minutes(2));
            assert_eq!(age_seconds, 0);
        }
        other => panic!("expected Current, got {other:?}"),
    }
}

#[test]
fn populated_store_snapshot_round_trips_cache_without_private_markers() {
    const ROOT_MARKER: &str = "/ABSOLUTE_SECRET_ROOT_MARKER/ws-a";
    const MODEL_MARKER: &str = "SECRET_MODEL_TOKEN_MARKER";
    const DESCRIPTION_MARKER: &str = "sh -c 'EXFIL_COMMAND_MARKER'";
    const TAG_MARKER: &str = "AWS_SECRET_ACCESS_KEY_MARKER";
    const BRANCH_MARKER: &str = "/private/worktree/BRANCH_MARKER";

    let store = RegistryStore::open_in_memory().expect("store");
    let hub = HostRegistration {
        machine_id: "hm_hub".to_string(),
        host_id: "hub-old".to_string(),
        labels: BTreeSet::from(["coordination".to_string()]),
    };
    store.register_hub(&hub).expect("register hub");
    store.rename_host("hm_hub", "hub").expect("rename hub");
    store
        .register_host(&HostRegistration {
            machine_id: "hm_retired".to_string(),
            host_id: "retired".to_string(),
            labels: BTreeSet::new(),
        })
        .expect("register retired host");
    store.retire_host("hm_retired").expect("retire host");
    store
        .bind_workspace_owner("ws-a", "hm_hub")
        .expect("bind owner");

    let now = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
    store
        .replace_host_workspace_presence(
            "hm_hub",
            &[WorkspacePresenceDeclaration {
                workspace_id: "ws-a".to_string(),
                root: PathBuf::from(ROOT_MARKER),
                last_verified: now,
            }],
            now,
        )
        .expect("presence");

    let mut profile = ExecutionProfileV1 {
        schema_version: 1,
        workspace_id: "ws-a".to_string(),
        owner_machine_id: "hm_hub".to_string(),
        observed_at: now,
        config_digest: String::new(),
        default_crew: "private-crew".to_string(),
        crews: vec![ExecutionProfileCrewV1 {
            name: "private-crew".to_string(),
            provider: "codex".to_string(),
            model: MODEL_MARKER.to_string(),
            backend: "cli".to_string(),
            description: Some(DESCRIPTION_MARKER.to_string()),
            tags: vec![TAG_MARKER.to_string()],
        }],
        ship: ExecutionProfileShipV1 {
            mode: "pr".to_string(),
            base_branch: BRANCH_MARKER.to_string(),
            ship_closure_digest: "b".repeat(64),
        },
    };
    profile.config_digest = profile.compute_config_digest().expect("config digest");
    let private_config_digest = profile.config_digest.clone();
    store
        .publish_execution_profile(
            "hm_hub",
            0,
            &profile,
            now,
            Duration::minutes(10),
            Duration::minutes(2),
        )
        .expect("publish profile");

    let snapshot = store
        .read_registry_snapshot(now, Duration::minutes(5), Duration::minutes(10))
        .expect("snapshot");
    assert_eq!(snapshot.hosts.len(), 2);
    assert!(
        snapshot
            .hosts
            .iter()
            .any(|host| { host.machine_id == "hm_retired" && host.status == HostStatus::Retired })
    );
    let projected_hub = snapshot
        .hosts
        .iter()
        .find(|host| host.machine_id == "hm_hub")
        .expect("hub projection");
    assert_eq!(projected_hub.aliases[0].alias_host_id, "hub-old");
    assert_eq!(projected_hub.presence[0].workspace_id, "ws-a");
    assert_eq!(snapshot.workspaces[0].owner_machine_id, "hm_hub");
    assert_eq!(snapshot.workspaces[0].profile.generation, Some(1));

    let (_dir, service) = service();
    service
        .refresh(snapshot.clone(), now)
        .expect("cache refresh");
    let cache_bytes = std::fs::read(service.cache_path()).expect("cache bytes");
    let cache_json = String::from_utf8(cache_bytes).expect("cache utf8");
    for forbidden in [
        ROOT_MARKER,
        MODEL_MARKER,
        DESCRIPTION_MARKER,
        TAG_MARKER,
        BRANCH_MARKER,
        private_config_digest.as_str(),
        "private-crew",
    ] {
        assert!(
            !cache_json.contains(forbidden),
            "cache leaked private marker '{forbidden}': {cache_json}"
        );
    }
    match service.load(now, Duration::minutes(5)).expect("load cache") {
        RegistryCacheState::Current { cache, age_seconds } => {
            assert_eq!(cache.snapshot, snapshot);
            assert_eq!(age_seconds, 0);
        }
        other => panic!("expected Current, got {other:?}"),
    }

    let later_view = store
        .read_registry_snapshot(
            now + Duration::minutes(20),
            Duration::minutes(5),
            Duration::minutes(10),
        )
        .expect("later snapshot view");
    assert_eq!(later_view.registry_revision, snapshot.registry_revision);
    assert_eq!(
        later_view.hosts[0].presence[0].freshness,
        ProjectionFreshness::Stale
    );
    assert_eq!(
        service
            .refresh(later_view, now + Duration::minutes(20))
            .expect("renew receipt across derived freshness change"),
        RegistryCacheOutcome::ReceiptRenewed {
            revision: snapshot.registry_revision
        }
    );
}
