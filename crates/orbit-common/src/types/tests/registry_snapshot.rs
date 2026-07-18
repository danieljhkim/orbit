use std::collections::BTreeSet;

use chrono::{Duration, TimeZone};

use super::*;

fn snapshot(revision: u64, hub: Option<&str>) -> RegistrySnapshotV1 {
    RegistrySnapshotV1 {
        schema_version: REGISTRY_SNAPSHOT_SCHEMA_VERSION,
        hub_machine_id: hub.map(str::to_string),
        registry_revision: revision,
        hosts: Vec::new(),
        workspaces: Vec::new(),
    }
}

#[test]
fn canonical_payload_eq_ignores_only_receipt() {
    let left = snapshot(4, Some("hm_a"));
    let right = snapshot(4, Some("hm_a"));
    assert!(left.canonical_payload_eq(&right));
}

#[test]
fn canonical_payload_eq_rejects_revision_and_hub_changes() {
    let base = snapshot(4, Some("hm_a"));
    assert!(!base.canonical_payload_eq(&snapshot(5, Some("hm_a"))));
    assert!(!base.canonical_payload_eq(&snapshot(4, Some("hm_b"))));
}

#[test]
fn canonical_payload_eq_ignores_read_time_freshness_views_but_not_timestamps() {
    let observed = Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap();
    let mut left = snapshot(4, Some("hm_a"));
    left.hosts.push(RegistryHostV1 {
        machine_id: "hm_a".to_string(),
        host_id: "hub".to_string(),
        labels: BTreeSet::new(),
        status: HostStatus::Active,
        registered_at: observed - Duration::hours(1),
        updated_at: observed,
        retired_at: None,
        last_seen_at: Some(observed),
        aliases: Vec::new(),
        presence: vec![RegistryPresenceV1 {
            workspace_id: "ws_a".to_string(),
            freshness: ProjectionFreshness::Current,
            last_verified: Some(observed),
            age_seconds: Some(0),
        }],
    });
    left.workspaces.push(RegistryWorkspaceV1 {
        workspace_id: "ws_a".to_string(),
        owner_machine_id: "hm_a".to_string(),
        owner_host_id: Some("hub".to_string()),
        profile: RegistryProfileV1 {
            freshness: ProjectionFreshness::Current,
            generation: Some(3),
            observed_at: Some(observed - Duration::seconds(1)),
            received_at: Some(observed),
            age_seconds: Some(0),
        },
    });

    let mut later_view = left.clone();
    later_view.hosts[0].presence[0].freshness = ProjectionFreshness::Stale;
    later_view.hosts[0].presence[0].age_seconds = Some(600);
    later_view.workspaces[0].profile.freshness = ProjectionFreshness::Stale;
    later_view.workspaces[0].profile.age_seconds = Some(600);
    assert!(left.canonical_payload_eq(&later_view));

    later_view.hosts[0].presence[0].last_verified = Some(observed + Duration::seconds(1));
    assert!(!left.canonical_payload_eq(&later_view));
}

#[test]
fn cache_round_trips_through_json() {
    let cache = RegistryCacheV1 {
        schema_version: REGISTRY_CACHE_SCHEMA_VERSION,
        received_at: Utc.with_ymd_and_hms(2026, 7, 18, 12, 0, 0).unwrap(),
        snapshot: snapshot(7, Some("hm_hub")),
    };
    let encoded = serde_json::to_string(&cache).expect("serialize cache");
    let decoded: RegistryCacheV1 = serde_json::from_str(&encoded).expect("deserialize cache");
    assert_eq!(cache, decoded);
}
