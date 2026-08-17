use rusqlite::Connection;

use super::super::sqlite::{DEFAULT_BUSY_TIMEOUT_MS, apply_default_pragmas};

fn pragma_i64(conn: &Connection, name: &str) -> i64 {
    conn.pragma_query_value(None, name, |row| row.get::<_, i64>(0))
        .expect("query pragma")
}

fn pragma_string(conn: &Connection, name: &str) -> String {
    conn.pragma_query_value(None, name, |row| row.get::<_, String>(0))
        .expect("query pragma")
}

#[test]
fn file_backed_connection_gets_all_defaults() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = Connection::open(dir.path().join("defaults.db")).expect("open");

    let outcome = apply_default_pragmas(&conn).expect("apply defaults");

    assert!(
        outcome.wal_active(),
        "journal_mode: {}",
        outcome.journal_mode
    );
    assert_eq!(pragma_string(&conn, "journal_mode").to_lowercase(), "wal");
    assert_eq!(
        pragma_i64(&conn, "busy_timeout"),
        i64::from(DEFAULT_BUSY_TIMEOUT_MS)
    );
    assert_eq!(pragma_i64(&conn, "foreign_keys"), 1);
    // synchronous=NORMAL is reported as 1.
    assert_eq!(pragma_i64(&conn, "synchronous"), 1);
}

#[test]
fn in_memory_connection_keeps_memory_journal_without_error() {
    let conn = Connection::open_in_memory().expect("open in-memory");

    let outcome = apply_default_pragmas(&conn).expect("apply defaults");

    assert!(!outcome.wal_active());
    assert_eq!(outcome.journal_mode.to_lowercase(), "memory");
    assert_eq!(pragma_i64(&conn, "foreign_keys"), 1);
    assert_eq!(
        pragma_i64(&conn, "busy_timeout"),
        i64::from(DEFAULT_BUSY_TIMEOUT_MS)
    );
}

#[test]
fn defaults_are_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let conn = Connection::open(dir.path().join("idempotent.db")).expect("open");

    apply_default_pragmas(&conn).expect("first apply");
    let outcome = apply_default_pragmas(&conn).expect("second apply");

    assert!(outcome.wal_active());
}
