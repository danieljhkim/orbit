//! Sibling tests for `sqlite/connection.rs` health probes [ORB-10005].

use crate::Store;
use crate::sqlite::migration::SUPPORTED_SCHEMA_VERSION;

#[test]
fn quick_check_passes_on_fresh_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(&dir.path().join("store.db")).expect("open store");
    store.quick_check().expect("fresh store passes quick_check");
}

#[test]
fn quick_check_passes_in_memory() {
    let store = Store::open_in_memory().expect("open in-memory store");
    store
        .quick_check()
        .expect("in-memory store passes quick_check");
}

#[test]
fn check_writable_acquires_and_releases_write_lock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(&dir.path().join("store.db")).expect("open store");

    store.check_writable().expect("writable store passes");
    // The probe rolled back: the write lock is free again immediately.
    store.check_writable().expect("probe is repeatable");
    // And the store still accepts real transactions afterwards.
    store
        .with_transaction(|_| Ok(()))
        .expect("store still accepts transactions after the probe");
}

#[test]
fn quick_check_reports_page_corruption() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("store.db");

    // Build a multi-page database, then close every connection.
    {
        let store = Store::open(&path).expect("open store");
        store
            .with_transaction(|tx| {
                tx.tx
                    .execute_batch("CREATE TABLE corruption_fixture(payload TEXT)")
                    .map_err(|e| orbit_common::types::OrbitError::Store(e.to_string()))?;
                for _ in 0..64 {
                    tx.tx
                        .execute(
                            "INSERT INTO corruption_fixture VALUES (hex(randomblob(512)))",
                            [],
                        )
                        .map_err(|e| orbit_common::types::OrbitError::Store(e.to_string()))?;
                }
                Ok(())
            })
            .expect("seed fixture rows");
    }
    // Checkpoint the WAL into the main file so on-disk bytes are canonical.
    {
        let conn = rusqlite::Connection::open(&path).expect("open raw");
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .expect("checkpoint");
    }

    // Clobber the header of the last b-tree page (page 1 — the file header
    // and sqlite_master — stays intact so the database still opens). The
    // page header carries the page type and cell pointers, so structural
    // validation must notice; flipping payload bytes alone would not.
    let mut bytes = std::fs::read(&path).expect("read db bytes");
    let page_size = usize::from(u16::from_be_bytes([bytes[16], bytes[17]]));
    assert!(
        bytes.len() >= page_size * 3,
        "fixture must span multiple pages (len {}, page size {page_size})",
        bytes.len()
    );
    let last_page_start = (bytes.len() / page_size - 1) * page_size;
    for byte in &mut bytes[last_page_start..last_page_start + 64] {
        *byte ^= 0xFF;
    }
    std::fs::write(&path, bytes).expect("write corrupted db");

    let store = Store::open(&path).expect("corrupted db still opens");
    let err = store
        .quick_check()
        .expect_err("quick_check must flag the corrupted page");
    assert!(
        err.to_string().contains("quick_check"),
        "error names the failing probe: {err}"
    );
}

#[test]
fn schema_version_matches_binary_after_open() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(&dir.path().join("store.db")).expect("open store");
    assert_eq!(
        store.schema_version().expect("schema version"),
        SUPPORTED_SCHEMA_VERSION,
        "a freshly-opened store is migrated to the binary's schema version"
    );
}
