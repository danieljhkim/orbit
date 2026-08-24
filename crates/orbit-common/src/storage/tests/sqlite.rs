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

#[cfg(unix)]
#[test]
fn private_open_repairs_existing_database_and_sidecars() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("repair.db");
    let connection = Connection::open(&path).expect("open fixture");
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .expect("enable WAL");
    connection
        .execute_batch("CREATE TABLE fixture(value TEXT); INSERT INTO fixture VALUES ('secret');")
        .expect("write fixture");

    for suffix in ["", "-wal", "-shm"] {
        let file = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        assert!(file.exists(), "fixture sidecar exists: {}", file.display());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o666))
            .expect("make fixture permissive");
    }

    let opened = super::super::sqlite::open_private(&path).expect("open private");
    assert!(!opened.read_only);
    for suffix in ["", "-wal", "-shm"] {
        let file = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        let mode = std::fs::metadata(&file)
            .expect("file metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "private mode for {}", file.display());
    }
}

#[cfg(unix)]
#[test]
fn private_open_preserves_an_immutable_database_without_side_effects() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("immutable.db");
    let connection = Connection::open(&path).expect("open fixture");
    connection
        .execute_batch("CREATE TABLE fixture(value TEXT); INSERT INTO fixture VALUES ('kept');")
        .expect("write fixture");
    drop(connection);
    for suffix in ["-wal", "-shm"] {
        let sidecar = std::path::PathBuf::from(format!("{}{suffix}", path.display()));
        if sidecar.exists() {
            std::fs::remove_file(sidecar).expect("remove fixture sidecar");
        }
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400))
        .expect("make fixture read-only");

    let opened = super::super::sqlite::open_private(&path).expect("open immutable");
    assert!(opened.read_only);
    let value = opened
        .connection
        .query_row("SELECT value FROM fixture", [], |row| {
            row.get::<_, String>(0)
        })
        .expect("read immutable fixture");
    assert_eq!(value, "kept");
    assert_eq!(
        std::fs::metadata(&path)
            .expect("database metadata")
            .permissions()
            .mode()
            & 0o777,
        0o400,
        "read-only mode must not be repaired"
    );
    assert!(!std::path::PathBuf::from(format!("{}-wal", path.display())).exists());
    assert!(!std::path::PathBuf::from(format!("{}-shm", path.display())).exists());
}
