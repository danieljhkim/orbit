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
    assert_eq!(applied.len(), SUPPORTED_SCHEMA_VERSION as usize);
    assert_eq!(applied[0].version, 1);
    assert_eq!(applied[0].name, "baseline");
    assert!(!applied[0].applied_at.is_empty());
    assert_eq!(applied[1].version, 2);
    assert_eq!(applied[1].name, "learnings_index_workspace_scope");
    assert!(!applied[1].applied_at.is_empty());
    assert_eq!(applied[2].version, 3);
    assert_eq!(applied[2].name, "flat_crew_model");
    assert!(!applied[2].applied_at.is_empty());
    assert_eq!(applied[3].version, 4);
    assert_eq!(applied[3].name, "job_run_archive_stage");
    assert!(!applied[3].applied_at.is_empty());
    assert_eq!(applied[4].version, 5);
    assert_eq!(applied[4].name, "host_registry_core");
    assert!(!applied[4].applied_at.is_empty());
    assert_eq!(applied[5].version, 6);
    assert_eq!(applied[5].name, "workspace_coordination_projections");
    assert!(!applied[5].applied_at.is_empty());
    assert_eq!(applied[6].version, 7);
    assert_eq!(applied[6].name, "trusted_mcp_audit_provenance");
    assert!(!applied[6].applied_at.is_empty());
    assert_eq!(applied[7].version, 8);
    assert_eq!(applied[7].name, "hub_registry_metadata");
    assert!(!applied[7].applied_at.is_empty());
    assert_eq!(applied[8].version, 9);
    assert_eq!(applied[8].name, "feature_schema_ledger");
    assert!(!applied[8].applied_at.is_empty());
}

#[test]
fn reapplying_schema_is_a_noop() {
    let conn = Connection::open_in_memory().expect("open in-memory connection");

    apply_schema(&conn).expect("first apply");
    let first = applied_migrations(&conn).expect("applied after first apply");
    apply_schema(&conn).expect("second apply");
    let second = applied_migrations(&conn).expect("applied after second apply");

    assert_eq!(first, second);
    assert_eq!(ledger_rows(&conn).len(), SUPPORTED_SCHEMA_VERSION as usize);
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
        vec![
            ("migration.v0001".to_string(), "baseline".to_string()),
            (
                "migration.v0002".to_string(),
                "learnings_index_workspace_scope".to_string()
            ),
            ("migration.v0003".to_string(), "flat_crew_model".to_string()),
            (
                "migration.v0004".to_string(),
                "job_run_archive_stage".to_string()
            ),
            (
                "migration.v0005".to_string(),
                "host_registry_core".to_string()
            ),
            (
                "migration.v0006".to_string(),
                "workspace_coordination_projections".to_string()
            ),
            (
                "migration.v0007".to_string(),
                "trusted_mcp_audit_provenance".to_string()
            ),
            (
                "migration.v0008".to_string(),
                "hub_registry_metadata".to_string()
            ),
            (
                "migration.v0009".to_string(),
                "feature_schema_ledger".to_string()
            ),
        ]
    );
}

#[test]
fn refuses_db_from_a_newer_binary() {
    let conn = Connection::open_in_memory().expect("open in-memory connection");
    apply_schema(&conn).expect("apply schema");

    conn.execute(
        "INSERT INTO schema_meta(key, value, updated_at)
         VALUES ('migration.v0010', 'from-the-future', '2099-01-01T00:00:00Z')",
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
fn store_reopens_database_at_shipped_schema_v4_and_applies_through_latest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("orbit.db");

    // Model a database last opened by the shipped v4 binary. V5 is additive
    // and must preserve that schema while installing the host registry.
    let conn = Connection::open(&path).expect("open raw store connection");
    conn.execute_batch(
        r#"
            CREATE TABLE schema_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            INSERT INTO schema_meta VALUES
                ('migration.v0001', 'baseline', '2026-07-01T00:00:00Z'),
                ('migration.v0002', 'learnings_index_workspace_scope', '2026-07-02T00:00:00Z'),
                ('migration.v0003', 'flat_crew_model', '2026-07-03T00:00:00Z'),
                ('migration.v0004', 'job_run_archive_stage', '2026-07-04T00:00:00Z');
            CREATE TABLE audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                execution_id TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                command TEXT NOT NULL,
                subcommand TEXT,
                tool_name TEXT,
                target_type TEXT,
                target_id TEXT,
                role TEXT NOT NULL,
                status TEXT NOT NULL,
                exit_code INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                working_directory TEXT NOT NULL,
                arguments_json TEXT,
                stdout_truncated TEXT,
                stderr_truncated TEXT,
                error_message TEXT,
                host TEXT,
                pid INTEGER NOT NULL,
                session_id TEXT,
                task_id TEXT,
                job_run_id TEXT,
                activity_id TEXT,
                step_index INTEGER
            );
            CREATE TABLE job_runs (id TEXT PRIMARY KEY, archived_at TEXT);
            INSERT INTO job_runs(id, archived_at) VALUES ('preserved-run', NULL);
        "#,
    )
    .expect("seed shipped v4 database");
    drop(conn);

    let store = crate::Store::open(&path).expect("reopen shipped v4 store");
    assert_eq!(store.schema_version().expect("schema version"), 9);
    let applied = store.applied_migrations().expect("applied migrations");
    assert_eq!(applied.last().map(|migration| migration.version), Some(9));
    assert_eq!(
        applied.last().map(|migration| migration.name.as_str()),
        Some("feature_schema_ledger")
    );
    let connection = store.connection();
    let conn = connection.lock().expect("connection");
    assert!(table_exists(&conn, "hosts").expect("hosts table"));
    assert!(table_exists(&conn, "host_aliases").expect("aliases table"));
    assert!(table_exists(&conn, "workspace_ownership").expect("ownership table"));
    let preserved: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM job_runs WHERE id = 'preserved-run'",
            [],
            |row| row.get(0),
        )
        .expect("preserved v4 record");
    assert_eq!(preserved, 1);
}

#[test]
fn store_reopens_shipped_v6_audit_rows_and_applies_v7_additively() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("orbit.db");
    let conn = Connection::open(&path).expect("open raw store connection");
    conn.execute_batch(
        r#"
            CREATE TABLE schema_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            INSERT INTO schema_meta VALUES
                ('migration.v0001', 'baseline', '2026-07-01T00:00:00Z'),
                ('migration.v0002', 'learnings_index_workspace_scope', '2026-07-02T00:00:00Z'),
                ('migration.v0003', 'flat_crew_model', '2026-07-03T00:00:00Z'),
                ('migration.v0004', 'job_run_archive_stage', '2026-07-04T00:00:00Z'),
                ('migration.v0005', 'host_registry_core', '2026-07-05T00:00:00Z'),
                ('migration.v0006', 'workspace_coordination_projections', '2026-07-06T00:00:00Z');
            CREATE TABLE audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                execution_id TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                command TEXT NOT NULL,
                subcommand TEXT,
                tool_name TEXT,
                target_type TEXT,
                target_id TEXT,
                role TEXT NOT NULL,
                status TEXT NOT NULL,
                exit_code INTEGER NOT NULL,
                duration_ms INTEGER NOT NULL,
                working_directory TEXT NOT NULL,
                arguments_json TEXT,
                stdout_truncated TEXT,
                stderr_truncated TEXT,
                error_message TEXT,
                host TEXT,
                pid INTEGER NOT NULL,
                session_id TEXT,
                task_id TEXT,
                job_run_id TEXT,
                activity_id TEXT,
                step_index INTEGER
            );
            INSERT INTO audit_events(
                execution_id, timestamp, command, role, status, exit_code,
                duration_ms, working_directory, host, pid, session_id, task_id,
                job_run_id, activity_id, step_index
            ) VALUES (
                'exec-v6', '2026-07-06T00:00:00Z', 'tool', 'codex', 'success', 0,
                1, '/repo', 'legacy-process-host', 42, 'legacy-session', 'ORB-10228',
                'jrun-v6', 'agent_implement', 3
            );
        "#,
    )
    .expect("seed shipped v6 database");
    drop(conn);

    let store = crate::Store::open(&path).expect("open and migrate v6 store");
    assert_eq!(store.schema_version().expect("schema version"), 9);
    let rows = store
        .list_audit_events(&crate::AuditEventFilter::default())
        .expect("read migrated audit rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].host.as_deref(), Some("legacy-process-host"));
    assert_eq!(rows[0].session_id.as_deref(), Some("legacy-session"));
    assert_eq!(rows[0].job_run_id.as_deref(), Some("jrun-v6"));
    assert_eq!(rows[0].workspace_id, None);
    assert!(rows[0].effective_capabilities.is_empty());
    assert_eq!(rows[0].mcp_call_id, None);
    drop(store);

    let reopened = crate::Store::open(&path).expect("reopen migrated store");
    assert_eq!(reopened.schema_version().expect("schema version"), 9);
    assert_eq!(
        reopened
            .list_audit_events(&crate::AuditEventFilter::default())
            .expect("read after reopen")
            .len(),
        1
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
    assert_eq!(applied.len(), SUPPORTED_SCHEMA_VERSION as usize);
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
