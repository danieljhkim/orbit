use orbit_types::OrbitError;
use rusqlite::Connection;

pub(crate) fn apply_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE TABLE IF NOT EXISTS tools (
                name TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
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
                session_id TEXT
            );
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    ensure_agent_sessions_schema(conn)?;
    ensure_tools_schema(conn)?;
    ensure_audit_events_schema(conn)?;

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
                session_id TEXT
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
    .map_err(|e| OrbitError::Store(e.to_string()))
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
