use chrono::{TimeZone, Utc};

use crate::utility::audit_pending::{
    clear, clear_published, for_each_blob_hash, is_blob_hash, list, mark, pending_dir,
};

const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn audit_root() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path().join("state/audit");
    (temp, root)
}

#[test]
fn mark_then_list_roundtrips_the_hash_and_timestamp() {
    let (_temp, root) = audit_root();
    let ts = Utc.with_ymd_and_hms(2026, 7, 12, 9, 30, 0).unwrap();
    mark(&root, HASH_A, ts).expect("mark");

    let markers = list(&root).expect("list");
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].hash, HASH_A);
    assert_eq!(markers[0].created, Some(ts));
}

#[test]
fn list_is_empty_when_the_pending_dir_is_absent() {
    let (_temp, root) = audit_root();
    assert!(list(&root).expect("list").is_empty());
}

#[test]
fn mark_ignores_non_hash_tokens() {
    let (_temp, root) = audit_root();
    // An `error:`-prefixed write-failure sentinel must never become a marker.
    mark(&root, "error:disk full", Utc::now()).expect("mark");
    mark(&root, "short", Utc::now()).expect("mark");
    assert!(list(&root).expect("list").is_empty());
}

#[test]
fn clear_removes_only_the_named_marker() {
    let (_temp, root) = audit_root();
    mark(&root, HASH_A, Utc::now()).expect("mark a");
    mark(&root, HASH_B, Utc::now()).expect("mark b");

    clear(&root, HASH_A).expect("clear a");

    let remaining: Vec<String> = list(&root)
        .expect("list")
        .into_iter()
        .map(|m| m.hash)
        .collect();
    assert_eq!(remaining, vec![HASH_B.to_string()]);
    // Clearing an absent marker is not an error.
    clear(&root, HASH_A).expect("clear absent");
}

#[test]
fn clear_published_clears_exactly_the_referenced_hashes() {
    let (_temp, root) = audit_root();
    mark(&root, HASH_A, Utc::now()).expect("mark a");
    mark(&root, HASH_B, Utc::now()).expect("mark b");

    // A serialized envelope that references only HASH_A.
    clear_published(&root, &format!(r#"{{"stdin_blob_ref":"{HASH_A}"}}"#));

    let remaining: Vec<String> = list(&root)
        .expect("list")
        .into_iter()
        .map(|m| m.hash)
        .collect();
    assert_eq!(remaining, vec![HASH_B.to_string()]);
}

#[test]
fn list_skips_non_marker_files() {
    let (_temp, root) = audit_root();
    mark(&root, HASH_A, Utc::now()).expect("mark");
    // A stray non-hash file in the pending dir is ignored, not surfaced.
    std::fs::write(pending_dir(&root).join("README"), b"ignore me").expect("write stray");

    let markers = list(&root).expect("list");
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].hash, HASH_A);
}

#[test]
fn list_reports_malformed_marker_as_created_none() {
    let (_temp, root) = audit_root();
    std::fs::create_dir_all(pending_dir(&root)).expect("dir");
    std::fs::write(pending_dir(&root).join(HASH_A), b"not-a-timestamp").expect("torn marker");

    let markers = list(&root).expect("list");
    assert_eq!(markers.len(), 1);
    assert!(
        markers[0].created.is_none(),
        "an unparseable marker must fail closed with an unknown age"
    );
}

#[test]
fn for_each_blob_hash_lowercases_and_filters() {
    let mut found = Vec::new();
    for_each_blob_hash(
        &format!("ref={} noise=zzz other={}", HASH_A.to_uppercase(), HASH_B),
        |hash| found.push(hash),
    );
    found.sort();
    assert_eq!(found, vec![HASH_A.to_string(), HASH_B.to_string()]);
    assert!(is_blob_hash(HASH_A));
    assert!(!is_blob_hash("zzzz"));
}
