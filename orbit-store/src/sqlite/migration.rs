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
                context_files TEXT NOT NULL DEFAULT '[]',
                workspace_path TEXT,
                identity_id TEXT,
                assigned_to TEXT,
                created_by TEXT,
                approved_at TEXT,
                approved_by TEXT,
                approval_note TEXT,
                status TEXT NOT NULL DEFAULT 'todo',
                priority TEXT NOT NULL DEFAULT 'medium',
                task_type TEXT NOT NULL DEFAULT 'task',
                owner TEXT NOT NULL DEFAULT '',
                parent_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS memos (
                id TEXT PRIMARY KEY,
                body TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS watches (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                command TEXT NOT NULL,
                debounce_ms INTEGER NOT NULL,
                updated_at TEXT NOT NULL
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
    migrate_jobs_table_to_v2(conn)?;
    normalize_job_targets_to_work(conn)?;
    ensure_job_schema_v2(conn)?;
    repair_legacy_scheduler_rows_in_jobs_table(conn)?;
    ensure_execution_targets_schema(conn)?;
    migrate_legacy_work_rows(conn)?;
    ensure_audit_events_schema(conn)?;

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

fn migrate_jobs_table_to_v2(conn: &Connection) -> Result<(), OrbitError> {
    if !table_exists(conn, "schedulers")? {
        conn.execute_batch(
            r#"
                CREATE TABLE schedulers (
                    id TEXT PRIMARY KEY,
                    target_type TEXT NOT NULL CHECK (target_type IN ('job')),
                    target_id TEXT NOT NULL,
                    schedule TEXT NOT NULL,
                    agent_cli TEXT NOT NULL,
                    timeout_seconds INTEGER NOT NULL,
                    retry_max_attempts INTEGER NOT NULL DEFAULT 0,
                    retry_backoff_strategy TEXT NOT NULL DEFAULT 'none',
                    retry_initial_delay_seconds INTEGER NOT NULL DEFAULT 0,
                    state TEXT NOT NULL CHECK (state IN ('enabled','paused','disabled')),
                    next_run_at TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
            "#,
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
        return Ok(());
    }

    if table_has_column(conn, "schedulers", "target_type")? {
        return Ok(());
    }

    if table_has_column(conn, "schedulers", "scheduler_id")? {
        conn.execute_batch(
            r#"
                ALTER TABLE schedulers RENAME TO jobs_v1;

                CREATE TABLE schedulers (
                    id TEXT PRIMARY KEY,
                    target_type TEXT NOT NULL CHECK (target_type IN ('job')),
                    target_id TEXT NOT NULL,
                    schedule TEXT NOT NULL,
                    agent_cli TEXT NOT NULL,
                    timeout_seconds INTEGER NOT NULL,
                    retry_max_attempts INTEGER NOT NULL DEFAULT 0,
                    retry_backoff_strategy TEXT NOT NULL DEFAULT 'none',
                    retry_initial_delay_seconds INTEGER NOT NULL DEFAULT 0,
                    state TEXT NOT NULL CHECK (state IN ('enabled','paused','disabled')),
                    next_run_at TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                INSERT INTO schedulers(
                    id, target_type, target_id, schedule, agent_cli, timeout_seconds,
                    retry_max_attempts, retry_backoff_strategy, retry_initial_delay_seconds,
                    state, next_run_at, created_at, updated_at
                )
                SELECT
                    scheduler_id,
                    'job',
                    CASE WHEN task_id = '' THEN scheduler_id ELSE task_id END,
                    schedule_spec,
                    'claude',
                    300,
                    0,
                    'none',
                    0,
                    CASE state
                        WHEN 'active' THEN 'enabled'
                        WHEN 'paused' THEN 'paused'
                        ELSE 'disabled'
                    END,
                    COALESCE(next_run_at, datetime('now')),
                    created_at,
                    updated_at
                FROM jobs_v1;

                DROP TABLE jobs_v1;
            "#,
        )
        .map_err(|e| OrbitError::Store(format!("failed v1 schedulers migration: {e}")))?;

        if table_exists(conn, "scheduler_sessions")? {
            migrate_job_sessions_to_job_runs(conn)?;
        }
        return Ok(());
    }

    if table_has_column(conn, "schedulers", "command")? {
        conn.execute_batch(
            r#"
                ALTER TABLE schedulers RENAME TO jobs_legacy;

                CREATE TABLE schedulers (
                    id TEXT PRIMARY KEY,
                    target_type TEXT NOT NULL CHECK (target_type IN ('job')),
                    target_id TEXT NOT NULL,
                    schedule TEXT NOT NULL,
                    agent_cli TEXT NOT NULL,
                    timeout_seconds INTEGER NOT NULL,
                    retry_max_attempts INTEGER NOT NULL DEFAULT 0,
                    retry_backoff_strategy TEXT NOT NULL DEFAULT 'none',
                    retry_initial_delay_seconds INTEGER NOT NULL DEFAULT 0,
                    state TEXT NOT NULL CHECK (state IN ('enabled','paused','disabled')),
                    next_run_at TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                INSERT INTO schedulers(
                    id, target_type, target_id, schedule, agent_cli, timeout_seconds,
                    retry_max_attempts, retry_backoff_strategy, retry_initial_delay_seconds,
                    state, next_run_at, created_at, updated_at
                )
                SELECT
                    id,
                    'job',
                    id,
                    '@daily',
                    'claude',
                    300,
                    0,
                    'none',
                    0,
                    'disabled',
                    COALESCE(next_run_at, datetime('now')),
                    COALESCE(last_run_at, next_run_at, datetime('now')),
                    COALESCE(last_run_at, next_run_at, datetime('now'))
                FROM jobs_legacy;

                DROP TABLE jobs_legacy;
            "#,
        )
        .map_err(|e| OrbitError::Store(format!("failed legacy schedulers migration: {e}")))?;
    }

    Ok(())
}

fn migrate_job_sessions_to_job_runs(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE TABLE IF NOT EXISTS scheduler_runs (
                id TEXT PRIMARY KEY,
                scheduler_id TEXT NOT NULL,
                attempt INTEGER NOT NULL,
                state TEXT NOT NULL CHECK (state IN ('pending','running','success','failed','timeout')),
                scheduled_at TEXT NOT NULL,
                started_at TEXT,
                finished_at TEXT,
                duration_ms INTEGER,
                exit_code INTEGER,
                agent_response_json TEXT,
                error_code TEXT,
                error_message TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY(scheduler_id) REFERENCES schedulers(id)
            );

            INSERT OR IGNORE INTO scheduler_runs(
                id, scheduler_id, attempt, state, scheduled_at, started_at, finished_at,
                duration_ms, exit_code, agent_response_json, error_code, error_message, created_at
            )
            SELECT
                session_id,
                scheduler_id,
                1,
                CASE status
                    WHEN 'running' THEN 'running'
                    WHEN 'succeeded' THEN 'success'
                    WHEN 'failed' THEN 'failed'
                    ELSE 'failed'
                END,
                trigger_time,
                started_at,
                finished_at,
                NULL,
                exit_code,
                NULL,
                CASE WHEN status = 'failed' THEN 'LEGACY_JOB_SESSION_FAILURE' ELSE NULL END,
                error,
                created_at
            FROM scheduler_sessions;
        "#,
    )
    .map_err(|e| OrbitError::Store(format!("failed scheduler_sessions migration: {e}")))
}

fn ensure_job_schema_v2(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE INDEX IF NOT EXISTS idx_jobs_state ON schedulers(state);
            CREATE INDEX IF NOT EXISTS idx_jobs_target ON schedulers(target_type, target_id);
            CREATE INDEX IF NOT EXISTS idx_jobs_next_run ON schedulers(state, next_run_at);

            CREATE TABLE IF NOT EXISTS scheduler_runs (
                id TEXT PRIMARY KEY,
                scheduler_id TEXT NOT NULL,
                attempt INTEGER NOT NULL,
                state TEXT NOT NULL CHECK (state IN ('pending','running','success','failed','timeout')),
                scheduled_at TEXT NOT NULL,
                started_at TEXT,
                finished_at TEXT,
                duration_ms INTEGER,
                exit_code INTEGER,
                agent_response_json TEXT,
                error_code TEXT,
                error_message TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY(scheduler_id) REFERENCES schedulers(id)
            );

            CREATE INDEX IF NOT EXISTS idx_job_runs_job
            ON scheduler_runs(scheduler_id, created_at);

            CREATE INDEX IF NOT EXISTS idx_job_runs_state
            ON scheduler_runs(state);

            CREATE UNIQUE INDEX IF NOT EXISTS uq_job_runs_single_running
            ON scheduler_runs(scheduler_id)
            WHERE state = 'running';
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))
}

fn normalize_job_targets_to_work(conn: &Connection) -> Result<(), OrbitError> {
    if !table_exists(conn, "schedulers")? || !table_has_column(conn, "schedulers", "target_type")? {
        return Ok(());
    }

    let sql: String = conn
        .query_row(
            "SELECT COALESCE(sql, '') FROM sqlite_master WHERE type='table' AND name='schedulers'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;

    let non_work_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM schedulers WHERE target_type NOT IN ('job')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;

    let supports_work = sql.to_lowercase().contains("'job'");
    if supports_work && non_work_count == 0 {
        return Ok(());
    }

    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    let rewrite_result = conn.execute_batch(
        r#"
            CREATE TABLE jobs_rewrite (
                id TEXT PRIMARY KEY,
                target_type TEXT NOT NULL CHECK (target_type IN ('job')),
                target_id TEXT NOT NULL,
                schedule TEXT NOT NULL,
                agent_cli TEXT NOT NULL,
                timeout_seconds INTEGER NOT NULL,
                retry_max_attempts INTEGER NOT NULL DEFAULT 0,
                retry_backoff_strategy TEXT NOT NULL DEFAULT 'none',
                retry_initial_delay_seconds INTEGER NOT NULL DEFAULT 0,
                state TEXT NOT NULL CHECK (state IN ('enabled','paused','disabled')),
                next_run_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            INSERT INTO jobs_rewrite(
                id, target_type, target_id, schedule, agent_cli, timeout_seconds,
                retry_max_attempts, retry_backoff_strategy, retry_initial_delay_seconds,
                state, next_run_at, created_at, updated_at
            )
            SELECT
                id,
                'job',
                target_id,
                schedule,
                agent_cli,
                timeout_seconds,
                retry_max_attempts,
                retry_backoff_strategy,
                retry_initial_delay_seconds,
                state,
                next_run_at,
                created_at,
                updated_at
            FROM schedulers;

            DROP TABLE schedulers;
            ALTER TABLE jobs_rewrite RENAME TO schedulers;
        "#,
    );
    let fk_enable_result = conn.execute_batch("PRAGMA foreign_keys=ON;");

    rewrite_result.map_err(|e| OrbitError::Store(e.to_string()))?;
    fk_enable_result.map_err(|e| OrbitError::Store(e.to_string()))?;

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

fn ensure_execution_targets_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            DROP TABLE IF EXISTS workflows;

            CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                type TEXT NOT NULL,
                description TEXT NOT NULL,
                input_schema_json TEXT NOT NULL,
                output_schema_json TEXT NOT NULL,
                artifact_path_template TEXT,
                skill_refs_json TEXT,
                identity_id TEXT,
                assigned_to TEXT,
                created_by TEXT,
                is_active INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_works_type
            ON jobs(type);

            CREATE INDEX IF NOT EXISTS idx_works_active
            ON jobs(is_active);
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;
    add_column_if_missing(conn, "ALTER TABLE jobs ADD COLUMN identity_id TEXT")?;
    add_column_if_missing(conn, "ALTER TABLE jobs ADD COLUMN assigned_to TEXT")?;
    add_column_if_missing(conn, "ALTER TABLE jobs ADD COLUMN created_by TEXT")?;
    Ok(())
}

fn repair_legacy_scheduler_rows_in_jobs_table(conn: &Connection) -> Result<(), OrbitError> {
    if !table_exists(conn, "jobs")? {
        return Ok(());
    }

    let jobs_has_target_type = table_has_column(conn, "jobs", "target_type")?;
    let jobs_has_type = table_has_column(conn, "jobs", "type")?;
    if !jobs_has_target_type || jobs_has_type {
        return Ok(());
    }

    conn.execute_batch(
        r#"
            INSERT OR IGNORE INTO schedulers(
                id, target_type, target_id, schedule, agent_cli, timeout_seconds,
                retry_max_attempts, retry_backoff_strategy, retry_initial_delay_seconds,
                state, next_run_at, created_at, updated_at
            )
            SELECT
                id,
                'job',
                target_id,
                schedule,
                agent_cli,
                timeout_seconds,
                COALESCE(retry_max_attempts, 0),
                COALESCE(retry_backoff_strategy, 'none'),
                COALESCE(retry_initial_delay_seconds, 0),
                state,
                next_run_at,
                created_at,
                updated_at
            FROM jobs;

            DROP TABLE jobs;
        "#,
    )
    .map_err(|e| OrbitError::Store(e.to_string()))
}

fn migrate_legacy_work_rows(conn: &Connection) -> Result<(), OrbitError> {
    if !table_exists(conn, "jobs")? {
        return Ok(());
    }

    let works_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM jobs", [], |row| row.get(0))
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    if works_count > 0 {
        return Ok(());
    }

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name != 'jobs'")
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| OrbitError::Store(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| OrbitError::Store(e.to_string()))?;

    let required_cols = [
        "id",
        "type",
        "description",
        "input_schema_json",
        "output_schema_json",
        "artifact_path_template",
        "skill_refs_json",
        "is_active",
        "created_at",
        "updated_at",
    ];

    for table in names {
        if !is_safe_identifier(&table) {
            continue;
        }

        let mut all_present = true;
        for col in required_cols {
            if !table_has_column(conn, &table, col)? {
                all_present = false;
                break;
            }
        }
        if !all_present {
            continue;
        }

        let sql = format!(
            "INSERT OR IGNORE INTO jobs(
                id, type, description, input_schema_json, output_schema_json,
                artifact_path_template, skill_refs_json, identity_id, assigned_to, created_by, is_active, created_at, updated_at
            )
            SELECT
                id, type, description, input_schema_json, output_schema_json,
                artifact_path_template, skill_refs_json, NULL, NULL, NULL, is_active, created_at, updated_at
            FROM {table}"
        );
        conn.execute_batch(&sql)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        break;
    }

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

fn is_safe_identifier(value: &str) -> bool {
    value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::{apply_schema, table_has_foreign_key_to};
    use rusqlite::Connection;

    #[test]
    fn apply_schema_migrates_legacy_jobs_before_index_creation() {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            r#"
                CREATE TABLE schedulers (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    command TEXT NOT NULL,
                    next_run_at TEXT,
                    last_run_at TEXT,
                    last_status TEXT
                );
            "#,
        )
        .expect("legacy schedulers");

        apply_schema(&conn).expect("apply schema");

        let mut stmt = conn
            .prepare("PRAGMA table_info(schedulers)")
            .expect("table info");
        let mut rows = stmt.query([]).expect("query");
        let mut saw_target_type = false;
        while let Some(row) = rows.next().expect("row") {
            let name: String = row.get(1).expect("name");
            if name == "target_type" {
                saw_target_type = true;
            }
        }
        assert!(saw_target_type);

        let run_table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='scheduler_runs'",
                [],
                |row| row.get(0),
            )
            .expect("query scheduler_runs");
        assert_eq!(run_table_exists, 1);
    }

    #[test]
    fn apply_schema_migrates_v1_jobs_to_v2() {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            r#"
                CREATE TABLE schedulers (
                    scheduler_id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    task_id TEXT NOT NULL,
                    schedule_spec TEXT NOT NULL,
                    timezone TEXT NOT NULL,
                    state TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    paused_at TEXT,
                    deleted_at TEXT,
                    last_run_session_id TEXT,
                    last_run_at TEXT,
                    next_run_at TEXT,
                    last_error TEXT
                );

                INSERT INTO schedulers(
                    scheduler_id, name, task_id, schedule_spec, timezone, state,
                    created_at, updated_at, next_run_at
                ) VALUES (
                    'scheduler-1', 'demo', 'task-1', '0 * * * *', 'UTC', 'active',
                    '2026-02-23T00:00:00Z', '2026-02-23T00:00:00Z', '2026-02-23T01:00:00Z'
                );
            "#,
        )
        .expect("v1 schedulers");

        apply_schema(&conn).expect("apply schema");

        let (target_type, state): (String, String) = conn
            .query_row(
                "SELECT target_type, state FROM schedulers WHERE id = 'scheduler-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("query migrated scheduler");
        assert_eq!(target_type, "job");
        assert_eq!(state, "enabled");
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
    fn apply_schema_normalizes_non_work_job_targets() {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            r#"
                CREATE TABLE schedulers (
                    id TEXT PRIMARY KEY,
                    target_type TEXT NOT NULL CHECK (target_type IN ('legacy_target','job')),
                    target_id TEXT NOT NULL,
                    schedule TEXT NOT NULL,
                    agent_cli TEXT NOT NULL,
                    timeout_seconds INTEGER NOT NULL,
                    retry_max_attempts INTEGER NOT NULL DEFAULT 0,
                    retry_backoff_strategy TEXT NOT NULL DEFAULT 'none',
                    retry_initial_delay_seconds INTEGER NOT NULL DEFAULT 0,
                    state TEXT NOT NULL CHECK (state IN ('enabled','paused','disabled')),
                    next_run_at TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                INSERT INTO schedulers(
                    id, target_type, target_id, schedule, agent_cli, timeout_seconds,
                    retry_max_attempts, retry_backoff_strategy, retry_initial_delay_seconds,
                    state, next_run_at, created_at, updated_at
                ) VALUES (
                    'scheduler-legacy', 'legacy_target', 'w-1', '@daily', 'claude', 300,
                    0, 'none', 0, 'enabled', '2026-02-23T01:00:00Z',
                    '2026-02-23T00:00:00Z', '2026-02-23T00:00:00Z'
                );
            "#,
        )
        .expect("legacy schedulers");

        apply_schema(&conn).expect("apply schema");

        let target: String = conn
            .query_row(
                "SELECT target_type FROM schedulers WHERE id = 'scheduler-legacy'",
                [],
                |row| row.get(0),
            )
            .expect("query target");
        assert_eq!(target, "job");
    }

    #[test]
    fn apply_schema_migrates_legacy_work_like_table_into_works() {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            r#"
                CREATE TABLE job_legacy (
                    id TEXT PRIMARY KEY,
                    type TEXT NOT NULL,
                    description TEXT NOT NULL,
                    input_schema_json TEXT NOT NULL,
                    output_schema_json TEXT NOT NULL,
                    artifact_path_template TEXT,
                    skill_refs_json TEXT,
                    is_active INTEGER NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                INSERT INTO job_legacy(
                    id, type, description, input_schema_json, output_schema_json,
                    artifact_path_template, skill_refs_json, is_active, created_at, updated_at
                ) VALUES (
                    'job-1', 'analysis', 'legacy job',
                    '{}', '{}', NULL, '[]', 1, '2026-02-23T00:00:00Z', '2026-02-23T00:00:00Z'
                );
            "#,
        )
        .expect("legacy jobs");

        apply_schema(&conn).expect("apply schema");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM jobs WHERE id = 'job-1'", [], |row| {
                row.get(0)
            })
            .expect("query jobs");
        assert_eq!(count, 1);
    }

    #[test]
    fn apply_schema_repairs_scheduler_shaped_jobs_table() {
        let conn = Connection::open_in_memory().expect("open");
        conn.execute_batch(
            r#"
                CREATE TABLE jobs (
                    id TEXT PRIMARY KEY,
                    target_type TEXT NOT NULL CHECK (target_type IN ('work')),
                    target_id TEXT NOT NULL,
                    schedule TEXT NOT NULL,
                    agent_cli TEXT NOT NULL,
                    timeout_seconds INTEGER NOT NULL,
                    retry_max_attempts INTEGER NOT NULL DEFAULT 0,
                    retry_backoff_strategy TEXT NOT NULL DEFAULT 'none',
                    retry_initial_delay_seconds INTEGER NOT NULL DEFAULT 0,
                    state TEXT NOT NULL CHECK (state IN ('enabled','paused','disabled')),
                    next_run_at TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                INSERT INTO jobs(
                    id, target_type, target_id, schedule, agent_cli, timeout_seconds,
                    retry_max_attempts, retry_backoff_strategy, retry_initial_delay_seconds,
                    state, next_run_at, created_at, updated_at
                ) VALUES (
                    'scheduler-legacy', 'work', 'spec-1', 'every 1m', 'mock-agent', 300,
                    0, 'none', 0, 'enabled', '2026-02-23T01:00:00Z',
                    '2026-02-23T00:00:00Z', '2026-02-23T00:00:00Z'
                );
            "#,
        )
        .expect("legacy scheduler-shaped jobs table");

        apply_schema(&conn).expect("apply schema");

        let jobs_has_type: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('jobs') WHERE name = 'type'",
                [],
                |row| row.get(0),
            )
            .expect("jobs schema has type");
        assert_eq!(jobs_has_type, 1);

        let migrated_target_type: String = conn
            .query_row(
                "SELECT target_type FROM schedulers WHERE id = 'scheduler-legacy'",
                [],
                |row| row.get(0),
            )
            .expect("migrated scheduler exists");
        assert_eq!(migrated_target_type, "job");
    }
}
