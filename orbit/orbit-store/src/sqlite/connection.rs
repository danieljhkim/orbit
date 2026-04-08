use std::path::Path;
use std::sync::{Arc, Mutex};

use orbit_types::OrbitError;
use rusqlite::{Connection, Transaction};

use crate::sqlite::migration;

#[derive(Clone)]
pub struct Store {
    pub(crate) conn: Arc<Mutex<Connection>>,
}

pub struct StoreTx<'a> {
    pub(crate) tx: Transaction<'a>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, OrbitError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| OrbitError::Store(e.to_string()))?;
        }

        let conn = Connection::open(path).map_err(|e| OrbitError::Store(e.to_string()))?;
        enable_best_effort_wal_mode(&conn);
        conn.pragma_update(None, "busy_timeout", "5000")
            .map_err(|e| OrbitError::Store(format!("failed to set busy_timeout: {e}")))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| OrbitError::Store(format!("failed to enable foreign keys: {e}")))?;

        migration::apply_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_in_memory() -> Result<Self, OrbitError> {
        let conn = Connection::open_in_memory().map_err(|e| OrbitError::Store(e.to_string()))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| OrbitError::Store(format!("failed to enable foreign keys: {e}")))?;
        migration::apply_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn with_transaction<T, F>(&self, op: F) -> Result<T, OrbitError>
    where
        F: FnOnce(&mut StoreTx<'_>) -> Result<T, OrbitError>,
    {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        let tx = conn
            .transaction()
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let mut store_tx = StoreTx { tx };
        let result = op(&mut store_tx)?;
        store_tx
            .tx
            .commit()
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(result)
    }

    pub fn connection(&self) -> Arc<Mutex<Connection>> {
        self.conn.clone()
    }
}

fn enable_best_effort_wal_mode(conn: &Connection) {
    // WAL mode is best-effort: when the database file is read-only or the
    // filesystem refuses WAL sidecar writes, fall back to the default journal
    // mode so that read operations can still succeed.
    match set_journal_mode_wal(conn) {
        Ok(mode) if mode.eq_ignore_ascii_case("wal") => {}
        Ok(mode) => {
            eprintln!(
                "orbit: warning: requested WAL mode on the store database, but SQLite kept journal_mode={mode}; continuing with the active journal mode"
            );
        }
        Err(err) => {
            eprintln!(
                "orbit: warning: could not set WAL mode on the store database ({err}); continuing with the default journal mode"
            );
        }
    }
}

fn set_journal_mode_wal(conn: &Connection) -> Result<String, OrbitError> {
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))
        .map_err(|e| OrbitError::Store(format!("failed to set journal_mode=WAL: {e}")))
}
