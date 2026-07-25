//! Workspace-level self-diagnostics behind `orbit doctor` [ORB-10005].
//!
//! Complements the narrower `orbit skill doctor` / `orbit tool doctor`
//! surfaces with whole-workspace checks: config validity, store database
//! integrity and schema-ledger version, free disk space on the volume
//! holding `.orbit`, semantic/graph index staleness, leftover lock files
//! from crashed holders, orphaned `running`/`pending` job runs, and task
//! relation/dependency targets that no longer resolve in the registry
//! (grandfathered relations that block index rebuilds — ORB-10305).
//!
//! Every check degrades rather than errors: subsystems that are absent in a
//! fresh workspace report [`WorkspaceDoctorStatus::Skipped`], and probe
//! failures become `Warning`/`Error` rows instead of aborting the whole
//! diagnosis. The cheap probes shared with the dashboard's
//! `/healthz?detailed=true` ([`DoctorCommands::health_check_store_writable`],
//! [`DoctorCommands::health_check_graph_index`]) also live here.

use std::path::{Path, PathBuf};

use orbit_common::types::{OrbitError, WorkspacePaths};
use orbit_core::OrbitRuntime;
use orbit_store::sqlite::migration::SUPPORTED_SCHEMA_VERSION;
use serde::Serialize;

/// Outcome of one workspace doctor check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceDoctorStatus {
    /// Check passed.
    Ok,
    /// Something is off but the workspace remains usable.
    Warning,
    /// The workspace is unhealthy; `orbit doctor` exits nonzero.
    Error,
    /// The subsystem is absent (fresh workspace) — nothing to check.
    Skipped,
}

/// One row of `orbit doctor` output.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceDoctorResult {
    /// Stable check identifier (e.g. `config`, `database`, `disk-space`).
    pub check_name: String,
    /// Pass/warn/fail/skip outcome.
    pub status: WorkspaceDoctorStatus,
    /// Human-readable detail line.
    pub message: String,
}

fn check(name: &str, status: WorkspaceDoctorStatus, message: String) -> WorkspaceDoctorResult {
    WorkspaceDoctorResult {
        check_name: name.to_string(),
        status,
        message,
    }
}

/// Warn when the volume holding `.orbit` has less than this many free bytes.
const DISK_WARN_BYTES: u64 = 1024 * 1024 * 1024; // 1 GiB
/// Fail when the volume holding `.orbit` has less than this many free bytes.
const DISK_FAIL_BYTES: u64 = 256 * 1024 * 1024; // 256 MiB
/// Warn when less than this percentage of the volume is free.
const DISK_WARN_PCT: f64 = 5.0;
/// Fail when less than this percentage of the volume is free.
const DISK_FAIL_PCT: f64 = 1.0;

/// Workspace doctor / health-probe command surface for [`OrbitRuntime`]
/// (extension trait — the implementation moved out of orbit-core in
/// [ORB-10016]).
pub trait DoctorCommands {
    /// Run every workspace-level doctor check. Individual checks never abort
    /// the diagnosis: probe failures surface as `Warning`/`Error` rows and
    /// absent subsystems as `Skipped`.
    fn doctor_workspace(&self) -> Result<Vec<WorkspaceDoctorResult>, OrbitError>;

    /// Cheap store write probe for health endpoints: open the store and
    /// acquire + roll back the write lock without mutating anything.
    fn health_check_store_writable(&self) -> Result<String, OrbitError>;

    /// Cheap read probe of the newest code-graph database, if one exists.
    /// Returns `None` when no graph index has been built (fresh workspace),
    /// so callers can skip rather than fail.
    ///
    /// Deliberately opens the SQLite file directly instead of adding an
    /// orbit-cmd → orbit-graph dependency edge (see `ARCHITECTURE.md`).
    fn health_check_graph_index(&self) -> Option<Result<String, OrbitError>>;
}

impl DoctorCommands for OrbitRuntime {
    fn doctor_workspace(&self) -> Result<Vec<WorkspaceDoctorResult>, OrbitError> {
        Ok(vec![
            doctor_check_config(self),
            doctor_check_database(self),
            doctor_check_disk_space(self),
            doctor_check_semantic_index(self),
            doctor_check_graph_index(self),
            doctor_check_stale_locks(self),
            doctor_check_job_runs(self),
            doctor_check_task_relations(self),
        ])
    }

    fn health_check_store_writable(&self) -> Result<String, OrbitError> {
        let store = self.sqlite_store()?;
        store.check_writable()?;
        Ok("store database accepts writes".to_string())
    }

    fn health_check_graph_index(&self) -> Option<Result<String, OrbitError>> {
        let graph_dir = self.local_root().join("graph");
        let newest = newest_graph_db(&graph_dir)?;
        Some(read_graph_db(&newest))
    }
}

/// Parse + validate the effective (workspace-over-global) `config.toml`.
fn doctor_check_config(runtime: &OrbitRuntime) -> WorkspaceDoctorResult {
    let path = runtime.config_path();
    match orbit_core::config::validate_layered_config(&runtime.global_root(), &runtime.data_root())
    {
        Ok(_) => check(
            "config",
            WorkspaceDoctorStatus::Ok,
            format!("valid ({})", path.display()),
        ),
        Err(error) => check(
            "config",
            WorkspaceDoctorStatus::Error,
            format!("invalid ({}): {error}", path.display()),
        ),
    }
}

/// `PRAGMA quick_check` plus migration-ledger schema version vs binary.
fn doctor_check_database(runtime: &OrbitRuntime) -> WorkspaceDoctorResult {
    let store = match runtime.sqlite_store() {
        Ok(store) => store,
        Err(error) => {
            return check(
                "database",
                WorkspaceDoctorStatus::Error,
                format!("cannot open store database: {error}"),
            );
        }
    };
    if let Err(error) = store.quick_check() {
        return check(
            "database",
            WorkspaceDoctorStatus::Error,
            format!("integrity check failed: {error}"),
        );
    }
    match store.schema_version() {
        Ok(version) if version == SUPPORTED_SCHEMA_VERSION => check(
            "database",
            WorkspaceDoctorStatus::Ok,
            format!("quick_check ok; schema version {version} matches this binary"),
        ),
        Ok(version) if version < SUPPORTED_SCHEMA_VERSION => check(
            "database",
            WorkspaceDoctorStatus::Warning,
            format!(
                "quick_check ok; schema version {version} is behind this binary \
                 ({SUPPORTED_SCHEMA_VERSION}) — migrations apply on next store open"
            ),
        ),
        Ok(version) => check(
            "database",
            WorkspaceDoctorStatus::Error,
            format!(
                "schema version {version} is newer than this binary supports \
                 ({SUPPORTED_SCHEMA_VERSION}); upgrade orbit"
            ),
        ),
        Err(error) => check(
            "database",
            WorkspaceDoctorStatus::Warning,
            format!("quick_check ok; cannot read migration ledger: {error}"),
        ),
    }
}

/// Free space on the volume holding the workspace `.orbit` directory.
fn doctor_check_disk_space(runtime: &OrbitRuntime) -> WorkspaceDoctorResult {
    let root = runtime.local_root();
    disk_space_check(&root)
}

/// Semantic (docs/tasks/learnings) embedding index staleness, using the
/// stale-row signal the vector store already tracks.
fn doctor_check_semantic_index(runtime: &OrbitRuntime) -> WorkspaceDoctorResult {
    match runtime.semantic_stats() {
        Err(error) => check(
            "semantic-index",
            WorkspaceDoctorStatus::Warning,
            format!("cannot read semantic index: {error}"),
        ),
        Ok(stats) => {
            let total: usize = stats.rows.counts.iter().map(|count| count.rows).sum();
            if total == 0 {
                check(
                    "semantic-index",
                    WorkspaceDoctorStatus::Skipped,
                    "no semantic embeddings indexed yet".to_string(),
                )
            } else if stats.rows.stale_rows > 0 {
                check(
                    "semantic-index",
                    WorkspaceDoctorStatus::Warning,
                    format!(
                        "{} of {total} embedding rows are stale; re-run `orbit semantic index`",
                        stats.rows.stale_rows
                    ),
                )
            } else {
                check(
                    "semantic-index",
                    WorkspaceDoctorStatus::Ok,
                    format!("{total} embedding rows, none stale"),
                )
            }
        }
    }
}

/// Code-graph index presence + readability (skip when never built).
fn doctor_check_graph_index(runtime: &OrbitRuntime) -> WorkspaceDoctorResult {
    match runtime.health_check_graph_index() {
        None => check(
            "graph-index",
            WorkspaceDoctorStatus::Skipped,
            "no graph index built (the code-graph CLI surface was removed in ORB-10357; \
             this check is a legacy vestige with no build command)"
                .to_string(),
        ),
        Some(Ok(detail)) => check("graph-index", WorkspaceDoctorStatus::Ok, detail),
        Some(Err(error)) => check(
            "graph-index",
            WorkspaceDoctorStatus::Warning,
            format!(
                "graph index unreadable ({error}); no rebuild command exists \
                 (the code-graph CLI surface was removed in ORB-10357)"
            ),
        ),
    }
}

/// Lock files whose recorded holder PID is dead. Advisory `flock`s are
/// released by the OS on process death, so these are leftover metadata
/// from crashed holders — a crash signal, not an availability problem.
fn doctor_check_stale_locks(runtime: &OrbitRuntime) -> WorkspaceDoctorResult {
    let lock_files = collect_lock_files(runtime.paths());
    let mut stale = Vec::new();
    for path in &lock_files {
        let Some(holder) = orbit_store::read_lock_holder(path) else {
            continue;
        };
        if !process_is_alive(holder.pid) {
            stale.push(format!(
                "{} (dead pid {}, op: {}, since {})",
                path.display(),
                holder.pid,
                holder.label,
                holder.acquired_at
            ));
        }
    }
    if stale.is_empty() {
        check(
            "stale-locks",
            WorkspaceDoctorStatus::Ok,
            format!("{} lock file(s) scanned, none stale", lock_files.len()),
        )
    } else {
        check(
            "stale-locks",
            WorkspaceDoctorStatus::Warning,
            format!(
                "{} lock file(s) left by dead holders (the OS already released the \
                 flock; safe to delete): {}",
                stale.len(),
                stale.join("; ")
            ),
        )
    }
}

/// `running`/`pending` job runs with no live worker process — dead recorded
/// owner, or a queued run no worker ever claimed (read-only view of the
/// reconcile signal; see `job/run/reconcile.rs`) [ORB-10070].
fn doctor_check_job_runs(runtime: &OrbitRuntime) -> WorkspaceDoctorResult {
    let running = match runtime.list_orphaned_running_job_runs() {
        Ok(orphans) => orphans,
        Err(error) => {
            return check(
                "job-runs",
                WorkspaceDoctorStatus::Warning,
                format!("cannot inspect job runs: {error}"),
            );
        }
    };
    let pending = match runtime.list_orphaned_pending_job_runs() {
        Ok(orphans) => orphans,
        Err(error) => {
            return check(
                "job-runs",
                WorkspaceDoctorStatus::Warning,
                format!("cannot inspect job runs: {error}"),
            );
        }
    };
    if running.is_empty() && pending.is_empty() {
        return check(
            "job-runs",
            WorkspaceDoctorStatus::Ok,
            "no orphaned running or pending job runs".to_string(),
        );
    }
    let mut segments = Vec::new();
    if !running.is_empty() {
        let ids: Vec<&str> = running.iter().map(|run| run.run_id.as_str()).collect();
        segments.push(format!(
            "{} running run(s) whose owner process is gone: {} — they \
             finalize as `interrupted` on reconcile; resume with \
             `orbit job resume <run_id>`",
            running.len(),
            ids.join(", ")
        ));
    }
    if !pending.is_empty() {
        let ids: Vec<&str> = pending.iter().map(|run| run.run_id.as_str()).collect();
        segments.push(format!(
            "{} pending run(s) with no live worker process: {} — they \
             finalize as `interrupted` on reconcile; clear immediately with \
             `orbit run cancel <run_id>`",
            pending.len(),
            ids.join(", ")
        ));
    }
    check(
        "job-runs",
        WorkspaceDoctorStatus::Warning,
        segments.join("; "),
    )
}

/// Task relation/dependency targets that no longer resolve to a registered
/// task bundle — the "grandfathered" relations that make a generated task
/// index fail to rebuild against its relation validator, forcing an unbounded
/// bundle-scan fallback (ORB-10305 / ORB-10246). Scoped to the current
/// workspace; surfacing them here lets an operator fix or remove the offending
/// relation before the validator trips over it at rebuild time.
fn doctor_check_task_relations(runtime: &OrbitRuntime) -> WorkspaceDoctorResult {
    let workspace_id = match runtime.workspace_id() {
        Ok(id) => id,
        Err(error) => {
            return check(
                "task-relations",
                WorkspaceDoctorStatus::Warning,
                format!("cannot resolve workspace id to audit relations: {error}"),
            );
        }
    };
    match runtime.audit_dangling_relations(Some(&workspace_id)) {
        Err(error) => check(
            "task-relations",
            WorkspaceDoctorStatus::Warning,
            format!("cannot audit task relations: {error}"),
        ),
        Ok(dangling) if dangling.is_empty() => check(
            "task-relations",
            WorkspaceDoctorStatus::Ok,
            "no unresolved relation/dependency targets".to_string(),
        ),
        Ok(dangling) => {
            let detail = dangling
                .iter()
                .map(|target| {
                    format!(
                        "{} ({}) -> {}",
                        target.source_task_id, target.relation_type, target.target_task_id
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            check(
                "task-relations",
                WorkspaceDoctorStatus::Warning,
                format!(
                    "{} unresolved relation/dependency target(s) will block index rebuild \
                     until fixed or removed: {detail}",
                    dangling.len()
                ),
            )
        }
    }
}

/// Free/total space thresholds for the volume containing `path`.
pub(crate) fn disk_space_check(path: &Path) -> WorkspaceDoctorResult {
    let (available, total) = match (fs2::available_space(path), fs2::total_space(path)) {
        (Ok(available), Ok(total)) => (available, total),
        (Err(error), _) | (_, Err(error)) => {
            return check(
                "disk-space",
                WorkspaceDoctorStatus::Warning,
                format!(
                    "cannot determine free space for {}: {error}",
                    path.display()
                ),
            );
        }
    };
    let pct_free = if total == 0 {
        100.0
    } else {
        available as f64 * 100.0 / total as f64
    };
    let message = format!(
        "{} free of {} ({pct_free:.1}%) on the volume holding {}",
        human_bytes(available),
        human_bytes(total),
        path.display()
    );
    let status = if available < DISK_FAIL_BYTES || pct_free < DISK_FAIL_PCT {
        WorkspaceDoctorStatus::Error
    } else if available < DISK_WARN_BYTES || pct_free < DISK_WARN_PCT {
        WorkspaceDoctorStatus::Warning
    } else {
        WorkspaceDoctorStatus::Ok
    };
    check("disk-space", status, message)
}

/// Lock files in the directories the file-backed stores lock in:
/// `state/` (ID allocator), `tasks/` (v2 bundle locks), `learnings/`,
/// and `adrs/.locks/`. Non-recursive on purpose — the lock layouts are flat.
pub(crate) fn collect_lock_files(paths: &WorkspacePaths) -> Vec<PathBuf> {
    let dirs = [
        paths.state_dir.clone(),
        paths.tasks_dir.clone(),
        paths.learnings_dir.clone(),
        paths.adrs_dir.join(".locks"),
    ];
    let mut lock_files = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_lock = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".lock"));
            if is_lock && path.is_file() {
                lock_files.push(path);
            }
        }
    }
    lock_files
}

/// Liveness probe for a recorded holder PID. `kill(pid, 0)` — EPERM still
/// means alive. Conservative on doubt: an unprobeable PID counts as alive so
/// a live holder is never reported stale.
#[cfg(unix)]
pub(crate) fn process_is_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return true;
    };
    if pid <= 0 {
        return false;
    }
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Non-Unix: no cheap probe; treat every holder as alive (never report stale).
#[cfg(not(unix))]
pub(crate) fn process_is_alive(_pid: u32) -> bool {
    true
}

/// Newest `*.db` file in the graph directory, by modification time.
fn newest_graph_db(graph_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(graph_dir).ok()?;
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "db") && path.is_file())
        .max_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .ok()
        })
}

/// Open a graph database read-only and prove it answers a trivial query.
fn read_graph_db(path: &Path) -> Result<String, OrbitError> {
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| OrbitError::Store(format!("open {}: {error}", path.display())))?;
    let objects: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
        .map_err(|error| OrbitError::Store(format!("read {}: {error}", path.display())))?;
    let age_suffix = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .map(|age| format!(", last synced {}", human_age(age)))
        .unwrap_or_default();
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    Ok(format!(
        "{name} readable ({objects} schema objects{age_suffix})"
    ))
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn human_age(age: std::time::Duration) -> String {
    let secs = age.as_secs();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}
