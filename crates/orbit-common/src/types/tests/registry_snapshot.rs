use chrono::TimeZone;

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
