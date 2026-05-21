//! Read/cascade operations over the index.
//!
//! `delete_source` cascades both the vector rows and the FTS5 rows for a
//! given `(source_kind, source_id)`. `stats` aggregates row counts by
//! `(source_kind, model_id)` and counts orphaned `task` rows whose
//! `source_id` is no longer in the live task corpus.

use std::collections::BTreeSet;

use orbit_common::types::OrbitError;
use rusqlite::params;

use super::VectorStore;
use crate::vector::{SemanticStats, SourceModelCount};

impl VectorStore {
    pub fn source_ids(&self, source_kind: &str) -> Result<BTreeSet<String>, OrbitError> {
        let conn = self.connection();
        let conn = conn
            .lock()
            .map_err(|error| OrbitError::Store(format!("mutex poisoned: {error}")))?;
        let mut stmt = conn
            .prepare("SELECT DISTINCT source_id FROM embeddings WHERE source_kind = ?1")
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let rows = stmt
            .query_map(params![source_kind], |row| row.get::<_, String>(0))
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let mut source_ids = BTreeSet::new();
        for row in rows {
            source_ids.insert(row.map_err(|error| OrbitError::Store(error.to_string()))?);
        }
        Ok(source_ids)
    }

    pub fn delete_source(&self, source_kind: &str, source_id: &str) -> Result<(), OrbitError> {
        let conn = self.connection();
        let conn = conn
            .lock()
            .map_err(|error| OrbitError::Store(format!("mutex poisoned: {error}")))?;
        conn.execute(
            "DELETE FROM embeddings WHERE source_kind = ?1 AND source_id = ?2",
            params![source_kind, source_id],
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
        conn.execute(
            "DELETE FROM corpus_fts WHERE source_kind = ?1 AND source_id = ?2",
            params![source_kind, source_id],
        )
        .map_err(|error| OrbitError::Store(error.to_string()))?;
        Ok(())
    }

    pub fn stats(&self, current_task_ids: &[String]) -> Result<SemanticStats, OrbitError> {
        let conn = self.connection();
        let conn = conn
            .lock()
            .map_err(|error| OrbitError::Store(format!("mutex poisoned: {error}")))?;
        let mut stmt = conn
            .prepare(
                r#"
                    SELECT source_kind, model_id, COUNT(*)
                    FROM embeddings
                    GROUP BY source_kind, model_id
                    ORDER BY source_kind, model_id
                "#,
            )
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SourceModelCount {
                    source_kind: row.get(0)?,
                    model_id: row.get(1)?,
                    rows: row.get::<_, i64>(2)? as usize,
                })
            })
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let mut counts = Vec::new();
        for row in rows {
            counts.push(row.map_err(|error| OrbitError::Store(error.to_string()))?);
        }

        let current = current_task_ids.iter().cloned().collect::<BTreeSet<_>>();
        let mut stmt = conn
            .prepare("SELECT DISTINCT source_id FROM embeddings WHERE source_kind = 'task'")
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| OrbitError::Store(error.to_string()))?;
        let mut stale_rows = 0;
        for row in rows {
            let source_id = row.map_err(|error| OrbitError::Store(error.to_string()))?;
            if !current.contains(&source_id) {
                stale_rows += 1;
            }
        }
        Ok(SemanticStats { counts, stale_rows })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoopEmbedder;
    use crate::vector::EmbeddingField;

    #[test]
    fn delete_source_cascades_vector_and_fts_rows() {
        let store = VectorStore::open_in_memory().unwrap();
        let embedder = NoopEmbedder::small();
        store
            .upsert_embeddings(
                "task",
                "T1",
                &[EmbeddingField::new("purpose", "delete me")],
                &embedder,
                false,
            )
            .unwrap();

        store.delete_source("task", "T1").unwrap();

        let conn = store.connection();
        let conn = conn.lock().unwrap();
        let embeddings: i64 = conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0))
            .unwrap();
        let fts: i64 = conn
            .query_row("SELECT COUNT(*) FROM corpus_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(embeddings, 0);
        assert_eq!(fts, 0);
    }
}
