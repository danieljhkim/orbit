use orbit_types::OrbitError;
use rusqlite::Connection;

pub fn apply_lock_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS file_locks (
            file_path TEXT NOT NULL,
            task_id TEXT NOT NULL,
            repo_root TEXT NOT NULL,
            acquired_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (repo_root, file_path)
        );

        CREATE INDEX IF NOT EXISTS idx_file_locks_task_id
            ON file_locks(task_id);
        "#,
    )
    .map_err(|error| OrbitError::Store(format!("failed to apply file lock schema: {error}")))?;

    Ok(())
}
