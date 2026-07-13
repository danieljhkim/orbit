//! Versioned schema-migration ledger for the SQLite store (ORB-10003).
//!
//! Migrations are a stable, ordered registry of `(version, name, apply)`
//! entries. Each applied migration is recorded in the `schema_meta`
//! key/value table under `migration.v<NNNN>` (value = migration name,
//! `updated_at` = applied-at timestamp), so the ledger doubles as the
//! data source for a future `orbit migrate` command (P3.4).
//!
//! Guarantees:
//! - Each migration runs inside a single transaction together with its
//!   ledger insert, so an interrupted migration rolls back instead of
//!   leaving half-applied schema (SQLite ALTER/rename-copy-drop batches
//!   are wrappable in a transaction).
//! - A database whose recorded version is newer than
//!   [`SUPPORTED_SCHEMA_VERSION`] is refused with
//!   [`OrbitError::Migration`] (downgrade guard).
//! - Legacy databases created by the pre-ledger idempotent migrations
//!   adopt the ledger transparently: the v1 baseline is the same
//!   idempotent schema code, so running it on an existing database is a
//!   no-op that then records version 1.

use orbit_common::types::OrbitError;
use rusqlite::{Connection, params};

/// One entry in the migration registry.
pub(crate) struct Migration {
    pub(crate) version: u32,
    pub(crate) name: &'static str,
    pub(crate) apply: fn(&Connection) -> Result<(), OrbitError>,
}

/// Stable ordered registry of store-database migrations. Append-only:
/// never renumber or edit an entry that has shipped.
pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "baseline",
        apply: super::apply_baseline_schema,
    },
    Migration {
        version: 2,
        name: "learnings_index_workspace_scope",
        apply: super::apply_learning_index_workspace_scope,
    },
    Migration {
        version: 3,
        name: "flat_crew_model",
        apply: super::apply_flat_crew_model,
    },
    Migration {
        version: 4,
        name: "job_run_archive_stage",
        apply: super::apply_job_run_archive_stage,
    },
];

/// Highest schema version this binary knows how to produce. Public for
/// the future `orbit migrate` surface (P3.4), alongside
/// [`AppliedMigration`] and the `Store` version accessors.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 4;

const LEDGER_KEY_PREFIX: &str = "migration.v";

/// A migration recorded as applied in the `schema_meta` ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMigration {
    pub version: u32,
    pub name: String,
    pub applied_at: String,
}

/// Bring `conn` up to the newest version in `migrations`, recording each
/// applied migration in the ledger. Refuses databases newer than the
/// registry supports.
pub(crate) fn run_migrations(
    conn: &Connection,
    migrations: &[Migration],
) -> Result<(), OrbitError> {
    validate_registry(migrations)?;
    ensure_schema_meta_table(conn)?;

    let current = current_schema_version(conn)?;
    let supported = migrations.last().map(|m| m.version).unwrap_or(0);
    if current > supported {
        return Err(OrbitError::Migration(format!(
            "store database schema version {current} is newer than the newest version this \
             orbit binary supports ({supported}); upgrade orbit to open this database"
        )));
    }

    for migration in migrations.iter().filter(|m| m.version > current) {
        apply_one(conn, migration)?;
    }

    Ok(())
}

/// Current schema version recorded in the ledger; 0 when no versioned
/// migration has been applied (fresh or pre-ledger legacy database).
pub(crate) fn current_schema_version(conn: &Connection) -> Result<u32, OrbitError> {
    Ok(applied_migrations(conn)?
        .last()
        .map(|m| m.version)
        .unwrap_or(0))
}

/// All migrations recorded as applied, ordered by version ascending.
pub(crate) fn applied_migrations(conn: &Connection) -> Result<Vec<AppliedMigration>, OrbitError> {
    if !super::table_exists(conn, "schema_meta")? {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT key, value, updated_at FROM schema_meta
             WHERE key LIKE ?1 ORDER BY key",
        )
        .map_err(|e| OrbitError::Store(e.to_string()))?;
    let rows = stmt
        .query_map([format!("{LEDGER_KEY_PREFIX}%")], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| OrbitError::Store(e.to_string()))?;

    let mut applied = Vec::new();
    for row in rows {
        let (key, name, applied_at) = row.map_err(|e| OrbitError::Store(e.to_string()))?;
        let version = parse_ledger_key(&key)?;
        applied.push(AppliedMigration {
            version,
            name,
            applied_at,
        });
    }
    applied.sort_by_key(|m| m.version);
    Ok(applied)
}

fn apply_one(conn: &Connection, migration: &Migration) -> Result<(), OrbitError> {
    // `unchecked_transaction` rolls back on drop, so a failure inside the
    // migration (or a process crash) leaves neither partial schema nor a
    // ledger row behind.
    let tx = conn.unchecked_transaction().map_err(|e| {
        OrbitError::Migration(format!(
            "failed to begin transaction for migration v{} ({}): {e}",
            migration.version, migration.name
        ))
    })?;

    (migration.apply)(&tx)?;

    tx.execute(
        "INSERT INTO schema_meta(key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        params![
            ledger_key(migration.version),
            migration.name,
            crate::now_string()
        ],
    )
    .map_err(|e| {
        OrbitError::Migration(format!(
            "failed to record migration v{} ({}) in schema_meta: {e}",
            migration.version, migration.name
        ))
    })?;

    tx.commit().map_err(|e| {
        OrbitError::Migration(format!(
            "failed to commit migration v{} ({}): {e}",
            migration.version, migration.name
        ))
    })?;

    orbit_common::tracing::info!(
        target: "orbit.store.sqlite",
        version = migration.version,
        name = migration.name,
        "applied store schema migration",
    );
    Ok(())
}

fn ensure_schema_meta_table(conn: &Connection) -> Result<(), OrbitError> {
    // Bootstrap table for the ledger itself; must match the shape the
    // baseline schema declares (also used for state-import markers).
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )
    .map_err(|e| OrbitError::Store(e.to_string()))
}

fn validate_registry(migrations: &[Migration]) -> Result<(), OrbitError> {
    let mut previous = 0u32;
    for migration in migrations {
        if migration.version <= previous {
            return Err(OrbitError::Migration(format!(
                "migration registry is not strictly increasing: v{} ({}) follows v{previous}",
                migration.version, migration.name
            )));
        }
        previous = migration.version;
    }
    Ok(())
}

fn ledger_key(version: u32) -> String {
    format!("{LEDGER_KEY_PREFIX}{version:04}")
}

fn parse_ledger_key(key: &str) -> Result<u32, OrbitError> {
    key.strip_prefix(LEDGER_KEY_PREFIX)
        .and_then(|suffix| suffix.parse::<u32>().ok())
        .ok_or_else(|| {
            OrbitError::Migration(format!(
                "corrupt schema_meta migration ledger entry: {key:?}"
            ))
        })
}
