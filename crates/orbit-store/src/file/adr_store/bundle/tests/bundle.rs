// Migrated from file/adr_store/bundle.rs per ORB-00231
use chrono::TimeZone;
use chrono::Utc;
use orbit_common::types::{Adr, AdrStatus, LegacyValidation, NotFoundKind};
use tempfile::tempdir;

use super::super::super::constants::ADR_SCHEMA_VERSION;
use super::super::*;

fn sample_bundle(id: &str) -> AdrBundle {
    let ts = Utc.with_ymd_and_hms(2026, 5, 11, 0, 0, 0).unwrap();
    AdrBundle {
        doc: AdrFileDocument {
            schema_version: ADR_SCHEMA_VERSION,
            adr: Adr {
                id: id.to_string(),
                title: "Test decision".to_string(),
                status: AdrStatus::Proposed,
                owner: "claude".to_string(),
                created_at: ts,
                accepted_at: None,
                last_updated: ts,
                related_features: vec![],
                related_tasks: vec![],
                tags: vec![],
                paths: vec![],
                supersedes: vec![],
                superseded_by: None,
                legacy_ids: vec![],
                validation_warnings: vec![],
                legacy_validation: LegacyValidation::None,
            },
        },
        body: "## Context\n\nSomething.\n".to_string(),
    }
}

#[test]
fn write_then_read_round_trips_the_bundle() {
    let tempdir = tempdir().expect("tempdir");
    let dir = tempdir.path().join("ADR-0001");
    let bundle = sample_bundle("ADR-0001");

    write_bundle_at(&dir, &bundle).expect("write bundle");
    let loaded = read_bundle_at(&dir).expect("read bundle");

    assert_eq!(loaded, bundle);
}

#[test]
fn read_bundle_returns_empty_body_when_body_md_missing() {
    let tempdir = tempdir().expect("tempdir");
    let dir = tempdir.path().join("ADR-0001");
    fs::create_dir_all(&dir).expect("create adr dir");
    let bundle = sample_bundle("ADR-0001");
    write_yaml_atomic_with(&adr_doc_path(&dir), &bundle.doc, serialize_adr_doc_yaml)
        .expect("write doc only");

    let loaded = read_bundle_at(&dir).expect("read bundle");

    assert_eq!(loaded.body, "");
    assert_eq!(loaded.doc, bundle.doc);
}

#[test]
fn read_bundle_on_nonexistent_dir_returns_adr_not_found() {
    let tempdir = tempdir().expect("tempdir");
    let dir = tempdir.path().join("ADR-9999");

    let err = read_bundle_at(&dir).expect_err("missing dir should error");

    assert!(
        matches!(
            err,
            OrbitError::NotFound {
                kind: NotFoundKind::Adr,
                ref id,
            } if id == "ADR-9999"
        ),
        "expected missing ADR error, got {err:?}"
    );
}
