// ORB-10003: versioned schema-migration ledger.
use orbit_common::types::OrbitError;
use rusqlite::Connection;

use super::super::ledger::{self, Migration};
use super::super::*;

fn ledger_rows(conn: &Connection) -> Vec<(String, String)> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM schema_meta WHERE key LIKE 'migration.v%' ORDER BY key")
        .expect("prepare ledger query");
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query ledger rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect ledger rows")
}

#[test]
fn fresh_db_applies_baseline_and_records_ledger() {
    let conn = Connection::open_in_memory().expect("open in-memory connection");

    apply_schema(&conn).expect("apply schema on fresh db");

    assert!(table_exists(&conn, "tools").expect("tools table"));
    assert!(table_exists(&conn, "adrs").expect("adrs table"));
    assert!(table_exists(&conn, "schema_meta").expect("schema_meta table"));

    assert_eq!(
        current_schema_version(&conn).expect("current version"),
        SUPPORTED_SCHEMA_VERSION
    );
    let applied = applied_migrations(&conn).expect("applied migrations");
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].version, 1);
    assert_eq!(applied[0].name, "baseline");
    assert!(!applied[0].applied_at.is_empty());
}

#[test]
fn reapplying_schema_is_a_noop() {
    let conn = Connection::open_in_memory().expect("open in-memory connection");

    apply_schema(&conn).expect("first apply");
    let first = applied_migrations(&conn).expect("applied after first apply");
    apply_schema(&conn).expect("second apply");
    let second = applied_migrations(&conn).expect("applied after second apply");

    assert_eq!(first, second);
    assert_eq!(ledger_rows(&conn).len(), 1);
}

#[test]
fn legacy_db_adopts_versioned_ledger() {
    let conn = Connection::open_in_memory().expect("open in-memory connection");

    // Schema as the pre-ledger idempotent migrations would have left an
    // old database: legacy `tools` shape, `adrs` without tags/paths, and
    // an `agent_sessions` with a foreign key to `tasks` (the shape the
    // rename-copy-drop migration rewrites). No schema_meta table at all.
    conn.execute_batch(
        r#"
            CREATE TABLE tools (
                name TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                is_enabled INTEGER NOT NULL DEFAULT 1,
                is_builtin INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO tools(name, path, description, is_enabled, is_builtin)
            VALUES ('legacy-tool', '/bin/legacy', 'legacy tool', 0, 1);

            CREATE TABLE adrs (
                id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                title TEXT NOT NULL,
                owner TEXT NOT NULL,
                related_features TEXT NOT NULL DEFAULT '[]',
                related_tasks TEXT NOT NULL DEFAULT '[]',
                legacy_ids TEXT NOT NULL DEFAULT '[]',
                supersedes TEXT NOT NULL DEFAULT '[]',
                superseded_by TEXT,
                validation_warnings TEXT NOT NULL DEFAULT '[]',
                legacy_validation TEXT NOT NULL DEFAULT 'none',
                created_at TEXT NOT NULL,
                accepted_at TEXT,
                last_updated TEXT NOT NULL
            );

            CREATE TABLE tasks (id TEXT PRIMARY KEY);
            INSERT INTO tasks(id) VALUES ('T1');
            CREATE TABLE agent_sessions (
                session_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                skill_names TEXT NOT NULL,
                composed_context_hash TEXT NOT NULL,
                effective_allowed_tools TEXT NOT NULL,
                tool_calls TEXT NOT NULL,
                outcome TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(task_id) REFERENCES tasks(id)
            );
            INSERT INTO agent_sessions VALUES (
                's1', 'T1', '[]', 'hash', '[]', '[]', 'ok', 'done',
                '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z'
            );
        "#,
    )
    .expect("create legacy schema");

    apply_schema(&conn).expect("adopt legacy db");

    // Baseline ran idempotently: new columns exist and legacy data survived.
    assert!(table_has_column(&conn, "tools", "enabled").expect("enabled column"));
    assert!(table_has_column(&conn, "adrs", "tags").expect("tags column"));
    assert!(table_has_column(&conn, "agent_sessions", "identity_id").expect("identity column"));
    let enabled: i64 = conn
        .query_row(
            "SELECT enabled FROM tools WHERE name = 'legacy-tool'",
            [],
            |row| row.get(0),
        )
        .expect("query migrated tool");
    assert_eq!(enabled, 0);
    let session_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM agent_sessions WHERE session_id = 's1'",
            [],
            |row| row.get(0),
        )
        .expect("query migrated session");
    assert_eq!(session_count, 1);

    // ...and the ledger now records the adoption.
    assert_eq!(
        current_schema_version(&conn).expect("current version"),
        SUPPORTED_SCHEMA_VERSION
    );
    assert_eq!(
        ledger_rows(&conn),
        vec![("migration.v0001".to_string(), "baseline".to_string())]
    );
}

#[test]
fn refuses_db_from_a_newer_binary() {
    let conn = Connection::open_in_memory().expect("open in-memory connection");
    apply_schema(&conn).expect("apply schema");

    conn.execute(
        "INSERT INTO schema_meta(key, value, updated_at)
         VALUES ('migration.v0002', 'from-the-future', '2099-01-01T00:00:00Z')",
        [],
    )
    .expect("record future migration");

    let err = apply_schema(&conn).expect_err("must refuse newer schema");
    assert!(matches!(err, OrbitError::Migration(_)), "got {err:?}");
    let message = err.to_string();
    assert!(message.contains("newer"), "unexpected message: {message}");
    assert!(
        message.contains("upgrade orbit"),
        "unexpected message: {message}"
    );
}

#[test]
fn non_migration_schema_meta_keys_are_ignored() {
    let conn = Connection::open_in_memory().expect("open in-memory connection");
    apply_schema(&conn).expect("apply schema");

    // State-import markers share the schema_meta table; they must not be
    // mistaken for ledger entries.
    conn.execute(
        "INSERT INTO schema_meta(key, value, updated_at)
         VALUES ('v2_state_import.ws_a', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        [],
    )
    .expect("record import marker");

    let applied = applied_migrations(&conn).expect("applied migrations");
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].version, 1);
    apply_schema(&conn).expect("marker must not break reopen");
}

fn migration_v1_marker(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch("CREATE TABLE ledger_test_v1 (x INTEGER)")
        .map_err(|e| OrbitError::Store(e.to_string()))
}

fn migration_v2_fails_midway(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch("CREATE TABLE ledger_test_half_applied (x INTEGER)")
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    Err(OrbitError::Migration(
        "intentional mid-migration failure".to_string(),
    ))
}

#[test]
fn failed_migration_rolls_back_schema_and_ledger() {
    let conn = Connection::open_in_memory().expect("open in-memory connection");
    let registry = [
        Migration {
            version: 1,
            name: "marker",
            apply: migration_v1_marker,
        },
        Migration {
            version: 2,
            name: "fails-midway",
            apply: migration_v2_fails_midway,
        },
    ];

    let err = ledger::run_migrations(&conn, &registry).expect_err("v2 must fail");
    assert!(matches!(err, OrbitError::Migration(_)), "got {err:?}");

    // v1 committed; v2 rolled back completely — no half-applied schema,
    // no ledger row.
    assert!(table_exists(&conn, "ledger_test_v1").expect("v1 table"));
    assert!(!table_exists(&conn, "ledger_test_half_applied").expect("v2 table rolled back"));
    assert_eq!(current_schema_version(&conn).expect("current version"), 1);
    assert_eq!(ledger_rows(&conn).len(), 1);

    // A fixed registry can resume from where the ledger left off.
    let fixed = [
        Migration {
            version: 1,
            name: "marker",
            apply: migration_v1_marker,
        },
        Migration {
            version: 2,
            name: "fixed",
            apply: migration_v1_marker_v2,
        },
    ];
    ledger::run_migrations(&conn, &fixed).expect("resume after fix");
    assert_eq!(current_schema_version(&conn).expect("current version"), 2);
}

fn migration_v1_marker_v2(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch("CREATE TABLE ledger_test_v2 (x INTEGER)")
        .map_err(|e| OrbitError::Store(e.to_string()))
}

#[test]
fn registry_last_version_matches_supported_constant() {
    // Drift guard: appending a migration requires bumping the constant.
    assert_eq!(
        ledger::MIGRATIONS.last().map(|m| m.version),
        Some(SUPPORTED_SCHEMA_VERSION)
    );
}

#[test]
fn rejects_non_increasing_registry() {
    let conn = Connection::open_in_memory().expect("open in-memory connection");
    let registry = [
        Migration {
            version: 2,
            name: "second",
            apply: migration_v1_marker,
        },
        Migration {
            version: 1,
            name: "first",
            apply: migration_v1_marker,
        },
    ];

    let err = ledger::run_migrations(&conn, &registry).expect_err("must reject registry");
    assert!(matches!(err, OrbitError::Migration(_)), "got {err:?}");
}
