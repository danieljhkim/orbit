use orbit_types::OrbitError;
use rusqlite::Connection;

pub(crate) fn apply_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                instructions TEXT NOT NULL DEFAULT '',
                execution_summary TEXT NOT NULL DEFAULT '',
                context_files TEXT NOT NULL DEFAULT '[]',
                workspace_path TEXT,
                assigned_to TEXT,
                created_by TEXT,
                status TEXT NOT NULL DEFAULT 'backlog',
                priority TEXT NOT NULL DEFAULT 'medium',
                task_type TEXT NOT NULL DEFAULT 'task',
                branch TEXT,
                pr_number TEXT,
                proposed_by TEXT,
                proposal_approved_by TEXT,
                proposal_rejected_by TEXT,
                proposal_decision_note TEXT,
                review_approved_by TEXT,
                review_rejected_by TEXT,
                review_decision_note TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS memos (
                id TEXT PRIMARY KEY,
                body TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS audits (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                message TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS locks (
                name TEXT PRIMARY KEY,
                owner TEXT NOT NULL,
                acquired_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS tools (
                name TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 1,
                builtin INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS skills (
                schema_version INTEGER NOT NULL,
                name TEXT PRIMARY KEY,
                description TEXT,
                instructions TEXT NOT NULL,
                context_files TEXT NOT NULL DEFAULT '[]',
                allowed_tools TEXT NOT NULL DEFAULT '[]',
                role TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS task_skills (
                task_id TEXT NOT NULL,
                skill_name TEXT NOT NULL,
                attachment_order INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (task_id, skill_name),
                FOREIGN KEY(skill_name) REFERENCES skills(name) ON DELETE CASCADE
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
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    ensure_tasks_schema(conn)?;
    ensure_task_metadata_schema(conn)?;
    ensure_tools_schema(conn)?;
    ensure_audit_events_schema(conn)?;
    ensure_no_legacy_watch_state(conn)?;

    Ok(())
}

fn ensure_task_metadata_schema(conn: &Connection) -> Result<(), OrbitError> {
    if table_exists(conn, "task_skills")? && table_has_foreign_key_to(conn, "task_skills", "tasks")?
    {
        conn.execute_batch(
            r#"
                ALTER TABLE task_skills RENAME TO task_skills_legacy;

                CREATE TABLE task_skills (
                    task_id TEXT NOT NULL,
                    skill_name TEXT NOT NULL,
                    attachment_order INTEGER NOT NULL,
                    created_at TEXT NOT NULL,
                    PRIMARY KEY (task_id, skill_name),
                    FOREIGN KEY(skill_name) REFERENCES skills(name) ON DELETE CASCADE
                );

                INSERT INTO task_skills(task_id, skill_name, attachment_order, created_at)
                SELECT task_id, skill_name, attachment_order, created_at
                FROM task_skills_legacy;

                DROP TABLE task_skills_legacy;
            "#,
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    }

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

fn ensure_tasks_schema(conn: &Connection) -> Result<(), OrbitError> {
    add_column_if_missing(
        conn,
        "ALTER TABLE tasks ADD COLUMN instructions TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tasks ADD COLUMN execution_summary TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tasks ADD COLUMN context_files TEXT NOT NULL DEFAULT '[]'",
    )?;
    add_column_if_missing(conn, "ALTER TABLE tasks ADD COLUMN workspace_path TEXT")?;
    add_column_if_missing(conn, "ALTER TABLE tasks ADD COLUMN identity_id TEXT")?;
    add_column_if_missing(conn, "ALTER TABLE tasks ADD COLUMN assigned_to TEXT")?;
    add_column_if_missing(conn, "ALTER TABLE tasks ADD COLUMN created_by TEXT")?;
    add_column_if_missing(conn, "ALTER TABLE tasks ADD COLUMN approved_at TEXT")?;
    add_column_if_missing(conn, "ALTER TABLE tasks ADD COLUMN approved_by TEXT")?;
    add_column_if_missing(conn, "ALTER TABLE tasks ADD COLUMN approval_note TEXT")?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tasks ADD COLUMN status TEXT NOT NULL DEFAULT 'todo'",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tasks ADD COLUMN priority TEXT NOT NULL DEFAULT 'medium'",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tasks ADD COLUMN task_type TEXT NOT NULL DEFAULT 'task'",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tasks ADD COLUMN owner TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(conn, "ALTER TABLE tasks ADD COLUMN parent_id TEXT")?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tasks ADD COLUMN created_at TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tasks ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        conn,
        "ALTER TABLE tasks ADD COLUMN proposal_rejected_by TEXT",
    )?;
    add_column_if_missing(conn, "ALTER TABLE tasks ADD COLUMN review_rejected_by TEXT")?;

    if table_has_column(conn, "tasks", "type")? {
        conn.execute(
            r#"
                UPDATE tasks
                SET task_type = type
                WHERE task_type = 'task'
                  AND trim(COALESCE(type, '')) != ''
            "#,
            [],
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    }

    conn.execute(
        "UPDATE tasks SET created_at = datetime('now') WHERE created_at = ''",
        [],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;
    conn.execute(
        "UPDATE tasks SET updated_at = datetime('now') WHERE updated_at = ''",
        [],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    Ok(())
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

fn ensure_no_legacy_watch_state(conn: &Connection) -> Result<(), OrbitError> {
    if !table_exists(conn, "watches")? {
        return Ok(());
    }

    let watch_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM watches", [], |row| row.get(0))
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    if watch_count == 0 {
        return Ok(());
    }

    Err(OrbitError::InvalidInput(
        "legacy watch persistence is no longer supported; remove rows from the `watches` table before running this Orbit version"
            .to_string(),
    ))
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
    use super::{apply_schema, table_has_foreign_key_to};
    use rusqlite::Connection;

    #[test]
    fn apply_schema_keeps_sqlite_bootstrap_audit_focused() {
        let conn = Connection::open_in_memory().expect("open");

        apply_schema(&conn).expect("apply schema");

        let jobs_table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='jobs'",
                [],
                |row| row.get(0),
            )
            .expect("query jobs");
        let activities_table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='activities'",
                [],
                |row| row.get(0),
            )
            .expect("query activities");

        assert_eq!(jobs_table_exists, 0);
        assert_eq!(activities_table_exists, 0);
    }

    #[test]
    fn apply_schema_backfills_legacy_tools_columns() {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            r#"
                CREATE TABLE tools (
                    name TEXT PRIMARY KEY,
                    path TEXT NOT NULL,
                    description TEXT NOT NULL,
                    is_enabled INTEGER NOT NULL DEFAULT 1
                );

                INSERT INTO tools(name, path, description, is_enabled)
                VALUES ('legacy', '/bin/echo', 'legacy tool', 0);
            "#,
        )
        .expect("legacy tools");

        apply_schema(&conn).expect("apply schema");

        let enabled: i64 = conn
            .query_row(
                "SELECT enabled FROM tools WHERE name = 'legacy'",
                [],
                |row| row.get(0),
            )
            .expect("select enabled");
        let builtin: i64 = conn
            .query_row(
                "SELECT builtin FROM tools WHERE name = 'legacy'",
                [],
                |row| row.get(0),
            )
            .expect("select builtin");

        assert_eq!(enabled, 0);
        assert_eq!(builtin, 0);
    }

    #[test]
    fn apply_schema_backfills_legacy_tasks_columns() {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            r#"
                CREATE TABLE tasks (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    description TEXT NOT NULL DEFAULT '',
                    type TEXT NOT NULL DEFAULT 'feature'
                );

                INSERT INTO tasks(id, title, description, type)
                VALUES ('task-legacy', 'legacy task', 'legacy desc', 'feature');
            "#,
        )
        .expect("legacy tasks");

        apply_schema(&conn).expect("apply schema");

        let (task_type, owner, has_status): (String, String, i64) = conn
            .query_row(
                "SELECT task_type, owner, CASE WHEN status = 'todo' THEN 1 ELSE 0 END FROM tasks WHERE id = 'task-legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("query migrated task");
        assert_eq!(task_type, "feature");
        assert_eq!(owner, "");
        assert_eq!(has_status, 1);
    }

    #[test]
    fn apply_schema_removes_task_foreign_keys_from_task_metadata_tables() {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            r#"
                CREATE TABLE tasks (
                    id TEXT PRIMARY KEY
                );
                CREATE TABLE skills (
                    schema_version INTEGER NOT NULL,
                    name TEXT PRIMARY KEY,
                    description TEXT,
                    instructions TEXT NOT NULL,
                    context_files TEXT NOT NULL,
                    allowed_tools TEXT NOT NULL,
                    role TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE task_skills (
                    task_id TEXT NOT NULL,
                    skill_name TEXT NOT NULL,
                    attachment_order INTEGER NOT NULL,
                    created_at TEXT NOT NULL,
                    PRIMARY KEY (task_id, skill_name),
                    FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE,
                    FOREIGN KEY(skill_name) REFERENCES skills(name) ON DELETE CASCADE
                );
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
                    FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE
                );
            "#,
        )
        .expect("legacy metadata tables");

        apply_schema(&conn).expect("apply schema");

        let task_skills_has_fk =
            table_has_foreign_key_to(&conn, "task_skills", "tasks").expect("task_skills pragma");
        let agent_sessions_has_fk = table_has_foreign_key_to(&conn, "agent_sessions", "tasks")
            .expect("agent_sessions pragma");

        assert!(!task_skills_has_fk);
        assert!(!agent_sessions_has_fk);
    }

    #[test]
    fn apply_schema_does_not_create_watches_table() {
        let conn = Connection::open_in_memory().expect("open");

        apply_schema(&conn).expect("apply schema");

        let watches_table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='watches'",
                [],
                |row| row.get(0),
            )
            .expect("query watches table");
        assert_eq!(watches_table_exists, 0);
    }

    #[test]
    fn apply_schema_fails_fast_for_legacy_watch_rows() {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            r#"
                CREATE TABLE watches (
                    id TEXT PRIMARY KEY,
                    path TEXT NOT NULL,
                    command TEXT NOT NULL,
                    debounce_ms INTEGER NOT NULL,
                    updated_at TEXT NOT NULL
                );

                INSERT INTO watches(id, path, command, debounce_ms, updated_at)
                VALUES (
                    'watch-1',
                    '/tmp/input.txt',
                    'echo run',
                    500,
                    '2026-03-09T00:00:00Z'
                );
            "#,
        )
        .expect("legacy watches");

        let err = apply_schema(&conn).expect_err("must fail fast");
        assert!(
            err.to_string()
                .contains("legacy watch persistence is no longer supported")
        );
    }
}
