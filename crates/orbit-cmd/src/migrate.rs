//! Versioned `.orbit/` upgrade surface behind `orbit migrate` [ORB-10012].
//!
//! Both migration ledgers auto-apply when a workspace opens — the SQLite
//! schema ledger inside `Store::open` (ORB-10003) and the workspace-layout
//! registry in the [`OrbitRuntime::from_resolved_roots`] pre-flight —
//! so `orbit migrate` is the *explicit* surface over the same machinery:
//!
//! - `orbit migrate --confirm` (apply): opens the runtime normally, which applies
//!   everything pending, then reports the resulting versions and what the
//!   open applied.
//! - `orbit migrate` (the default) and `orbit migrate --dry-run`: inspect
//!   explicit resolved roots *without*
//!   opening the runtime (see [`migrate_dry_run_at`]) so pending migrations
//!   are listed instead of silently applied — the only way to see "pending"
//!   given auto-migration on open. Environment and workspace-catalog
//!   resolution belongs to the calling application.

use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use orbit_store::layout::{self, LayoutMigrationInfo, SUPPORTED_LAYOUT_VERSION};
use orbit_store::sqlite::migration::{
    PendingSchemaMigration, SUPPORTED_SCHEMA_VERSION, pending_schema_migrations_after,
    read_schema_ledger_status,
};

use orbit_core::OrbitRuntime;

/// Version alignment between a workspace's on-disk state and this binary:
/// current vs supported versions for both ledgers, plus whatever is pending
/// (dry-run) or was just applied (apply path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateStatus {
    /// The workspace `.orbit` directory the status describes.
    pub orbit_dir: PathBuf,
    /// Workspace `.orbit/` layout version recorded in the marker (0 =
    /// pre-versioning workspace that has never been opened by this binary).
    pub layout_version: u32,
    /// Newest layout version this binary supports.
    pub layout_supported: u32,
    /// Store-database schema version recorded in the `schema_meta` ledger.
    pub schema_version: u32,
    /// Newest schema version this binary supports.
    pub schema_supported: u32,
    /// This orbit binary's package version.
    pub binary_version: &'static str,
    /// Layout migrations pending against the marker, in apply order.
    pub pending_layout: Vec<LayoutMigrationInfo>,
    /// Schema migrations pending against the ledger, in apply order.
    pub pending_schema: Vec<PendingSchemaMigration>,
    /// Layout migrations the current runtime open auto-applied (apply path
    /// only; always empty for a dry-run, which never applies).
    pub applied_layout: Vec<LayoutMigrationInfo>,
}

impl MigrateStatus {
    /// Total migrations still pending across both ledgers.
    pub fn pending_total(&self) -> usize {
        self.pending_layout.len() + self.pending_schema.len()
    }

    /// True when either recorded version is newer than this binary supports
    /// (the workspace was written by a newer orbit).
    pub fn newer_than_binary(&self) -> bool {
        self.layout_version > self.layout_supported || self.schema_version > self.schema_supported
    }
}

/// Dry-run inspection against explicit roots. Read-only: reads the layout
/// marker and the schema ledger (read-only SQLite open), never applies.
pub fn migrate_dry_run_at(
    global_root: &Path,
    orbit_dir: &Path,
) -> Result<MigrateStatus, OrbitError> {
    let audit_db = orbit_core::config::resolved_audit_db_path(global_root, orbit_dir)?;
    let ledger = read_schema_ledger_status(&audit_db)?;

    Ok(MigrateStatus {
        orbit_dir: orbit_dir.to_path_buf(),
        layout_version: layout::current_layout_version(orbit_dir)?,
        layout_supported: SUPPORTED_LAYOUT_VERSION,
        schema_version: ledger.current_version,
        schema_supported: SUPPORTED_SCHEMA_VERSION,
        binary_version: env!("CARGO_PKG_VERSION"),
        pending_layout: layout::pending_layout_migrations(orbit_dir)?,
        pending_schema: ledger.pending,
        applied_layout: Vec::new(),
    })
}

/// `orbit migrate` command surface for [`OrbitRuntime`] (extension trait —
/// the implementation moved out of orbit-core in [ORB-10016]).
pub trait MigrateCommands {
    /// Migration status of an open runtime (the `orbit migrate --confirm` apply path).
    /// Opening the runtime already applied everything pending — layout in
    /// the open pre-flight, schema inside `Store::open` — so this reports
    /// the resulting versions plus which layout migrations that open
    /// auto-applied (see [`OrbitRuntime::layout_upgrade_report`]).
    fn migrate_status(&self) -> Result<MigrateStatus, OrbitError>;
}

impl MigrateCommands for OrbitRuntime {
    fn migrate_status(&self) -> Result<MigrateStatus, OrbitError> {
        let orbit_dir = self.shared_root();
        let schema_version = self.sqlite_store()?.schema_version()?;

        Ok(MigrateStatus {
            layout_version: layout::current_layout_version(&orbit_dir)?,
            layout_supported: SUPPORTED_LAYOUT_VERSION,
            schema_version,
            schema_supported: SUPPORTED_SCHEMA_VERSION,
            binary_version: env!("CARGO_PKG_VERSION"),
            pending_layout: layout::pending_layout_migrations(&orbit_dir)?,
            pending_schema: pending_schema_migrations_after(schema_version),
            applied_layout: self.layout_upgrade_report().applied.clone(),
            orbit_dir,
        })
    }
}
