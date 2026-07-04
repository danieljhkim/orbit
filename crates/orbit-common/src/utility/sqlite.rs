//! Shared SQLite connection defaults for every Orbit SQLite store.
//!
//! Historically each store (orbit-store `Store`, its ID allocator and task
//! registry, and orbit-search's `VectorStore`) hand-rolled its own pragma
//! setup, and the copies drifted (missing `foreign_keys` here, missing
//! `busy_timeout` there). [`apply_default_pragmas`] is the single source of
//! truth: call it on every freshly opened connection, then layer any
//! store-specific overrides (e.g. the task registry's `synchronous=FULL`)
//! on top.
//!
//! orbit-graph cannot depend on this crate today (adding the edge needs an
//! ADR per `ARCHITECTURE.md`); its local `configure_connection` mirrors
//! these defaults and must be kept in sync manually.

use rusqlite::Connection;

use crate::types::OrbitError;

/// Default `busy_timeout` applied to every Orbit SQLite connection, in
/// milliseconds. Writers under WAL still serialize; this bounds how long a
/// contending connection spins before surfacing `database is locked`.
pub const DEFAULT_BUSY_TIMEOUT_MS: u32 = 5_000;

/// Result of [`apply_default_pragmas`]: what SQLite actually settled on for
/// the best-effort settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PragmaOutcome {
    /// Journal mode active after the WAL request, lowercased by SQLite
    /// convention (`wal`, `memory` for in-memory databases, or a fallback
    /// such as `delete` when the filesystem refuses WAL sidecars).
    pub journal_mode: String,
}

impl PragmaOutcome {
    /// True when the connection ended up in WAL mode. In-memory databases
    /// report `memory` and return false; callers that require WAL (e.g.
    /// orbit-graph semantics) can turn that into a hard error.
    pub fn wal_active(&self) -> bool {
        self.journal_mode.eq_ignore_ascii_case("wal")
    }
}

/// Apply the Orbit-wide SQLite connection defaults:
///
/// - `journal_mode=WAL` — best-effort: when the database file is read-only
///   or the filesystem refuses WAL sidecar writes, we warn and keep the
///   active journal mode so reads still succeed (in-memory databases keep
///   their `memory` mode silently — WAL does not apply to them);
/// - `busy_timeout` = [`DEFAULT_BUSY_TIMEOUT_MS`];
/// - `foreign_keys=ON`;
/// - `synchronous=NORMAL` — the recommended WAL durability level. Stores
///   that need commit-durable acks (e.g. the task registry) override to
///   `FULL` after calling this.
pub fn apply_default_pragmas(conn: &Connection) -> Result<PragmaOutcome, OrbitError> {
    let journal_mode = request_wal_journal_mode(conn);
    conn.pragma_update(None, "busy_timeout", DEFAULT_BUSY_TIMEOUT_MS)
        .map_err(|e| OrbitError::Store(format!("failed to set busy_timeout: {e}")))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| OrbitError::Store(format!("failed to enable foreign keys: {e}")))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| OrbitError::Store(format!("failed to set synchronous=NORMAL: {e}")))?;
    Ok(PragmaOutcome { journal_mode })
}

/// Request WAL and report the journal mode SQLite settled on. Never fails:
/// WAL is a performance/concurrency upgrade, not a correctness requirement,
/// so refusals degrade to a warning plus the active mode.
fn request_wal_journal_mode(conn: &Connection) -> String {
    match conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0)) {
        Ok(mode) => {
            if !mode.eq_ignore_ascii_case("wal") && !mode.eq_ignore_ascii_case("memory") {
                tracing::warn!(
                    target: "orbit.common.sqlite",
                    journal_mode = mode.as_str(),
                    "requested WAL mode, but SQLite kept the active journal mode",
                );
            }
            mode
        }
        Err(error) => {
            tracing::warn!(
                target: "orbit.common.sqlite",
                error = %error,
                "could not set WAL mode; continuing with the active journal mode",
            );
            conn.pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
                .unwrap_or_else(|_| "unknown".to_string())
        }
    }
}
