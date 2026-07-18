use std::collections::BTreeSet;

use chrono::{TimeZone, Utc};
use serde_json::json;

use crate::types::{HostAlias, HostNameResolution, HostRecord, HostStatus};

fn host() -> HostRecord {
    let timestamp = Utc
        .with_ymd_and_hms(2026, 7, 18, 6, 0, 0)
        .single()
        .expect("fixed timestamp");
    HostRecord {
        machine_id: "hm_alpha".to_string(),
        host_id: "alpha".to_string(),
        labels: BTreeSet::from(["codex".to_string(), "os:linux".to_string()]),
        status: HostStatus::Active,
        registered_at: timestamp,
        updated_at: timestamp,
        retired_at: None,
        last_seen_at: Some(timestamp),
    }
}

#[test]
fn host_projection_serializes_only_registry_core_fields() {
    let value = serde_json::to_value(host()).expect("serialize host");
    assert_eq!(
        value,
        json!({
            "machine_id": "hm_alpha",
            "host_id": "alpha",
            "labels": ["codex", "os:linux"],
            "status": "active",
            "registered_at": "2026-07-18T06:00:00Z",
            "updated_at": "2026-07-18T06:00:00Z",
            "retired_at": null,
            "last_seen_at": "2026-07-18T06:00:00Z"
        })
    );
    let serialized = value.to_string();
    for forbidden in [
        "credential",
        "secret",
        "ssh",
        "workspace",
        "repo_root",
        "orbit_dir",
        "transport",
        "target",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "host projection must not expose {forbidden}"
        );
    }
}

#[test]
fn alias_and_retired_resolution_keep_warning_and_lifecycle_metadata() {
    let mut retired = host();
    retired.status = HostStatus::Retired;
    retired.retired_at = Some(retired.updated_at);
    let alias = HostAlias {
        alias_host_id: "alpha-old".to_string(),
        machine_id: retired.machine_id.clone(),
        created_at: retired.registered_at,
        warning: "permanent tombstone alias".to_string(),
    };

    let resolution = HostNameResolution::Retired {
        host: retired,
        alias: Some(alias),
    };
    let value = serde_json::to_value(resolution).expect("serialize resolution");
    assert_eq!(value["kind"], "retired");
    assert_eq!(value["alias"]["alias_host_id"], "alpha-old");
    assert_eq!(value["alias"]["warning"], "permanent tombstone alias");
}

#[test]
fn status_parse_and_display_round_trip() {
    for status in [HostStatus::Active, HostStatus::Retired] {
        assert_eq!(status.to_string().parse::<HostStatus>(), Ok(status));
    }
    assert!("unknown".parse::<HostStatus>().is_err());
}
