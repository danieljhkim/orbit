//! Versioned `.orbit/` workspace-layout migrations (ORB-10012, P3.4).
//!
//! The SQLite schema ledger ([`crate::sqlite::migration`]) versions the store
//! *database*; this module versions everything else about a workspace's
//! on-disk `.orbit/` layout — directory structure, non-SQLite state files,
//! log/index locations. Layout migrations are an ordered, append-only
//! registry of `(version, name, description, apply)` entries mirroring the
//! schema ledger; the next breaking `.orbit/` change becomes a registry
//! entry instead of an undocumented break (see `RELEASING.md`).
//!
//! The current layout version is recorded in a plain-text marker file at
//! `<orbit_dir>/state/layout.version`. A marker file — not a `schema_meta`
//! row — because layout migrations may need to run *before* the store
//! database can open (a migration may move or restructure the database's own
//! location), and because reading one tiny file keeps the workspace-open
//! pre-flight cheap (no extra SQLite open on the hot path). A missing marker
//! means "pre-versioning workspace": all migrations run from the start, so
//! the v1 baseline (a no-op — version 1 *is* the current shape) adopts
//! existing workspaces exactly like the schema ledger's idempotent baseline
//! adopts legacy databases.
//!
//! Guarantees:
//! - **Auto-upgrade on open.** [`upgrade_workspace_layout`] runs as a
//!   pre-flight when a workspace opens (matching how the SQLite ledger
//!   auto-applies inside `Store::open`); `orbit migrate` is the explicit
//!   inspection/apply surface.
//! - **Downgrade guard.** A marker newer than
//!   [`SUPPORTED_LAYOUT_VERSION`] refuses to open with
//!   [`OrbitError::Migration`], mirroring the schema ledger's guard.
//! - **Crash tolerance.** Every migration MUST be idempotent (or stage via
//!   write-new-then-swap): the marker is advanced (atomic temp-file +
//!   rename) only *after* a migration's `apply` returns, so a crash in
//!   between re-runs that migration on the next open. The marker itself is
//!   gitignored runtime state — a fresh clone of a migrated workspace simply
//!   re-runs the idempotent migrations and re-stamps.
//! - **Single upgrader.** Pending migrations apply under an advisory file
//!   lock (`state/layout.lock`) with a re-check of the marker after
//!   acquisition, so concurrent opens do not interleave migrations. The
//!   up-to-date fast path never touches the lock.

use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;

/// Highest workspace-layout version this binary knows how to produce.
/// Bump together with a new [`LAYOUT_MIGRATIONS`] entry — never without one.
pub const SUPPORTED_LAYOUT_VERSION: u32 = 1;

/// Marker file recording the workspace's current layout version, relative to
/// the workspace `.orbit` directory. Lives under `state/` (gitignored
/// runtime state) so stamping a workspace never dirties repositories that
/// commit parts of `.orbit/`.
const MARKER_FILE: &str = "layout.version";

/// Advisory lock serializing concurrent upgraders, relative to `state/`.
const UPGRADE_LOCK_FILE: &str = "layout.lock";

/// One entry in the layout-migration registry.
pub(crate) struct LayoutMigration {
    pub(crate) version: u32,
    pub(crate) name: &'static str,
    /// One-line human description surfaced by `orbit migrate --dry-run`.
    pub(crate) description: &'static str,
    /// Applies the migration to the workspace `.orbit` directory. MUST be
    /// idempotent or staged (write-new-then-swap): a crash between `apply`
    /// and the marker write re-runs it on the next open. Directories the
    /// migration expects may be absent (fresh or partially-populated
    /// workspaces) — treat "nothing to do" as success.
    pub(crate) apply: fn(&Path) -> Result<(), OrbitError>,
}

/// Stable ordered registry of workspace-layout migrations. Append-only:
/// never renumber or edit an entry that has shipped. Every breaking
/// `.orbit/` layout change REQUIRES an entry here (see `RELEASING.md`).
pub(crate) const LAYOUT_MIGRATIONS: &[LayoutMigration] = &[LayoutMigration {
    version: 1,
    name: "baseline",
    description: "adopt the versioned .orbit/ layout (records the current shape; changes nothing)",
    apply: apply_baseline_layout,
}];

/// v1 baseline: the current `.orbit/` shape. Intentionally a no-op — running
/// it on any existing workspace changes nothing and then records version 1,
/// which is how pre-versioning workspaces adopt the marker.
fn apply_baseline_layout(_orbit_dir: &Path) -> Result<(), OrbitError> {
    Ok(())
}

/// Registry metadata for one layout migration, as surfaced to `orbit
/// migrate` (pending listings and applied reports).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutMigrationInfo {
    pub version: u32,
    pub name: String,
    pub description: String,
}

impl LayoutMigrationInfo {
    fn from_entry(entry: &LayoutMigration) -> Self {
        Self {
            version: entry.version,
            name: entry.name.to_string(),
            description: entry.description.to_string(),
        }
    }
}

/// Outcome of a workspace-layout pre-flight/upgrade.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutUpgradeReport {
    /// Marker version before the upgrade (0 = pre-versioning workspace).
    pub from_version: u32,
    /// Marker version after the upgrade.
    pub to_version: u32,
    /// Migrations applied by this call, in order. Empty when the workspace
    /// was already current.
    pub applied: Vec<LayoutMigrationInfo>,
}

/// Current layout version recorded in the workspace marker; 0 when no marker
/// exists (fresh or pre-versioning workspace).
pub fn current_layout_version(orbit_dir: &Path) -> Result<u32, OrbitError> {
    read_marker(orbit_dir)
}

/// Registry migrations newer than the workspace's recorded layout version,
/// in apply order. Empty when the workspace is current (or newer than this
/// binary — compare versions to distinguish; see [`SUPPORTED_LAYOUT_VERSION`]).
pub fn pending_layout_migrations(orbit_dir: &Path) -> Result<Vec<LayoutMigrationInfo>, OrbitError> {
    pending_with(orbit_dir, LAYOUT_MIGRATIONS)
}

/// Bring the workspace `.orbit` layout up to the newest supported version,
/// applying any pending registry migrations and advancing the marker after
/// each one. Refuses a workspace whose recorded layout version is newer than
/// this binary supports. The up-to-date fast path costs one marker read.
pub fn upgrade_workspace_layout(orbit_dir: &Path) -> Result<LayoutUpgradeReport, OrbitError> {
    upgrade_with(orbit_dir, LAYOUT_MIGRATIONS)
}

pub(crate) fn pending_with(
    orbit_dir: &Path,
    migrations: &[LayoutMigration],
) -> Result<Vec<LayoutMigrationInfo>, OrbitError> {
    validate_registry(migrations)?;
    let current = read_marker(orbit_dir)?;
    Ok(migrations
        .iter()
        .filter(|m| m.version > current)
        .map(LayoutMigrationInfo::from_entry)
        .collect())
}

pub(crate) fn upgrade_with(
    orbit_dir: &Path,
    migrations: &[LayoutMigration],
) -> Result<LayoutUpgradeReport, OrbitError> {
    validate_registry(migrations)?;
    let supported = migrations.last().map(|m| m.version).unwrap_or(0);

    let current = read_marker(orbit_dir)?;
    refuse_newer(orbit_dir, current, supported)?;
    if current == supported {
        return Ok(LayoutUpgradeReport {
            from_version: current,
            to_version: current,
            applied: Vec::new(),
        });
    }

    // Slow path: serialize concurrent upgraders, then re-check the marker —
    // another process may have finished the upgrade while we waited.
    let _guard =
        crate::file_lock::acquire_exclusive(&upgrade_lock_path(orbit_dir), "layout upgrade")?;
    let from_version = read_marker(orbit_dir)?;
    refuse_newer(orbit_dir, from_version, supported)?;

    let mut applied = Vec::new();
    let mut version = from_version;
    for migration in migrations.iter().filter(|m| m.version > from_version) {
        (migration.apply)(orbit_dir).map_err(|error| {
            OrbitError::Migration(format!(
                "layout migration v{} ({}) failed for '{}': {error}",
                migration.version,
                migration.name,
                orbit_dir.display()
            ))
        })?;
        write_marker(orbit_dir, migration.version)?;
        version = migration.version;
        applied.push(LayoutMigrationInfo::from_entry(migration));
        orbit_common::tracing::info!(
            target: "orbit.store.layout",
            version = migration.version,
            name = migration.name,
            orbit_dir = %orbit_dir.display(),
            "applied workspace layout migration",
        );
    }

    Ok(LayoutUpgradeReport {
        from_version,
        to_version: version,
        applied,
    })
}

fn refuse_newer(orbit_dir: &Path, current: u32, supported: u32) -> Result<(), OrbitError> {
    if current > supported {
        return Err(OrbitError::Migration(format!(
            "workspace '{}' has .orbit layout version {current}, newer than the newest version \
             this orbit binary supports ({supported}); upgrade orbit to open this workspace",
            orbit_dir.display()
        )));
    }
    Ok(())
}

fn validate_registry(migrations: &[LayoutMigration]) -> Result<(), OrbitError> {
    let mut previous = 0u32;
    for migration in migrations {
        if migration.version <= previous {
            return Err(OrbitError::Migration(format!(
                "layout migration registry is not strictly increasing: v{} ({}) follows v{previous}",
                migration.version, migration.name
            )));
        }
        previous = migration.version;
    }
    Ok(())
}

fn marker_path(orbit_dir: &Path) -> PathBuf {
    orbit_dir.join("state").join(MARKER_FILE)
}

fn upgrade_lock_path(orbit_dir: &Path) -> PathBuf {
    orbit_dir.join("state").join(UPGRADE_LOCK_FILE)
}

fn read_marker(orbit_dir: &Path) -> Result<u32, OrbitError> {
    let path = marker_path(orbit_dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(OrbitError::Migration(format!(
                "cannot read layout version marker '{}': {error}",
                path.display()
            )));
        }
    };
    raw.trim().parse::<u32>().map_err(|_| {
        OrbitError::Migration(format!(
            "corrupt layout version marker '{}': {:?} is not a version number \
             (delete the file to re-adopt from version 0 — migrations are idempotent)",
            path.display(),
            raw.trim()
        ))
    })
}

/// Advance the marker atomically (temp file + rename), so a crash mid-write
/// leaves either the old or the new version, never a torn marker.
fn write_marker(orbit_dir: &Path, version: u32) -> Result<(), OrbitError> {
    let path = marker_path(orbit_dir);
    let map_err = |op: &str, error: std::io::Error| {
        OrbitError::Migration(format!(
            "cannot {op} layout version marker '{}': {error}",
            path.display()
        ))
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| map_err("create directory for", e))?;
    }
    let tmp = path.with_extension("version.tmp");
    std::fs::write(&tmp, format!("{version}\n")).map_err(|e| map_err("stage", e))?;
    std::fs::rename(&tmp, &path).map_err(|e| map_err("commit", e))
}

#[cfg(test)]
mod tests;
