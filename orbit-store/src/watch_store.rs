use chrono::Utc;
use orbit_types::{OrbitError, Watch};
use rusqlite::{OptionalExtension, params};

use crate::{Store, StoreTx, new_id, parse_timestamp};

fn row_to_watch(row: &rusqlite::Row<'_>) -> rusqlite::Result<Watch> {
    let updated_at: String = row.get(4)?;
    let debounce_ms: i64 = row.get(3)?;
    Ok(Watch {
        id: row.get(0)?,
        path: row.get(1)?,
        command: row.get(2)?,
        debounce_ms: debounce_ms as u64,
        updated_at: parse_timestamp(&updated_at)?,
    })
}

impl Store {
    pub fn list_watches(&self) -> Result<Vec<Watch>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare(
                "SELECT id, path, command, debounce_ms, updated_at FROM watches ORDER BY updated_at DESC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map([], row_to_watch)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_watch(&self, id: &str) -> Result<Option<Watch>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        conn.query_row(
            "SELECT id, path, command, debounce_ms, updated_at FROM watches WHERE id = ?1",
            [id],
            row_to_watch,
        )
        .optional()
        .map_err(|e| OrbitError::Store(e.to_string()))
    }
}

impl<'a> StoreTx<'a> {
    pub fn insert_watch(
        &mut self,
        path: &str,
        command: &str,
        debounce_ms: u64,
    ) -> Result<Watch, OrbitError> {
        let watch = Watch {
            id: new_id("watch"),
            path: path.to_string(),
            command: command.to_string(),
            debounce_ms,
            updated_at: Utc::now(),
        };

        self.tx
            .execute(
                "INSERT INTO watches(id, path, command, debounce_ms, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    watch.id,
                    watch.path,
                    watch.command,
                    watch.debounce_ms as i64,
                    watch.updated_at.to_rfc3339()
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(watch)
    }
}
