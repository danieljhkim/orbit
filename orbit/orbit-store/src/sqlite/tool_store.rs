use orbit_types::{OrbitError, StoredTool};
use rusqlite::{OptionalExtension, params};

use crate::{Store, StoreTx, now_string};

impl Store {
    pub fn list_tools(&self) -> Result<Vec<StoredTool>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare("SELECT name, path, description, enabled, builtin FROM tools ORDER BY name")
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(StoredTool {
                    name: row.get(0)?,
                    path: row.get(1)?,
                    description: row.get(2)?,
                    enabled: row.get::<_, i32>(3)? != 0,
                    builtin: row.get::<_, i32>(4)? != 0,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_tool(&self, name: &str) -> Result<Option<StoredTool>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let mut stmt = conn
            .prepare("SELECT name, path, description, enabled, builtin FROM tools WHERE name = ?1")
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let result = stmt
            .query_row(params![name], |row| {
                Ok(StoredTool {
                    name: row.get(0)?,
                    path: row.get(1)?,
                    description: row.get(2)?,
                    enabled: row.get::<_, i32>(3)? != 0,
                    builtin: row.get::<_, i32>(4)? != 0,
                })
            })
            .optional()
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(result)
    }
}

impl<'a> StoreTx<'a> {
    pub fn insert_tool(&mut self, tool: &StoredTool) -> Result<(), OrbitError> {
        self.tx
            .execute(
                "INSERT INTO tools(name, path, description, enabled, builtin, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    tool.name,
                    tool.path,
                    tool.description,
                    tool.enabled as i32,
                    tool.builtin as i32,
                    now_string(),
                    now_string(),
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(())
    }

    pub fn delete_tool(&mut self, name: &str) -> Result<bool, OrbitError> {
        let affected = self
            .tx
            .execute("DELETE FROM tools WHERE name = ?1", params![name])
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(affected > 0)
    }

    pub fn set_tool_enabled(&mut self, name: &str, enabled: bool) -> Result<bool, OrbitError> {
        let affected = self
            .tx
            .execute(
                "UPDATE tools SET enabled = ?1, updated_at = ?2 WHERE name = ?3",
                params![enabled as i32, now_string(), name],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(affected > 0)
    }
}
