//! Workspace-level self-diagnostics behind `orbit doctor` [ORB-10005].
//!
//! Complements the narrower `orbit skill doctor` / `orbit tool doctor`
//! surfaces with whole-workspace checks: config validity, store database
//! integrity and schema-ledger version, free disk space on the volume
//! holding `.orbit`, semantic-index staleness, leftover lock files from
//! crashed holders, orphaned `running`/`pending` job runs, task
//! relation/dependency targets that no longer resolve in the registry
//! (grandfathered relations that block index rebuilds — ORB-10305), and
//! learning/ADR id allocations pinned to a worktree that has since been
//! reaped (ORB-10501).
//!
//! Every check degrades rather than errors: subsystems that are absent in a
//! fresh workspace report [`WorkspaceDoctorStatus::Skipped`], and probe
//! failures become `Warning`/`Error` rows instead of aborting the whole
//! diagnosis. The cheap probes shared with the dashboard's
//! `/healthz?detailed=true` ([`DoctorCommands::health_check_store_writable`])
//! also live here.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use fs2::FileExt;
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

    /// Remove lock files left by dead holders, without disturbing a lock that
    /// is currently held by another process.
    fn remove_stale_lock_files(&self) -> Result<usize, OrbitError>;

    /// Remove retired graph state from the exact worktree-local and shared
    /// workspace locations. Missing locations are a successful no-op.
    fn remove_retired_graph_state(&self) -> Result<usize, OrbitError>;

    /// [ORB-10501] Abandon every learning/ADR allocation whose pinned worktree
    /// is gone and whose body is unreadable, returning how many rows were
    /// retired. Each row is re-verified by the owning store before its write.
    fn clear_orphaned_id_allocations(&self) -> Result<usize, OrbitError>;

    /// Cheap store write probe for health endpoints: open the store and
    /// acquire + roll back the write lock without mutating anything.
    fn health_check_store_writable(&self) -> Result<String, OrbitError>;
}

impl DoctorCommands for OrbitRuntime {
    fn doctor_workspace(&self) -> Result<Vec<WorkspaceDoctorResult>, OrbitError> {
        Ok(vec![
            doctor_check_config(self),
            doctor_check_database(self),
            doctor_check_disk_space(self),
            doctor_check_semantic_index(self),
            doctor_check_stale_locks(self),
            doctor_check_job_runs(self),
            doctor_check_task_relations(self),
            doctor_check_id_allocations(self),
        ])
    }

    fn remove_stale_lock_files(&self) -> Result<usize, OrbitError> {
        let mut removed = 0;
        for path in collect_lock_files(self.paths()) {
            if remove_stale_lock_file(&path)? {
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn remove_retired_graph_state(&self) -> Result<usize, OrbitError> {
        let targets = [
            (self.local_root(), Path::new("graph")),
            (self.shared_root(), Path::new("knowledge/graph")),
        ];
        let mut removed = 0;
        for (root, relative) in targets {
            if remove_workspace_subtree(&root, relative)? {
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn clear_orphaned_id_allocations(&self) -> Result<usize, OrbitError> {
        Ok(self.abandon_orphaned_id_allocations()?.len())
    }

    fn health_check_store_writable(&self) -> Result<String, OrbitError> {
        let store = self.sqlite_store()?;
        store.check_writable()?;
        Ok("store database accepts writes".to_string())
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

/// Lock files whose recorded holder PID is dead. Clean lock-guard releases
/// clear their diagnostic metadata while still holding the advisory `flock`,
/// so metadata found here is from an interrupted/crashed holder — a crash
/// signal, not an availability problem.
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

/// Delete one dead-holder lock only after acquiring its advisory lock. A
/// fresh holder can win the race between the initial scan and this cleanup;
/// in that case `try_lock_exclusive` reports contention and the file remains.
fn remove_stale_lock_file(path: &Path) -> Result<bool, OrbitError> {
    let Some(holder) = orbit_store::read_lock_holder(path) else {
        return Ok(false);
    };
    if process_is_alive(holder.pid) {
        return Ok(false);
    }

    let file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(OrbitError::Io(format!(
                "open stale lock candidate {}: {error}",
                path.display()
            )));
        }
    };
    match file.try_lock_exclusive() {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
        Err(error) => {
            return Err(OrbitError::Store(format!(
                "acquire stale lock candidate {}: {error}",
                path.display()
            )));
        }
    }

    // Re-read after acquiring the advisory lock so a holder that appeared
    // after the first liveness probe is never removed.
    let should_remove = orbit_store::read_lock_holder(path)
        .is_some_and(|current_holder| !process_is_alive(current_holder.pid));
    if !should_remove {
        return Ok(false);
    }

    std::fs::remove_file(path).map_err(|error| {
        OrbitError::Io(format!(
            "remove stale lock candidate {}: {error}",
            path.display()
        ))
    })?;
    Ok(true)
}

/// Remove one fixed workspace-relative subtree without following a symlink at
/// the subtree boundary. The relative path is validated even though current
/// callers pass constants, keeping future cleanup additions inside the
/// resolved Orbit root by construction.
fn remove_workspace_subtree(root: &Path, relative: &Path) -> Result<bool, OrbitError> {
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(OrbitError::InvalidInput(format!(
            "cleanup path '{}' must remain relative to Orbit root '{}'",
            relative.display(),
            root.display()
        )));
    }
    let target = root.join(relative);
    let metadata = match std::fs::symlink_metadata(&target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(OrbitError::Io(format!(
                "inspect retired graph state {}: {error}",
                target.display()
            )));
        }
    };
    let result = if metadata.file_type().is_symlink() || !metadata.is_dir() {
        std::fs::remove_file(&target)
    } else {
        std::fs::remove_dir_all(&target)
    };
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(OrbitError::Io(format!(
            "remove retired graph state {}: {error}",
            target.display()
        ))),
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
/// bundle-scan fallback (ORB-10305). Scoped to the current
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

/// How many orphaned allocations the doctor message names before summarizing
/// the rest. A workspace can accumulate dozens; the row stays readable while
/// still reporting the true total.
const ORPHANED_ALLOCATION_DETAIL_LIMIT: usize = 10;

/// Learning/ADR id allocations pinned to a worktree that no longer exists and
/// whose body is unreadable anywhere locally (ORB-10501). These rows can never
/// resolve again — the body died with its worktree — so they are permanent
/// dead weight in `learning list --include-remote` and the ADR list until they
/// are retired.
fn doctor_check_id_allocations(runtime: &OrbitRuntime) -> WorkspaceDoctorResult {
    let orphaned = match runtime.list_orphaned_id_allocations() {
        Ok(orphaned) => orphaned,
        Err(error) => {
            return check(
                "id-allocations",
                WorkspaceDoctorStatus::Warning,
                format!("cannot inspect id allocations: {error}"),
            );
        }
    };
    if orphaned.is_empty() {
        return check(
            "id-allocations",
            WorkspaceDoctorStatus::Ok,
            "no learning/ADR allocations pinned to a missing worktree".to_string(),
        );
    }
    let mut detail = orphaned
        .iter()
        .take(ORPHANED_ALLOCATION_DETAIL_LIMIT)
        .map(|record| {
            format!(
                "{} ({}, worktree {})",
                record.id,
                record.status,
                record.worktree_root.display()
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    if let Some(remaining) = orphaned.len().checked_sub(ORPHANED_ALLOCATION_DETAIL_LIMIT)
        && remaining > 0
    {
        detail.push_str(&format!("; and {remaining} more"));
    }
    check(
        "id-allocations",
        WorkspaceDoctorStatus::Warning,
        format!(
            "{} learning/ADR allocation(s) pinned to a worktree that no longer exists, with no \
             readable body — unrecoverable; clear with `orbit doctor \
             --fix-orphaned-allocations`: {detail}",
            orphaned.len()
        ),
    )
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
