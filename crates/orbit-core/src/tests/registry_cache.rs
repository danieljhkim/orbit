use std::io;

use chrono::{Duration, TimeZone, Utc};
use orbit_common::types::{REGISTRY_SNAPSHOT_SCHEMA_VERSION, RegistrySnapshotV1};

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
