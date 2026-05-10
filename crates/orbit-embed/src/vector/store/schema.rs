//! `embeddings` + `tasks_fts` schema bootstrap.
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

            CREATE VIRTUAL TABLE IF NOT EXISTS tasks_fts USING fts5(
                source_id UNINDEXED,
                field UNINDEXED,
                content,
                tokenize = 'porter unicode61 remove_diacritics 2'
            );
        "#,
    )
    .map_err(|error| OrbitError::Store(error.to_string()))
}
