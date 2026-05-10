use std::collections::BTreeSet;

use orbit_common::types::{OrbitError, normalize_task_tags};
use rusqlite::params;

use crate::{Store, StoreTx};

impl Store {
    pub(crate) fn replace_task_tags(
        &self,
        task_id: &str,
        tags: &[String],
    ) -> Result<(), OrbitError> {
        let tags = normalize_task_tags(tags.to_vec());
        self.with_transaction(|tx| tx.replace_task_tags(task_id, &tags))
    }

    pub(crate) fn delete_task_tags(&self, task_id: &str) -> Result<(), OrbitError> {
        self.with_transaction(|tx| tx.delete_task_tags(task_id))
    }

    pub fn list_task_tags(&self, task_id: &str) -> Result<Vec<String>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare("SELECT tag FROM task_tags WHERE task_id = ?1 ORDER BY rowid")
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map(params![task_id], |row| row.get::<_, String>(0))
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let mut tags = Vec::new();
        for tag in rows {
            tags.push(tag.map_err(|e| OrbitError::Store(e.to_string()))?);
        }
        Ok(tags)
    }

    pub(crate) fn task_ids_with_all_tags(
        &self,
        tags: &[String],
    ) -> Result<Vec<String>, OrbitError> {
        let tags = normalize_task_tags(tags.to_vec());
        if tags.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut matched: Option<BTreeSet<String>> = None;
        for tag in tags {
            let mut stmt = conn
                .prepare("SELECT task_id FROM task_tags WHERE tag = ?1")
                .map_err(|e| OrbitError::Store(e.to_string()))?;
            let rows = stmt
                .query_map(params![tag], |row| row.get::<_, String>(0))
                .map_err(|e| OrbitError::Store(e.to_string()))?;
            let mut ids_for_tag = BTreeSet::new();
            for id in rows {
                ids_for_tag.insert(id.map_err(|e| OrbitError::Store(e.to_string()))?);
            }

            matched = Some(match matched {
                Some(previous) => previous.intersection(&ids_for_tag).cloned().collect(),
                None => ids_for_tag,
            });
        }

        Ok(matched.unwrap_or_default().into_iter().collect())
    }
}

impl StoreTx<'_> {
    fn replace_task_tags(&mut self, task_id: &str, tags: &[String]) -> Result<(), OrbitError> {
        self.delete_task_tags(task_id)?;
        for tag in tags {
            self.tx
                .execute(
                    "INSERT OR IGNORE INTO task_tags(task_id, tag) VALUES (?1, ?2)",
                    params![task_id, tag],
                )
                .map_err(|e| OrbitError::Store(e.to_string()))?;
        }
        Ok(())
    }

    fn delete_task_tags(&mut self, task_id: &str) -> Result<(), OrbitError> {
        self.tx
            .execute("DELETE FROM task_tags WHERE task_id = ?1", params![task_id])
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(())
    }
}
