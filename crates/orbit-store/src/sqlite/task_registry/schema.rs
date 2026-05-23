use orbit_common::types::OrbitError;
use rusqlite::Connection;

use super::REGISTRY_SCHEMA_VERSION;
use super::util::now_string;

pub(super) fn apply_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS allocator_state (
            authority TEXT PRIMARY KEY,
            next_number INTEGER NOT NULL CHECK(next_number >= 0 AND next_number <= 100000),
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS workspace_bindings (
            workspace_id TEXT PRIMARY KEY,
            slug TEXT NOT NULL,
            repo_root TEXT NOT NULL,
            workspace_path TEXT NOT NULL,
            orbit_dir TEXT NOT NULL UNIQUE,
            repo_fingerprint TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_workspace_bindings_paths
            ON workspace_bindings(repo_root, workspace_path, orbit_dir);

        CREATE TABLE IF NOT EXISTS task_bundle_bindings (
            task_id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            canonical_path TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY(workspace_id) REFERENCES workspace_bindings(workspace_id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_task_bundle_bindings_workspace
            ON task_bundle_bindings(workspace_id, task_id);

        CREATE TABLE IF NOT EXISTS task_bundle_index (
            task_id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL,
            status TEXT NOT NULL,
            priority TEXT NOT NULL,
            job_run_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            terminal_month TEXT,
            FOREIGN KEY(task_id) REFERENCES task_bundle_bindings(task_id) ON DELETE CASCADE,
            FOREIGN KEY(workspace_id) REFERENCES workspace_bindings(workspace_id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_task_bundle_index_workspace_created
            ON task_bundle_index(workspace_id, created_at DESC, task_id ASC);
        CREATE INDEX IF NOT EXISTS idx_task_bundle_index_workspace_status
            ON task_bundle_index(workspace_id, status, created_at DESC, task_id ASC);
        CREATE INDEX IF NOT EXISTS idx_task_bundle_index_workspace_priority
            ON task_bundle_index(workspace_id, priority, created_at DESC, task_id ASC);

        CREATE TABLE IF NOT EXISTS task_bundle_tags (
            task_id TEXT NOT NULL,
            workspace_id TEXT NOT NULL,
            tag TEXT NOT NULL,
            PRIMARY KEY(task_id, tag),
            FOREIGN KEY(task_id) REFERENCES task_bundle_bindings(task_id) ON DELETE CASCADE,
            FOREIGN KEY(workspace_id) REFERENCES workspace_bindings(workspace_id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_task_bundle_tags_workspace_tag
            ON task_bundle_tags(workspace_id, tag, task_id);

        CREATE TABLE IF NOT EXISTS task_bundle_relations (
            source_task_id TEXT NOT NULL,
            workspace_id TEXT NOT NULL,
            relation_type TEXT NOT NULL,
            target_task_id TEXT NOT NULL,
            PRIMARY KEY(source_task_id, relation_type, target_task_id),
            FOREIGN KEY(source_task_id) REFERENCES task_bundle_bindings(task_id) ON DELETE CASCADE,
            FOREIGN KEY(workspace_id) REFERENCES workspace_bindings(workspace_id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_task_bundle_relations_workspace_type_target
            ON task_bundle_relations(workspace_id, relation_type, target_task_id, source_task_id);
        ",
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    add_column_if_missing(
        conn,
        "task_bundle_index",
        "job_run_id",
        "ALTER TABLE task_bundle_index ADD COLUMN job_run_id TEXT",
    )?;
    add_column_if_missing(
        conn,
        "task_bundle_index",
        "terminal_month",
        "ALTER TABLE task_bundle_index ADD COLUMN terminal_month TEXT",
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_task_bundle_index_workspace_job_run
            ON task_bundle_index(workspace_id, job_run_id, created_at DESC, task_id ASC)",
        [],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_task_bundle_index_workspace_terminal
            ON task_bundle_index(workspace_id, terminal_month, task_id)",
        [],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;

    conn.execute(
        "INSERT OR IGNORE INTO allocator_state(authority, next_number, updated_at)
         VALUES ('local', 0, ?1)",
        [now_string()],
    )
    .map_err(|e| OrbitError::Store(e.to_string()))?;
    conn.pragma_update(None, "user_version", i64::from(REGISTRY_SCHEMA_VERSION))
        .map_err(|e| OrbitError::Store(format!("failed to set registry user_version: {e}")))?;
    Ok(())
}

pub(super) fn reject_unsupported_registry_schema(conn: &Connection) -> Result<(), OrbitError> {
    let version = registry_user_version(conn)?;
    if version > REGISTRY_SCHEMA_VERSION {
        return Err(OrbitError::Store(format!(
            "task registry schema version {version} is newer than supported version {REGISTRY_SCHEMA_VERSION}"
        )));
    }
    Ok(())
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> Result<(), OrbitError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| OrbitError::Store(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    if !columns.iter().any(|candidate| candidate == column) {
        conn.execute(alter_sql, [])
            .map_err(|e| OrbitError::Store(e.to_string()))?;
    }
    Ok(())
}

pub(super) fn registry_user_version(conn: &Connection) -> Result<u32, OrbitError> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| OrbitError::Store(format!("failed to read registry user_version: {e}")))?;
    u32::try_from(version)
        .map_err(|e| OrbitError::Store(format!("invalid registry user_version {version}: {e}")))
}

pub(super) fn assert_registry_user_version(conn: &Connection) -> Result<(), OrbitError> {
    let version = registry_user_version(conn)?;
    if version != REGISTRY_SCHEMA_VERSION {
        return Err(OrbitError::Store(format!(
            "task registry schema version {version} did not match expected version {REGISTRY_SCHEMA_VERSION}"
        )));
    }
    Ok(())
}
