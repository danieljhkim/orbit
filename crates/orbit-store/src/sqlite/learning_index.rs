//! Project-learnings envelope index.
//!
//! Schema is defined alongside the rest of `apply_schema` in
//! [`super::migration::ensure_learning_index_schema`]; this module owns only
//! the query/mutation surface invoked by `file::learning_store::api`.
//!
//! Phase 1 uses the index for scope-match fast-paths: list active rows once,
//! evaluate path globs / tag matches in memory, then read the matched bundles
//! from disk. The filesystem remains the source of truth — `reindex` rebuilds
//! the table from YAML if it drifts.

use orbit_common::types::{Learning, LearningStatus, OrbitError};
use rusqlite::params;

use crate::Store;

/// Indexed row of the `learnings_index` table. Mirrors the schema columns;
/// `paths` and `tags` are decoded from their JSON-array on-disk form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LearningIndexRow {
    pub(crate) id: String,
    pub(crate) status: LearningStatus,
    pub(crate) paths: Vec<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) summary: String,
    pub(crate) updated_at: String,
    pub(crate) priority: Option<u8>,
}

impl Store {
    pub(crate) fn upsert_learning_index_row(
        &self,
        workspace_id: &str,
        learning: &Learning,
    ) -> Result<(), OrbitError> {
        let paths_json = serde_json::to_string(&learning.scope.paths)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let tags_json = serde_json::to_string(&learning.scope.tags)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let status = learning.status.as_str().to_string();
        let updated_at = learning.updated_at.to_rfc3339();

        let priority = learning.priority.map(|value| value as i64);
        self.with_transaction(|tx| {
            tx.tx
                .execute(
                    "INSERT INTO learnings_index (workspace_id, id, status, paths, tags, summary, updated_at, priority)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(workspace_id, id) DO UPDATE SET
                        status = excluded.status,
                        paths = excluded.paths,
                        tags = excluded.tags,
                        summary = excluded.summary,
                        updated_at = excluded.updated_at,
                        priority = excluded.priority",
                    params![
                        workspace_id,
                        learning.id,
                        status,
                        paths_json,
                        tags_json,
                        learning.summary,
                        updated_at,
                        priority,
                    ],
                )
                .map_err(|e| OrbitError::Store(e.to_string()))?;
            Ok(())
        })
    }

    pub(crate) fn delete_learning_index_row(
        &self,
        workspace_id: &str,
        id: &str,
    ) -> Result<(), OrbitError> {
        self.with_transaction(|tx| {
            tx.tx
                .execute(
                    "DELETE FROM learnings_index WHERE workspace_id = ?1 AND id = ?2",
                    params![workspace_id, id],
                )
                .map_err(|e| OrbitError::Store(e.to_string()))?;
            Ok(())
        })
    }

    pub(crate) fn truncate_learning_index(&self, workspace_id: &str) -> Result<(), OrbitError> {
        self.with_transaction(|tx| {
            tx.tx
                .execute(
                    "DELETE FROM learnings_index WHERE workspace_id = ?1",
                    params![workspace_id],
                )
                .map_err(|e| OrbitError::Store(e.to_string()))?;
            Ok(())
        })
    }

    pub(crate) fn list_active_learning_rows(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<LearningIndexRow>, OrbitError> {
        let conn = self.read()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, status, paths, tags, summary, updated_at, priority
                 FROM learnings_index
                 WHERE workspace_id = ?1 AND status = 'active'
                 ORDER BY updated_at DESC, id ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map(params![workspace_id], decode_row)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| OrbitError::Store(e.to_string()))?);
        }
        Ok(out)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn get_learning_index_row(
        &self,
        workspace_id: &str,
        id: &str,
    ) -> Result<Option<LearningIndexRow>, OrbitError> {
        let conn = self.read()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, status, paths, tags, summary, updated_at, priority
                 FROM learnings_index
                 WHERE workspace_id = ?1 AND id = ?2",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let mut rows = stmt
            .query(params![workspace_id, id])
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let Some(row) = rows.next().map_err(|e| OrbitError::Store(e.to_string()))? else {
            return Ok(None);
        };
        let parsed = decode_row(row).map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(Some(parsed))
    }
}

fn decode_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LearningIndexRow> {
    let id: String = row.get(0)?;
    let status_raw: String = row.get(1)?;
    let paths_json: String = row.get(2)?;
    let tags_json: String = row.get(3)?;
    let summary: String = row.get(4)?;
    let updated_at: String = row.get(5)?;
    let priority_raw: Option<i64> = row.get(6)?;

    let status: LearningStatus = status_raw.parse().map_err(|e: String| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::from(e))
    })?;
    let paths: Vec<String> = serde_json::from_str(&paths_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let tags: Vec<String> = serde_json::from_str(&tags_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let priority = priority_raw
        .map(|value| {
            u8::try_from(value).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    6,
                    rusqlite::types::Type::Integer,
                    Box::new(e),
                )
            })
        })
        .transpose()?;

    Ok(LearningIndexRow {
        id,
        status,
        paths,
        tags,
        summary,
        updated_at,
        priority,
    })
}
