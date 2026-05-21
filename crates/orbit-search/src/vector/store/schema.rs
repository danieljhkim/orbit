//! `embeddings` + `corpus_fts` schema bootstrap.
//!
//! Idempotent — runs on every `VectorStore::open`. Kept separate from the
//! store module so future schema changes (e.g. swapping `embedding BLOB` for
//! a `sqlite-vec` virtual table per ADR-002) live in one place.

use orbit_common::types::OrbitError;
use rusqlite::Connection;

pub fn ensure_vector_schema(conn: &Connection) -> Result<(), OrbitError> {
    conn.execute_batch(
        r#"
            CREATE TABLE IF NOT EXISTS embeddings (
                source_kind TEXT NOT NULL,
                source_id TEXT NOT NULL,
                field TEXT NOT NULL,
                chunk_idx INTEGER NOT NULL,
                content_hash TEXT NOT NULL,
                model_id TEXT NOT NULL,
                dim INTEGER NOT NULL,
                embedding BLOB NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (source_kind, source_id, field, chunk_idx, model_id)
            );

            CREATE INDEX IF NOT EXISTS embeddings_by_source
            ON embeddings(source_kind, source_id);

            CREATE INDEX IF NOT EXISTS embeddings_by_model
            ON embeddings(model_id);

            CREATE VIRTUAL TABLE IF NOT EXISTS corpus_fts USING fts5(
                source_kind UNINDEXED,
                source_id UNINDEXED,
                field UNINDEXED,
                content,
                tokenize = 'porter unicode61 remove_diacritics 2'
            );
        "#,
    )
    .map_err(|error| OrbitError::Store(error.to_string()))?;
    migrate_legacy_task_fts(conn)
}

fn migrate_legacy_task_fts(conn: &Connection) -> Result<(), OrbitError> {
    let legacy_table = legacy_task_fts_table();
    if !table_exists(conn, legacy_table)? {
        return Ok(());
    }

    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    let result = migrate_legacy_task_fts_in_transaction(conn, legacy_table);
    match result {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .map_err(|error| OrbitError::Store(error.to_string())),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

fn migrate_legacy_task_fts_in_transaction(
    conn: &Connection,
    legacy_table: &str,
) -> Result<(), OrbitError> {
    let corpus_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM corpus_fts", [], |row| row.get(0))
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    if corpus_rows == 0 {
        let copy_sql = format!(
            r#"
                INSERT INTO corpus_fts(rowid, source_kind, source_id, field, content)
                SELECT rowid, 'task', source_id, field, content
                FROM {legacy_table}
            "#
        );
        conn.execute(&copy_sql, [])
            .map_err(|error| OrbitError::Store(error.to_string()))?;
    }
    let drop_sql = format!("DROP TABLE {legacy_table}");
    conn.execute(&drop_sql, [])
        .map_err(|error| OrbitError::Store(error.to_string()))?;
    Ok(())
}

// pub(crate) widened for tests/ layout under ORB-00230; test reaches via
// sibling `tests/schema.rs` (see docs/design-patterns/test_layout.md).
pub(crate) fn table_exists(conn: &Connection, table: &str) -> Result<bool, OrbitError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get::<_, i64>(0),
    )
    .map(|exists| exists != 0)
    .map_err(|error| OrbitError::Store(error.to_string()))
}

// pub(crate) widened for tests/ layout under ORB-00230; test reaches via
// sibling `tests/schema.rs` (see docs/design-patterns/test_layout.md).
pub(crate) fn legacy_task_fts_table() -> &'static str {
    concat!("tasks", "_fts")
}
