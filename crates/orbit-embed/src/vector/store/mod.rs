//! `VectorStore` — the SQLite-backed semantic-search index.
//!
//! Module layout:
//!
//! - [`schema`] — `CREATE TABLE IF NOT EXISTS` DDL for `embeddings` + `tasks_fts`.
//! - [`upsert`] — `upsert_embeddings`, the BLAKE3-deduped per-field write path,
//!   plus its private SQL helpers (`delete_field_rows`, content-hash check).
//! - [`tasks`] — `index_task` / `reindex_tasks` task-corpus entry points.
//! - [`queries`] — `delete_source` and `stats` read/cascade operations.
//!
//! This file owns the `VectorStore` struct itself plus the connection-handle
//! plumbing (`open`, `open_in_memory`, `connection`, the WAL pragma helper)
//! and the small `pub(super)` constants shared across the submodules above.

mod queries;
pub mod schema;
mod tasks;
mod upsert;

pub use schema::ensure_vector_schema;

use std::path::Path;
use std::sync::{Arc, Mutex};

use orbit_common::types::OrbitError;
use rusqlite::Connection;

pub(super) const SOURCE_KIND_TASK: &str = "task";

#[derive(Clone)]
pub struct VectorStore {
    conn: Arc<Mutex<Connection>>,
}

impl VectorStore {
    /// Open the workspace-local semantic-search SQLite at `path`, applying WAL
    /// + busy_timeout pragmas and creating the embeddings/tasks_fts schema if missing.
    pub fn open(path: &Path) -> Result<Self, OrbitError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| OrbitError::Store(e.to_string()))?;
        }
        let conn = Connection::open(path).map_err(|e| OrbitError::Store(e.to_string()))?;
        enable_best_effort_wal_mode(&conn);
        conn.pragma_update(None, "busy_timeout", "5000")
            .map_err(|e| OrbitError::Store(format!("failed to set busy_timeout: {e}")))?;
        schema::ensure_vector_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory semantic-search database. Used by tests.
    pub fn open_in_memory() -> Result<Self, OrbitError> {
        let conn = Connection::open_in_memory().map_err(|e| OrbitError::Store(e.to_string()))?;
        schema::ensure_vector_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub(super) fn connection(&self) -> Arc<Mutex<Connection>> {
        self.conn.clone()
    }
}

fn enable_best_effort_wal_mode(conn: &Connection) {
    // WAL mode is best-effort: when the database file is read-only or the
    // filesystem refuses WAL sidecar writes, fall back to the default journal
    // mode so that read operations can still succeed.
    match conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0)) {
        Ok(mode) if mode.eq_ignore_ascii_case("wal") => {}
        Ok(mode) => {
            orbit_common::tracing::warn!(
                target: "orbit.embed.sqlite",
                journal_mode = mode.as_str(),
                "requested WAL mode on the semantic database, but SQLite kept the active journal mode",
            );
        }
        Err(err) => {
            orbit_common::tracing::warn!(
                target: "orbit.embed.sqlite",
                error = %err,
                "could not set WAL mode on the semantic database; continuing with the default journal mode",
            );
        }
    }
}
