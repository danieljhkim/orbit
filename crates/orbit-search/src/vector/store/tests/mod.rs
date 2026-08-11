//! Unit tests for `store` (vector SQLite index), split by source file per
//! `docs/design-patterns/test_layout.md` (ORB-00230 sibling migration).
//!
//! The parent `store/mod.rs` declares `#[cfg(test)] mod tests;`.

#![allow(missing_docs)]

mod docs;
mod learning;
mod queries;
mod schema;
mod tasks;
mod upsert;

/// [ORB-10004] `VectorStore::open` must apply the shared Orbit connection
/// defaults from `orbit_common::utility::sqlite` — historically this store
/// drifted (it was missing `foreign_keys` and `synchronous`).
mod open_pragmas {
    use crate::vector::store::VectorStore;

    #[test]
    fn open_applies_shared_pragma_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = VectorStore::open(&dir.path().join("semantic.db")).expect("open store");

        let conn_arc = store.connection();
        let conn = conn_arc.lock().expect("lock connection");
        let journal_mode = conn
            .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
            .expect("journal_mode");
        assert_eq!(journal_mode.to_lowercase(), "wal");
        let pragma_i64 = |name: &str| -> i64 {
            conn.pragma_query_value(None, name, |row| row.get::<_, i64>(0))
                .expect("query pragma")
        };
        assert_eq!(pragma_i64("busy_timeout"), 5_000);
        assert_eq!(pragma_i64("foreign_keys"), 1);
        // synchronous=NORMAL reports as 1.
        assert_eq!(pragma_i64("synchronous"), 1);
    }
}
