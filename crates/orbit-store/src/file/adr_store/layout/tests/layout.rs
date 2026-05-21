// Migrated from file/adr_store/layout.rs per ORB-00231
use std::fs;

use tempfile::tempdir;

use super::super::*;

#[test]
fn all_returns_four_variants_in_stable_order() {
    let all = AdrStateDir::all();
    assert_eq!(all.len(), 4);
    assert_eq!(all[0], AdrStateDir::Proposed);
    assert_eq!(all[1], AdrStateDir::Accepted);
    assert_eq!(all[2], AdrStateDir::Superseded);
    assert_eq!(all[3], AdrStateDir::Deleted);
}

#[test]
fn from_status_to_status_round_trip_for_each_variant() {
    for status in [
        AdrStatus::Proposed,
        AdrStatus::Accepted,
        AdrStatus::Superseded,
        AdrStatus::Deleted,
    ] {
        let state = AdrStateDir::from_status(status);
        assert_eq!(state.to_status(), status);
    }
}

#[test]
fn next_adr_id_returns_first_id_for_empty_root() {
    let tempdir = tempdir().expect("tempdir");
    let id = next_adr_id(tempdir.path()).expect("next adr id");
    assert_eq!(id, "ADR-0001");
}

#[test]
fn next_adr_id_returns_max_plus_one_across_state_dirs() {
    let tempdir = tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("proposed").join("ADR-0003")).expect("create proposed");
    fs::create_dir_all(root.join("accepted").join("ADR-0017")).expect("create accepted");

    let id = next_adr_id(root).expect("next adr id");
    assert_eq!(id, "ADR-0018");
}

#[test]
fn next_adr_id_skips_non_conforming_directory_names() {
    let tempdir = tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("proposed").join("tmp")).expect("create tmp");
    fs::create_dir_all(root.join("proposed").join("notes")).expect("create notes");
    fs::create_dir_all(root.join("proposed").join("ADR-foo")).expect("create ADR-foo");
    fs::create_dir_all(root.join("accepted").join("ADR-001")).expect("create short id");
    fs::create_dir_all(root.join("accepted").join("ADR-0005")).expect("create valid");

    let id = next_adr_id(root).expect("next adr id");
    assert_eq!(id, "ADR-0006");
}

#[test]
fn next_adr_id_grows_pad_past_four_digits() {
    let tempdir = tempdir().expect("tempdir");
    let root = tempdir.path();
    fs::create_dir_all(root.join("accepted").join("ADR-9999")).expect("create 9999");

    let id = next_adr_id(root).expect("next adr id");
    assert_eq!(id, "ADR-10000");
}
