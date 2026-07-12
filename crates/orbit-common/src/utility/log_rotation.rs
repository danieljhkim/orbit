//! Rotation + retention for the global JSONL tracing feed
//! (`~/.orbit/state/logs/orbit.jsonl`). [ORB-00415]
//!
//! On an always-on host the JSONL feed would otherwise grow unbounded until the
//! disk fills — which then cascades into SQLite write failures across every
//! store. This module bounds it with an opportunistic, rename-based roll plus a
//! retention sweep, both run once at subscriber init (cheap, no daemon).
//!
//! The active file stays at the fixed path `orbit.jsonl` — readers
//! (`orbit log tail`, the dashboard) open that exact path, so we do NOT adopt
//! `tracing-appender`'s dated active filenames. Instead, when the active file
//! exceeds the per-file budget it is renamed to a dated archive
//! (`orbit.jsonl.<UTC-timestamp>`) and the subscriber reopens a fresh active
//! file. Retention then prunes archives older than the age budget or beyond the
//! total-size budget (delete-only; no compression, to avoid pulling a
//! compression codec into `orbit-common`).
//!
//! Concurrency: rolls are rare (size-triggered) and rename is atomic. A
//! long-running process that keeps its fd open after another process rolls the
//! file continues writing valid JSONL into the renamed inode (Unix) — no
//! corruption, though those lines land in the archive rather than the new
//! active file. That trade-off is acceptable given the criterion is
//! corruption-freedom, not real-time reader completeness.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::Utc;

use crate::types::OrbitError;

const DEFAULT_RETENTION_DAYS: u64 = 7;
const DEFAULT_MAX_TOTAL_MB: u64 = 500;
const DEFAULT_MAX_FILE_MB: u64 = 100;
const BYTES_PER_MB: u64 = 1024 * 1024;
const SECONDS_PER_DAY: u64 = 86_400;

/// Resolved rotation/retention budgets for the global JSONL feed. Overridable
/// via the `[runtime]` section of `~/.orbit/config.toml`
/// (`log_retention_days`, `log_max_total_mb`, `log_max_file_mb`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogRotationConfig {
    /// Delete archives whose mtime is older than this many days.
    pub retention_days: u64,
    /// Total byte budget across archive files; oldest are deleted first when
    /// exceeded.
    pub max_total_bytes: u64,
    /// Roll the active file once it grows beyond this many bytes.
    pub max_file_bytes: u64,
}

impl Default for LogRotationConfig {
    fn default() -> Self {
        Self {
            retention_days: DEFAULT_RETENTION_DAYS,
            max_total_bytes: DEFAULT_MAX_TOTAL_MB * BYTES_PER_MB,
            max_file_bytes: DEFAULT_MAX_FILE_MB * BYTES_PER_MB,
        }
    }
}

impl LogRotationConfig {
    /// Build from raw `[runtime]` values (megabytes / days), validating each.
    /// A `None` field falls back to the conservative default. Returns a clear
    /// [`OrbitError::InvalidInput`] for out-of-range values so the config
    /// loader can reject a malformed config at load time.
    pub fn from_parts(
        retention_days: Option<u64>,
        max_total_mb: Option<u64>,
        max_file_mb: Option<u64>,
    ) -> Result<Self, OrbitError> {
        let retention_days = retention_days.unwrap_or(DEFAULT_RETENTION_DAYS);
        let max_total_mb = max_total_mb.unwrap_or(DEFAULT_MAX_TOTAL_MB);
        let max_file_mb = max_file_mb.unwrap_or(DEFAULT_MAX_FILE_MB);

        if retention_days == 0 {
            return Err(OrbitError::InvalidInput(
                "[runtime] log_retention_days must be >= 1".to_string(),
            ));
        }
        if max_total_mb == 0 {
            return Err(OrbitError::InvalidInput(
                "[runtime] log_max_total_mb must be >= 1".to_string(),
            ));
        }
        if max_file_mb == 0 {
            return Err(OrbitError::InvalidInput(
                "[runtime] log_max_file_mb must be >= 1".to_string(),
            ));
        }
        if max_file_mb > max_total_mb {
            return Err(OrbitError::InvalidInput(format!(
                "[runtime] log_max_file_mb ({max_file_mb}) must be <= log_max_total_mb ({max_total_mb})"
            )));
        }

        Ok(Self {
            retention_days,
            max_total_bytes: max_total_mb * BYTES_PER_MB,
            max_file_bytes: max_file_mb * BYTES_PER_MB,
        })
    }

    /// Read rotation config from the global `~/.orbit/config.toml` `[runtime]`
    /// section, falling back to [`LogRotationConfig::default`] on any error
    /// (missing file, parse error, or invalid values). Lenient by design: the
    /// subscriber initializes before argument parsing and config load, so it
    /// must never fail here. `orbit-core`'s config loader validates the same
    /// keys strictly and surfaces a clear load-time error.
    pub fn load_global_best_effort() -> Self {
        load_global().unwrap_or_default()
    }

    /// The age budget expressed as a [`Duration`]. Saturates the multiply so an
    /// absurd (but validated nonzero) `retention_days` cannot overflow.
    pub fn retention_window(&self) -> Duration {
        Duration::from_secs(self.retention_days.saturating_mul(SECONDS_PER_DAY))
    }
}

/// Why the retention policy selected an archive for deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneReason {
    /// The archive's mtime is older than the retention window.
    Age,
    /// The archive was selected oldest-first to bring the surviving set back
    /// under the total-size budget.
    Size,
}

/// One archive the retention policy would delete, with the size and mtime
/// observed during the scan and the reason it was selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneCandidate {
    pub path: PathBuf,
    pub bytes: u64,
    pub modified: SystemTime,
    pub reason: PruneReason,
}

/// The archives beside an active log file that the retention policy would
/// delete, plus what was scanned. The active file is never included.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrunePlan {
    /// Number of archive files scanned (the active file is excluded).
    pub scanned: u64,
    /// Total bytes across all scanned archives.
    pub scanned_bytes: u64,
    /// Archives selected for deletion: age-selected first, then size-selected.
    pub candidates: Vec<PruneCandidate>,
}

/// Classify the archives beside `active_path` for deletion under the `retention`
/// age window and `max_total_bytes` total-size budget, relative to `now`.
///
/// Pure classification — performs no deletion. Age-selected archives come
/// first, then, from the survivors, the oldest are size-selected until the set
/// fits the budget. Only dated `<active_name>.<stamp>` archives are considered;
/// the active file itself (exactly `active_name`) is never a candidate. This is
/// the single classifier shared by subscriber-init pruning
/// ([`prune_archives`]) and the `orbit gc logs` collector, so the two never
/// disagree.
pub fn plan_prune(
    active_path: &Path,
    retention: Duration,
    max_total_bytes: u64,
    now: SystemTime,
) -> std::io::Result<PrunePlan> {
    let Some(dir) = active_path.parent() else {
        return Ok(PrunePlan::default());
    };
    let active_name = active_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("orbit.jsonl");
    // Archives are `<active_name>.<stamp>`; never touch the active file itself.
    let prefix = format!("{active_name}.");

    let mut archives: Vec<(PathBuf, SystemTime, u64)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        archives.push((entry.path(), mtime, meta.len()));
    }

    let scanned = archives.len() as u64;
    let scanned_bytes = archives.iter().map(|(_, _, size)| *size).sum();
    let mut candidates = Vec::new();

    // Age-based selection.
    if let Some(cutoff) = now.checked_sub(retention) {
        archives.retain(|(path, mtime, size)| {
            if *mtime < cutoff {
                candidates.push(PruneCandidate {
                    path: path.clone(),
                    bytes: *size,
                    modified: *mtime,
                    reason: PruneReason::Age,
                });
                false
            } else {
                true
            }
        });
    }

    // Size-based selection over the survivors: oldest first until under budget.
    let mut total: u64 = archives.iter().map(|(_, _, size)| *size).sum();
    if total > max_total_bytes {
        archives.sort_by_key(|(_, mtime, _)| *mtime); // oldest first
        for (path, mtime, size) in &archives {
            if total <= max_total_bytes {
                break;
            }
            candidates.push(PruneCandidate {
                path: path.clone(),
                bytes: *size,
                modified: *mtime,
                reason: PruneReason::Size,
            });
            total = total.saturating_sub(*size);
        }
    }

    Ok(PrunePlan {
        scanned,
        scanned_bytes,
        candidates,
    })
}

fn load_global() -> Option<LogRotationConfig> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|value| !value.is_empty())?;
    let path = Path::new(&home).join(".orbit").join("config.toml");
    let raw = std::fs::read_to_string(&path).ok()?;
    let value: toml::Value = toml::from_str(&raw).ok()?;
    let runtime = value.get("runtime")?;
    let read_u64 = |key: &str| {
        runtime
            .get(key)
            .and_then(toml::Value::as_integer)
            .and_then(|integer| u64::try_from(integer).ok())
    };
    LogRotationConfig::from_parts(
        read_u64("log_retention_days"),
        read_u64("log_max_total_mb"),
        read_u64("log_max_file_mb"),
    )
    .ok()
}

/// Opportunistically roll the active log if oversized, then prune archives by
/// age and total-size budget. Best-effort: logs a warning on failure but never
/// panics or fails the caller. Intended to run once at subscriber init.
pub fn rotate_and_prune(active_path: &Path, config: &LogRotationConfig) {
    if let Err(error) = maybe_roll(active_path, config) {
        tracing::warn!(
            target: "orbit.logging.rotation",
            path = %active_path.display(),
            error = %error,
            "failed to roll oversized JSONL log",
        );
    }
    if let Err(error) = prune_archives(active_path, config) {
        tracing::warn!(
            target: "orbit.logging.rotation",
            error = %error,
            "failed to prune JSONL log archives",
        );
    }
}

/// Rename the active file to a dated archive when it exceeds the per-file
/// budget. Returns `Ok(())` (no-op) when the file is absent or within budget.
pub(crate) fn maybe_roll(active_path: &Path, config: &LogRotationConfig) -> std::io::Result<()> {
    let size = match std::fs::metadata(active_path) {
        Ok(meta) => meta.len(),
        Err(_) => return Ok(()), // no active file yet
    };
    if size <= config.max_file_bytes {
        return Ok(());
    }
    std::fs::rename(active_path, archive_path(active_path))
}

fn archive_path(active_path: &Path) -> PathBuf {
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%3fZ");
    let name = active_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("orbit.jsonl");
    active_path.with_file_name(format!("{name}.{stamp}"))
}

/// Delete archives older than the age budget, then, if the surviving archive
/// set still exceeds the total-size budget, delete oldest-first until it fits.
/// Delegates selection to [`plan_prune`] (the shared classifier) so startup
/// pruning and `orbit gc logs` apply identical retention policy; deletion here
/// is best-effort per file.
pub(crate) fn prune_archives(
    active_path: &Path,
    config: &LogRotationConfig,
) -> std::io::Result<()> {
    // ADR-0221: startup pruning is retained but routed through the shared
    // `plan_prune` classifier so `orbit gc logs` cannot disagree with it.
    let plan = plan_prune(
        active_path,
        config.retention_window(),
        config.max_total_bytes,
        SystemTime::now(),
    )?;
    for candidate in &plan.candidates {
        let _ = std::fs::remove_file(&candidate.path);
    }
    Ok(())
}
