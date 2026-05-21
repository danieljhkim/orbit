// Migrated from file/learning_store/layout.rs per ORB-00231
use std::fs;

use chrono::TimeZone;
use tempfile::tempdir;

use super::super::*;

#[test]
fn next_learning_id_on_empty_root_is_one() {
    let dir = tempdir().expect("tempdir");
    let now = Utc.with_ymd_and_hms(2026, 5, 11, 0, 0, 0).unwrap();
    let id = next_learning_id(dir.path(), now).expect("next id");
    assert_eq!(id, "L-0001");
}

#[test]
fn next_learning_id_scans_active_and_superseded_dirs() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("L-0001")).expect("seed active dir");
    fs::write(dir.path().join("L-0001").join(LEARNING_DOC_FILE_NAME), "").expect("seed active");
    fs::create_dir_all(dir.path().join("L-0003")).expect("seed superseded dir");
    fs::write(dir.path().join("L-0003").join(LEARNING_DOC_FILE_NAME), "").expect("seed superseded");

    let now = Utc.with_ymd_and_hms(2026, 5, 11, 0, 0, 0).unwrap();
    let id = next_learning_id(dir.path(), now).expect("next id");
    assert_eq!(id, "L-0004");
}

#[test]
fn next_learning_id_ignores_legacy_date_ids() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("L20260510-99")).expect("seed yesterday dir");
    fs::write(
        dir.path().join("L20260510-99").join(LEARNING_DOC_FILE_NAME),
        "",
    )
    .expect("seed yesterday");
    let now = Utc.with_ymd_and_hms(2026, 5, 11, 0, 0, 0).unwrap();
    let id = next_learning_id(dir.path(), now).expect("next id");
    assert_eq!(id, "L-0001");
}

#[test]
fn locate_learning_finds_record_in_either_state() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("L-0001")).expect("mk active");
    fs::create_dir_all(dir.path().join("L-0002")).expect("mk superseded");
    fs::write(dir.path().join("L-0001").join(LEARNING_DOC_FILE_NAME), "").expect("active");
    fs::write(dir.path().join("L-0002").join(LEARNING_DOC_FILE_NAME), "").expect("superseded");

    let path = locate_learning(dir.path(), "L-0001")
        .expect("locate")
        .expect("found");
    assert_eq!(path, dir.path().join("L-0001").join(LEARNING_DOC_FILE_NAME));

    let path = locate_learning(dir.path(), "L-0002")
        .expect("locate")
        .expect("found");
    assert_eq!(path, dir.path().join("L-0002").join(LEARNING_DOC_FILE_NAME));
}

#[test]
fn validate_learning_id_accepts_well_formed_ids() {
    assert!(validate_learning_id("L-0001").is_ok());
    assert!(validate_learning_id("L-9999").is_ok());
    assert!(validate_learning_id("L-10000").is_ok());
}

#[test]
fn validate_learning_id_rejects_path_like_ids() {
    for bad in [
        "",
        "  ",
        "T20260511-1",
        "L20260511-1",
        "L-001",
        "L-",
        "L-0001/escape",
        "../L-0001",
    ] {
        assert!(
            validate_learning_id(bad).is_err(),
            "expected reject for {bad:?}"
        );
    }
}
