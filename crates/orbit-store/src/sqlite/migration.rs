use orbit_common::types::OrbitError;
use rusqlite::Connection;

pub(crate) fn apply_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE TABLE IF NOT EXISTS tools (
                name TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                parameters_json TEXT NOT NULL DEFAULT '[]',
                enabled INTEGER NOT NULL DEFAULT 1,
                builtin INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS agent_sessions (
                session_id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                identity_id TEXT,
                identity_name TEXT,
                identity_role TEXT,
                identity_block TEXT,
                skill_names TEXT NOT NULL,
                composed_context_hash TEXT NOT NULL,
                effective_allowed_tools TEXT NOT NULL,
                tool_calls TEXT NOT NULL,
                outcome TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS audit_events (
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

            CREATE TABLE IF NOT EXISTS task_reservations (
                reservation_id TEXT PRIMARY KEY,
                workspace_orbit_dir TEXT NOT NULL,
                workspace_id TEXT,
                task_ids_json TEXT NOT NULL,
                files_json TEXT NOT NULL,
                actor TEXT NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                released_at TEXT,
                owner_run_id TEXT,
                owner_metadata_json TEXT,
                release_reason TEXT,
                release_metadata_json TEXT
            );

            CREATE TABLE IF NOT EXISTS invocations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts TEXT NOT NULL,
                job_run_id TEXT NOT NULL,
                activity_id TEXT NOT NULL,
                agent TEXT NOT NULL,
                model TEXT,
                slot TEXT,
                duration_ms INTEGER NOT NULL DEFAULT 0,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_create_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                tool_call_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS invocation_tasks (
                invocation_id INTEGER NOT NULL,
                task_id TEXT NOT NULL,
                PRIMARY KEY(invocation_id, task_id),
                FOREIGN KEY(invocation_id) REFERENCES invocations(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS tool_calls (
                invocation_id INTEGER NOT NULL,
                seq INTEGER NOT NULL,
                tool_name TEXT NOT NULL,
                result_bytes INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(invocation_id, seq),
                FOREIGN KEY(invocation_id) REFERENCES invocations(id) ON DELETE CASCADE
            );

            -- ADR envelope index. Bodies live on disk under <root>/<state>/<id>/body.md;
            -- this table indexes the YAML envelope fields for filter queries.
            -- Arrays (related_features, related_tasks, legacy_ids, supersedes,
            -- validation_warnings) are stored as JSON-encoded strings so filters
            -- can use `LIKE '%"<value>"%'` until the corpus warrants junction
            -- tables. Per ADR-010 in docs/design/adr-artifact, FTS5 over body
            -- content is owned by `orbit-embed::vector`, not this schema.
            CREATE TABLE IF NOT EXISTS adrs (
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

            CREATE INDEX IF NOT EXISTS idx_adrs_status ON adrs(status);
            CREATE INDEX IF NOT EXISTS idx_adrs_owner ON adrs(owner);
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    ensure_agent_sessions_schema(conn)?;
    ensure_tools_schema(conn)?;
    ensure_audit_events_schema(conn)?;
    ensure_task_reservations_schema(conn)?;
    ensure_learning_index_schema(conn)?;
    ensure_invocation_schema(conn)?;

    Ok(())
}

fn ensure_learning_index_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            -- Project-learnings envelope index. YAML records live on disk under
            -- `<root>/<id>/learning.yaml`; status lives in the YAML body.
            -- this table indexes the envelope fields for fast scope-glob
            -- lookups. Arrays are stored as JSON strings for the same reason
            -- the ADR index does it: phase-1 corpora are small and a junction
            -- table is overkill. Per ADR-004, ranking and FTS over body
            -- content are deferred to phase 2.
            CREATE TABLE IF NOT EXISTS learnings_index (
                id          TEXT PRIMARY KEY,
                status      TEXT NOT NULL,
                paths       TEXT NOT NULL,
                tags        TEXT NOT NULL,
                summary     TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS learnings_active
                ON learnings_index(status) WHERE status = 'active';
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    // C2 (T20260511-6) adds an optional `priority` column used as the
    // secondary ranking key in `search`. NULL is acceptable; the search
    // path orders Some(N) ahead of None and falls back to updated_at.
    add_column_if_missing(
        conn,
        "ALTER TABLE learnings_index ADD COLUMN priority INTEGER",
    )?;

    Ok(())
}

fn ensure_agent_sessions_schema(conn: &Connection) -> Result<(), OrbitError> {
    if table_exists(conn, "agent_sessions")?
        && table_has_foreign_key_to(conn, "agent_sessions", "tasks")?
    {
        conn.execute_batch(
            r#"
                ALTER TABLE agent_sessions RENAME TO agent_sessions_legacy;

                CREATE TABLE agent_sessions (
                    session_id TEXT PRIMARY KEY,
                    task_id TEXT NOT NULL,
                    identity_id TEXT,
                    identity_name TEXT,
                    identity_role TEXT,
                    identity_block TEXT,
                    skill_names TEXT NOT NULL,
                    composed_context_hash TEXT NOT NULL,
                    effective_allowed_tools TEXT NOT NULL,
                    tool_calls TEXT NOT NULL,
                    outcome TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                INSERT INTO agent_sessions(
                    session_id, task_id, identity_id, identity_name, identity_role, identity_block, skill_names, composed_context_hash, effective_allowed_tools,
                    tool_calls, outcome, status, created_at, updated_at
                )
                SELECT
                    session_id, task_id, NULL, NULL, NULL, NULL, skill_names, composed_context_hash, effective_allowed_tools,
                    tool_calls, outcome, status, created_at, updated_at
                FROM agent_sessions_legacy;

                DROP TABLE agent_sessions_legacy;
            "#,
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    }

    add_column_if_missing(
        conn,
        "ALTER TABLE agent_sessions ADD COLUMN identity_id TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE agent_sessions ADD COLUMN identity_name TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE agent_sessions ADD COLUMN identity_role TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE agent_sessions ADD COLUMN identity_block TEXT",
    )?;

    Ok(())
}

fn add_column_if_missing(conn: &Connection, sql: &str) -> Result<(), OrbitError> {
    match conn.execute(sql, []) {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("duplicate column name") => Ok(()),
        Err(e) => Err(OrbitError::Store(e.to_string())),
    }
}

fn ensure_tools_schema(conn: &Connection) -> Result<(), OrbitError> {
    add_column_if_missing(
        conn,
        "ALTER TABLE tools ADD COLUMN parameters_json TEXT NOT NULL DEFAULT '[]'",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tools ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tools ADD COLUMN builtin INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tools ADD COLUMN created_at TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tools ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''",
    )?;

    if table_has_column(conn, "tools", "is_enabled")? {
        conn.execute(
            r#"
                UPDATE tools
                SET enabled = CASE
                    WHEN lower(CAST(is_enabled AS TEXT)) IN ('0', 'false', 'f', 'no') THEN 0
                    ELSE 1
                END
            "#,
            [],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    }

    if table_has_column(conn, "tools", "is_builtin")? {
        conn.execute(
            r#"
                UPDATE tools
                SET builtin = CASE
                    WHEN lower(CAST(is_builtin AS TEXT)) IN ('1', 'true', 't', 'yes') THEN 1
                    ELSE 0
                END
            "#,
            [],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    }

    conn.execute(
        "UPDATE tools SET parameters_json = '[]' WHERE parameters_json = ''",
        [],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;
    conn.execute(
        "UPDATE tools SET created_at = datetime('now') WHERE created_at = ''",
        [],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;
    conn.execute(
        "UPDATE tools SET updated_at = datetime('now') WHERE updated_at = ''",
        [],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    Ok(())
}

fn ensure_audit_events_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE TABLE IF NOT EXISTS audit_events (
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

            CREATE INDEX IF NOT EXISTS idx_audit_events_timestamp
            ON audit_events(timestamp);

            CREATE INDEX IF NOT EXISTS idx_audit_events_tool_name
            ON audit_events(tool_name);

            CREATE INDEX IF NOT EXISTS idx_audit_events_status
            ON audit_events(status);

            CREATE INDEX IF NOT EXISTS idx_audit_events_role
            ON audit_events(role);

            CREATE INDEX IF NOT EXISTS idx_audit_events_target
            ON audit_events(target_type, target_id);

            CREATE UNIQUE INDEX IF NOT EXISTS idx_audit_events_execution_id
            ON audit_events(execution_id);
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    add_column_if_missing(conn, "ALTER TABLE audit_events ADD COLUMN task_id TEXT")?;
    add_column_if_missing(conn, "ALTER TABLE audit_events ADD COLUMN job_run_id TEXT")?;
    add_column_if_missing(conn, "ALTER TABLE audit_events ADD COLUMN activity_id TEXT")?;
    add_column_if_missing(
        conn,
        "ALTER TABLE audit_events ADD COLUMN step_index INTEGER",
    )?;

    conn.execute_batch(
        r#"
            CREATE INDEX IF NOT EXISTS idx_audit_events_task_id
            ON audit_events(task_id);

            CREATE INDEX IF NOT EXISTS idx_audit_events_job_run_id
            ON audit_events(job_run_id);
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    Ok(())
}

fn ensure_invocation_schema(conn: &Connection) -> Result<(), OrbitError> {
    add_column_if_missing(conn, "ALTER TABLE invocations ADD COLUMN slot TEXT")?;
    conn.execute_batch(
        r#"
            CREATE INDEX IF NOT EXISTS idx_invocations_job_run_id
            ON invocations(job_run_id);

            CREATE INDEX IF NOT EXISTS idx_invocations_activity_id
            ON invocations(activity_id);

            CREATE INDEX IF NOT EXISTS idx_invocation_tasks_task_id
            ON invocation_tasks(task_id);

            CREATE INDEX IF NOT EXISTS idx_tool_calls_tool_name
            ON tool_calls(tool_name);
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))
}

fn ensure_task_reservations_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE TABLE IF NOT EXISTS task_reservations (
                reservation_id TEXT PRIMARY KEY,
                workspace_orbit_dir TEXT NOT NULL,
                task_ids_json TEXT NOT NULL,
                files_json TEXT NOT NULL,
                actor TEXT NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                released_at TEXT,
                owner_run_id TEXT,
                owner_metadata_json TEXT,
                release_reason TEXT,
                release_metadata_json TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_task_reservations_workspace_expires
            ON task_reservations(workspace_orbit_dir, expires_at);

            CREATE INDEX IF NOT EXISTS idx_task_reservations_workspace_release
            ON task_reservations(workspace_orbit_dir, released_at);
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    add_column_if_missing(
        conn,
        "ALTER TABLE task_reservations ADD COLUMN workspace_id TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE task_reservations ADD COLUMN owner_run_id TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE task_reservations ADD COLUMN owner_metadata_json TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE task_reservations ADD COLUMN release_reason TEXT",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE task_reservations ADD COLUMN release_metadata_json TEXT",
    )?;

    conn.execute_batch(
        r#"
            CREATE INDEX IF NOT EXISTS idx_task_reservations_workspace_owner_release
            ON task_reservations(workspace_orbit_dir, owner_run_id, released_at);

            CREATE INDEX IF NOT EXISTS idx_task_reservations_workspace_id_release
            ON task_reservations(workspace_id, released_at);

            CREATE INDEX IF NOT EXISTS idx_task_reservations_workspace_id_expires
            ON task_reservations(workspace_id, expires_at);
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, OrbitError> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?1",
            [table],
            |row| row.get(0),
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    Ok(exists > 0)
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, OrbitError> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn
        .prepare(&pragma)
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| OrbitError::Store(e.to_string()))?;

    for name in rows {
        let name = name.map_err(|e| OrbitError::Store(e.to_string()))?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn table_has_foreign_key_to(
    conn: &Connection,
    table: &str,
    referenced_table: &str,
) -> Result<bool, OrbitError> {
    let pragma = format!("PRAGMA foreign_key_list({table})");
    let mut stmt = conn
        .prepare(&pragma)
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(2))
        .map_err(|e| OrbitError::Store(e.to_string()))?;

    for name in rows {
        let name = name.map_err(|e| OrbitError::Store(e.to_string()))?;
        if name == referenced_table {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(
            table_has_column(&conn, "task_reservations", "owner_run_id").expect("owner column")
        );
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
}
