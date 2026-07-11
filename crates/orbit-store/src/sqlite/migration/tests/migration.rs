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

#[test]
fn learnings_index_migration_rekeys_by_workspace_and_discards_legacy_rows() {
    let conn = Connection::open_in_memory().expect("open in-memory connection");
    // Simulate a legacy database whose learning envelope index is keyed only
    // by id — no workspace discriminator — carrying a row that cannot be
    // attributed to any workspace (the `dk1` shape from ORB-10113).
    conn.execute_batch(
        r#"
            CREATE TABLE learnings_index (
                id          TEXT PRIMARY KEY,
                status      TEXT NOT NULL,
                paths       TEXT NOT NULL,
                tags        TEXT NOT NULL,
                summary     TEXT NOT NULL,
                updated_at  TEXT NOT NULL,
                priority    INTEGER
            );
            INSERT INTO learnings_index (id, status, paths, tags, summary, updated_at, priority)
            VALUES ('L-0002', 'active', '[]', '[]', 'legacy orrery summary', '2026-07-11T00:00:00Z', NULL);
        "#,
    )
    .expect("seed legacy learnings_index");

    apply_schema(&conn).expect("migrate legacy learnings_index");

    // The index is now scoped: `workspace_id` exists and the primary key is
    // the composite `(workspace_id, id)`.
    assert!(
        table_has_column(&conn, "learnings_index", "workspace_id").expect("workspace_id column")
    );
    let mut primary_key: Vec<(i64, String)> = conn
        .prepare("PRAGMA table_info(learnings_index)")
        .expect("prepare pragma")
        .query_map([], |row| {
            let name: String = row.get(1)?;
            let pk: i64 = row.get(5)?;
            Ok((pk, name))
        })
        .expect("query pragma")
        .filter_map(|row| {
            let (pk, name) = row.expect("pragma row");
            (pk > 0).then_some((pk, name))
        })
        .collect();
    primary_key.sort_by_key(|(pk, _)| *pk);
    let pk_columns: Vec<String> = primary_key.into_iter().map(|(_, name)| name).collect();
    assert_eq!(pk_columns, vec!["workspace_id", "id"]);

    // Legacy rows are discarded (YAML is the source of truth; each runtime
    // rebuilds its own rows via sync), and all migrations are recorded.
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM learnings_index", [], |row| row.get(0))
        .expect("count rows");
    assert_eq!(
        remaining, 0,
        "legacy envelope rows must be discarded, not migrated"
    );
    assert_eq!(
        current_schema_version(&conn).expect("schema version"),
        SUPPORTED_SCHEMA_VERSION
    );
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
