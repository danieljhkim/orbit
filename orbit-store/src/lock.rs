use orbit_types::OrbitError;
use rusqlite::params;

use crate::{Store, now_string};

const GLOBAL_JOB_LOCK: &str = "job/run";

impl Store {
    pub fn try_lock(&self, name: &str) -> Result<bool, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let changed = conn
            .execute(
                "INSERT INTO locks(name, owner, acquired_at) VALUES (?1, ?2, ?3) ON CONFLICT(name) DO NOTHING",
                params![name, "orbit-core", now_string()],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(changed == 1)
    }

    pub fn unlock(&self, name: &str) -> Result<bool, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let changed = conn
            .execute("DELETE FROM locks WHERE name = ?1", [name])
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(changed == 1)
    }

    pub fn global_job_lock_name() -> &'static str {
        GLOBAL_JOB_LOCK
    }
}
