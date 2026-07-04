// Migrated from sqlite/migration.rs per ORB-00231
use super::super::*;

#[test]
fn task_reservation_migration_adds_owner_columns_before_owner_index() {
    let conn = Connection::open_in_memory().expect("open in-memory connection");
    conn.execute_batch(
        r#"
                CREATE TABLE task_reservations (
                    reservation_id TEXT PRIMARY KEY,
                    workspace_orbit_dir TEXT NOT NULL,
                    task_ids_json TEXT NOT NULL,
                    files_json TEXT NOT NULL,
                    actor TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    expires_at TEXT NOT NULL,
                    released_at TEXT
                );

                INSERT INTO task_reservations(
                    reservation_id,
                    workspace_orbit_dir,
                    task_ids_json,
                    files_json,
                    actor,
                    created_at,
                    expires_at,
                    released_at
                ) VALUES (
                    'reservation-legacy',
                    '/workspace/.orbit',
                    '["T1"]',
                    '["file:src/lib.rs"]',
                    'legacy',
                    '2026-05-05T00:00:00Z',
                    '2026-05-05T01:00:00Z',
                    NULL
                );
            "#,
    )
    .expect("create legacy reservation table");

    apply_schema(&conn).expect("migrate legacy reservation table");

    assert!(
        table_has_column(&conn, "task_reservations", "workspace_id").expect("workspace column")
    );
    assert!(table_has_column(&conn, "task_reservations", "owner_run_id").expect("owner column"));
    let owner_run_id: Option<String> = conn
            .query_row(
                "SELECT owner_run_id FROM task_reservations WHERE reservation_id = 'reservation-legacy'",
                [],
                |row| row.get(0),
            )
            .expect("query migrated row");
    assert_eq!(owner_run_id, None);
    let owner_index: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index'
                   AND name = 'idx_task_reservations_workspace_owner_release'",
            [],
            |row| row.get(0),
        )
        .expect("query owner index");
    assert_eq!(owner_index, 1);
}

#[test]
fn apply_schema_creates_adrs_table_and_indexes() {
    let conn = Connection::open_in_memory().expect("open in-memory connection");

    apply_schema(&conn).expect("apply schema");

    assert!(table_exists(&conn, "adrs").expect("adrs table exists"));

    let primary_key_columns: Vec<String> = conn
        .prepare("PRAGMA table_info(adrs)")
        .expect("prepare pragma")
        .query_map([], |row| {
            let name: String = row.get(1)?;
            let pk: i64 = row.get(5)?;
            Ok((name, pk))
        })
        .expect("query pragma")
        .filter_map(|row| {
            let (name, pk) = row.expect("pragma row");
            (pk > 0).then_some(name)
        })
        .collect();
    assert_eq!(primary_key_columns, vec!["id"]);
    assert!(table_has_column(&conn, "adrs", "tags").expect("tags column"));
    assert!(table_has_column(&conn, "adrs", "paths").expect("paths column"));

    for index_name in ["idx_adrs_status", "idx_adrs_owner"] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                     WHERE type = 'index' AND name = ?1",
                [index_name],
                |row| row.get(0),
            )
            .expect("query index");
        assert_eq!(count, 1, "expected index {index_name} to exist");
    }
}

// ── read-only ledger inspection for `orbit migrate --dry-run` [ORB-10012] ──

#[test]
fn read_schema_ledger_status_of_a_missing_database_lists_everything_pending() {
    let temp = tempfile::tempdir().expect("tempdir");
    let status =
        read_schema_ledger_status(&temp.path().join("orbit.db")).expect("read missing db status");

    assert_eq!(status.current_version, 0);
    assert!(!status.pending.is_empty());
    assert_eq!(
        status.pending.last().map(|m| m.version),
        Some(SUPPORTED_SCHEMA_VERSION)
    );
    // Read-only: inspecting must not create the database.
    assert!(!temp.path().join("orbit.db").exists());
}

#[test]
fn read_schema_ledger_status_of_a_migrated_database_reports_current_without_writing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("orbit.db");
    drop(crate::Store::open(&db_path).expect("open store"));

    let status = read_schema_ledger_status(&db_path).expect("read status");
    assert_eq!(status.current_version, SUPPORTED_SCHEMA_VERSION);
    assert!(status.pending.is_empty());
    assert!(pending_schema_migrations_after(SUPPORTED_SCHEMA_VERSION).is_empty());
    assert_eq!(
        pending_schema_migrations_after(0).last().map(|m| m.version),
        Some(SUPPORTED_SCHEMA_VERSION)
    );
}
