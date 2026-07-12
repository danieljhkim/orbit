//! [ORB-10173] Pipeline worktree garbage collection.
//!
//! Orbit creates a git worktree per pipeline run under
//! `.orbit/state/worktrees/<prefix>-<run_id>/` and historically never reclaimed
//! it, so disk grew by the full build-artifact footprint (Rust `target/`, Java
//! `build/`, `node_modules`, …) every run and never shrank — a slow leak that
//! re-filled dk-server-1's root fs within weeks (F2026-07-019, and the
//! orphaned-run family of F2026-07-016 / ORB-10153).
//!
//! This module reclaims those worktrees under a retention policy:
//!
//! - **Success / cancelled / skipped** runs reap immediately — the branch is
//!   already merged or pushed, nothing debuggable remains.
//! - **Failed / timeout / interrupted** runs are retained for a configurable
//!   window (default [`DEFAULT_FAILED_RETENTION_DAYS`]) so a human can inspect
//!   the failing tree, then reaped once they age out.
//! - **A live run's worktree is never touched.** "Live" means a non-terminal
//!   run record whose owner process is still alive (probed via the same
//!   liveness helpers the orphan reconciler uses). This is the
//!   concurrency-safety guarantee: a worktree belonging to an in-flight run is
//!   never a reclaim candidate, even while other runs sweep concurrently.
//! - **Orphaned non-terminal records** (a `pending`/`running` row whose worker
//!   is conclusively gone) are cancelled, then their worktree is reclaimed.
//! - **Worktrees with no run record at all** are reclaimed as pure orphans.
//!
//! Reclaim is **language-agnostic**: it removes the worktree directory whatever
//! the workspace's build system put there, with no Cargo-specific casing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::{JobRun, JobRunState, OrbitError};

use crate::OrbitRuntime;

use super::owner::{pending_run_stale_reason, running_run_owner_is_stale};

/// Default retention window for failed / timeout / interrupted run worktrees.
/// A failed run's tree is useful for debugging (and an interrupted run's is
/// resumable), so keep it this many days before reaping.
pub const DEFAULT_FAILED_RETENTION_DAYS: i64 = 7;

/// Options for one worktree GC pass.
#[derive(Debug, Clone, Default)]
pub struct WorktreeGcOptions {
    /// Report what would be reclaimed without removing anything or cancelling
    /// any orphaned run record.
    pub dry_run: bool,
    /// Override the failed/timeout/interrupted retention window (days). When
    /// `None`, the workspace-configured value is used.
    pub failed_retention_days: Option<i64>,
}

/// What a GC pass decided to do with a single worktree directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeGcAction {
    /// Reclaimed: the worktree directory was removed.
    Reclaimed,
    /// Kept because a live run still owns the worktree.
    KeptLive,
    /// Kept because a terminal failure is still inside its retention window.
    KeptRetained,
    /// Skipped because the directory name did not map to a run id.
    SkippedUnknown,
    /// Would reclaim, but this was a dry run.
    WouldReclaim,
}

impl WorktreeGcAction {
    pub fn as_str(self) -> &'static str {
        match self {
            WorktreeGcAction::Reclaimed => "reclaimed",
            WorktreeGcAction::KeptLive => "kept_live",
            WorktreeGcAction::KeptRetained => "kept_retained",
            WorktreeGcAction::SkippedUnknown => "skipped_unknown",
            WorktreeGcAction::WouldReclaim => "would_reclaim",
        }
    }
}

/// Per-directory outcome of one GC pass.
#[derive(Debug, Clone)]
pub struct WorktreeGcEntry {
    /// Absolute path of the worktree directory.
    pub worktree: String,
    /// Run id extracted from the directory name, when one was found.
    pub run_id: Option<String>,
    /// State of the matched run record, when a record exists.
    pub run_state: Option<JobRunState>,
    /// Action taken.
    pub action: WorktreeGcAction,
    /// Human-readable justification.
    pub reason: String,
    /// True when an orphaned non-terminal run record was cancelled as part of
    /// reclaiming this worktree.
    pub cancelled_orphan: bool,
}

/// Result of one worktree GC pass.
#[derive(Debug, Clone, Default)]
pub struct WorktreeGcOutcome {
    /// Per-directory outcomes.
    pub entries: Vec<WorktreeGcEntry>,
    /// Number of worktrees reclaimed (or that would be, in a dry run).
    pub reclaimed: usize,
    /// Number of orphaned non-terminal run records cancelled.
    pub cancelled_orphans: usize,
    /// Number of directories scanned.
    pub scanned: usize,
}

/// Immutable per-pass context, computed once in [`OrbitRuntime::gc_worktrees`]
/// and threaded to each directory's reclaim decision. Bundled into one struct
/// so the per-directory helper stays under the argument-count lint.
struct GcPass {
    retention_days: i64,
    now: DateTime<Utc>,
    dry_run: bool,
    /// The caller's own worktree (canonicalized), never reclaimed.
    protected: Option<PathBuf>,
}

/// Reclaim disposition for a single worktree, computed purely from the matched
/// run record (if any) and the retention policy. Isolating this from the git /
/// filesystem side effects keeps the concurrency-safety rule
/// ("never reclaim a live run's worktree") unit-testable without a real repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorktreeDisposition {
    /// Non-terminal run with a live owner — never touch it.
    KeepLive,
    /// Terminal failure still inside the retention window.
    Retain,
    /// Terminal and reap-eligible (success/cancelled/skipped, or a failure past
    /// its retention window).
    ReclaimTerminal,
    /// Non-terminal run whose owner is conclusively gone: cancel the record,
    /// then reclaim.
    ReclaimOrphanRecord,
    /// No run record maps to this worktree — a pure orphan.
    ReclaimNoRecord,
}

/// Decide what to do with a worktree given its matched run record.
///
/// `now` and `retention_days` are injected so the retention boundary is
/// deterministic in tests. Owner liveness is probed via the shared
/// [`running_run_owner_is_stale`] / [`pending_run_stale_reason`] helpers so GC
/// classifies liveness exactly as the orphan reconciler does.
pub(super) fn classify_worktree(
    run: Option<&JobRun>,
    retention_days: i64,
    now: DateTime<Utc>,
) -> WorktreeDisposition {
    let Some(run) = run else {
        return WorktreeDisposition::ReclaimNoRecord;
    };
    if !run.state.is_terminal() {
        // Non-terminal: keep unless the owner is conclusively gone.
        let orphaned = match run.state {
            JobRunState::Pending => pending_run_stale_reason(run).is_some(),
            JobRunState::Running => running_run_owner_is_stale(run),
            // Retrying / Skipped-as-non-terminal and any other transient state:
            // treat as live and keep. These are short-lived and never own a
            // long-lived worktree that needs reaping.
            _ => false,
        };
        return if orphaned {
            WorktreeDisposition::ReclaimOrphanRecord
        } else {
            WorktreeDisposition::KeepLive
        };
    }

    // Terminal. Success/cancelled/skipped reap immediately; failures are
    // retained for the debugging/resume window.
    if is_retained_failure_state(run.state) {
        let finished = run.finished_at.unwrap_or(run.scheduled_at);
        let age = now.signed_duration_since(finished);
        if age < Duration::days(retention_days.max(0)) {
            return WorktreeDisposition::Retain;
        }
    }
    WorktreeDisposition::ReclaimTerminal
}

/// Terminal states whose worktree is retained for the failure window: a failed
/// or timed-out run is useful for debugging, and an interrupted run is
/// resumable from its checkpoints (ORB-10002).
fn is_retained_failure_state(state: JobRunState) -> bool {
    matches!(
        state,
        JobRunState::Failed | JobRunState::Timeout | JobRunState::Interrupted
    )
}

impl OrbitRuntime {
    /// Reconcile `.orbit/state/worktrees/*` against the run table and reclaim
    /// every worktree with no live run, applying the retention policy. Safe to
    /// run while other runs are in flight — a live run's worktree is never a
    /// reclaim candidate (see [`classify_worktree`]).
    pub fn gc_worktrees(
        &self,
        options: &WorktreeGcOptions,
    ) -> Result<WorktreeGcOutcome, OrbitError> {
        let pass = GcPass {
            retention_days: options
                .failed_retention_days
                .unwrap_or_else(|| self.worktree_gc_failed_retention_days()),
            now: Utc::now(),
            dry_run: options.dry_run,
            protected: current_worktree_guard(),
        };
        let repo_root = PathBuf::from(current_repo_root(self)?);

        let mut outcome = WorktreeGcOutcome::default();
        for dir in self.scan_worktree_dirs()? {
            outcome.scanned += 1;
            self.gc_one_worktree(&repo_root, &dir, &pass, &mut outcome)?;
        }
        Ok(outcome)
    }

    /// Best-effort reap of the just-finalized run's worktree. Invoked from the
    /// finalization path for terminal transitions; only reaps immediately
    /// reap-eligible states (success/cancelled/skipped) so failures ride the
    /// retention window. Never returns an error — a GC hiccup must never block
    /// a run from finalizing.
    pub(crate) fn best_effort_reap_finalized_worktree(&self, run_id: &str, state: JobRunState) {
        if !state.is_terminal() || is_retained_failure_state(state) {
            return;
        }
        if let Err(error) = self.reap_finalized_worktree(run_id) {
            tracing::warn!(
                target: "orbit.core.worktree_gc",
                run_id,
                error = %error,
                "best-effort worktree reap on finalize failed; the sweep will reclaim it later",
            );
        }
    }

    fn reap_finalized_worktree(&self, run_id: &str) -> Result<(), OrbitError> {
        let repo_root = PathBuf::from(current_repo_root(self)?);
        let protected = current_worktree_guard();
        for dir in self.scan_worktree_dirs()? {
            if extract_run_id(&dir).as_deref() != Some(run_id) {
                continue;
            }
            if is_protected(&dir, protected.as_deref()) {
                continue;
            }
            reclaim_worktree(&repo_root, &dir)?;
        }
        Ok(())
    }

    fn gc_one_worktree(
        &self,
        repo_root: &Path,
        dir: &Path,
        pass: &GcPass,
        outcome: &mut WorktreeGcOutcome,
    ) -> Result<(), OrbitError> {
        let worktree = dir.to_string_lossy().to_string();
        let Some(run_id) = extract_run_id(dir) else {
            outcome.entries.push(WorktreeGcEntry {
                worktree,
                run_id: None,
                run_state: None,
                action: WorktreeGcAction::SkippedUnknown,
                reason: "directory name does not contain a run id".to_string(),
                cancelled_orphan: false,
            });
            return Ok(());
        };

        // Never reclaim the caller's own worktree (e.g. a GC invoked from
        // inside a live run's checkout).
        if is_protected(dir, pass.protected.as_deref()) {
            outcome.entries.push(WorktreeGcEntry {
                worktree,
                run_id: Some(run_id),
                run_state: None,
                action: WorktreeGcAction::KeptLive,
                reason: "worktree is the current working directory".to_string(),
                cancelled_orphan: false,
            });
            return Ok(());
        }

        let run = self.stores().jobs().get_run(&run_id)?;
        let run_state = run.as_ref().map(|run| run.state);
        let disposition = classify_worktree(run.as_ref(), pass.retention_days, pass.now);
        let state_label = || {
            run_state
                .map(|state| state.to_string())
                .unwrap_or_else(|| "run".to_string())
        };

        let (action, reason, cancelled_orphan) = match disposition {
            WorktreeDisposition::KeepLive => (
                WorktreeGcAction::KeptLive,
                "live run owns this worktree".to_string(),
                false,
            ),
            WorktreeDisposition::Retain => (
                WorktreeGcAction::KeptRetained,
                format!(
                    "terminal {} retained for {}d debugging window",
                    state_label(),
                    pass.retention_days
                ),
                false,
            ),
            WorktreeDisposition::ReclaimNoRecord => {
                self.reclaim_or_would(repo_root, dir, pass.dry_run)?;
                (
                    reclaim_action(pass.dry_run),
                    "no run record maps to this worktree".to_string(),
                    false,
                )
            }
            WorktreeDisposition::ReclaimTerminal => {
                self.reclaim_or_would(repo_root, dir, pass.dry_run)?;
                (
                    reclaim_action(pass.dry_run),
                    format!("terminal {} reap-eligible", state_label()),
                    false,
                )
            }
            WorktreeDisposition::ReclaimOrphanRecord => {
                let cancelled = self.cancel_orphaned_run_record(&run_id, pass.dry_run);
                self.reclaim_or_would(repo_root, dir, pass.dry_run)?;
                (
                    reclaim_action(pass.dry_run),
                    "orphaned non-terminal run record (no live worker)".to_string(),
                    cancelled,
                )
            }
        };

        if matches!(
            action,
            WorktreeGcAction::Reclaimed | WorktreeGcAction::WouldReclaim
        ) {
            outcome.reclaimed += 1;
        }
        if cancelled_orphan {
            outcome.cancelled_orphans += 1;
        }
        outcome.entries.push(WorktreeGcEntry {
            worktree,
            run_id: Some(run_id),
            run_state,
            action,
            reason,
            cancelled_orphan,
        });
        Ok(())
    }

    fn reclaim_or_would(
        &self,
        repo_root: &Path,
        dir: &Path,
        dry_run: bool,
    ) -> Result<(), OrbitError> {
        if dry_run {
            return Ok(());
        }
        reclaim_worktree(repo_root, dir)
    }

    /// Cancel an orphaned non-terminal run record so its coupled tasks unblock
    /// and its reservations release. Best-effort: a cancel failure must not
    /// stop the worktree reclaim.
    fn cancel_orphaned_run_record(&self, run_id: &str, dry_run: bool) -> bool {
        if dry_run {
            return true;
        }
        match self.cancel_job_run_with_context(run_id, "system", "worktree_gc") {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(
                    target: "orbit.core.worktree_gc",
                    run_id,
                    error = %error,
                    "failed to cancel orphaned run record during worktree GC; reclaiming anyway",
                );
                false
            }
        }
    }

    /// All worktree directories to consider: the workspace-local
    /// `.orbit/state/worktrees/` plus, when `ORBIT_WORKTREE_ROOT` is set, the
    /// per-repo directory under that root (mirrors
    /// `resolve_worktree_path_from_prefix`).
    fn scan_worktree_dirs(&self) -> Result<Vec<PathBuf>, OrbitError> {
        let mut roots: BTreeSet<PathBuf> = BTreeSet::new();
        roots.insert(self.paths().worktrees_dir.clone());
        if let Ok(root) = std::env::var("ORBIT_WORKTREE_ROOT") {
            let root = root.trim();
            if !root.is_empty()
                && let Some(name) = self.paths().repo_root.file_name().and_then(|n| n.to_str())
            {
                roots.insert(PathBuf::from(root).join(name));
            }
        }

        let mut dirs = Vec::new();
        for root in roots {
            if !root.is_dir() {
                continue;
            }
            let entries = std::fs::read_dir(&root).map_err(|error| {
                OrbitError::Io(format!(
                    "failed to read worktree root '{}': {error}",
                    root.display()
                ))
            })?;
            for entry in entries {
                let entry = entry.map_err(|error| {
                    OrbitError::Io(format!("failed to read worktree entry: {error}"))
                })?;
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                }
            }
        }
        dirs.sort();
        Ok(dirs)
    }
}

/// Reclaim a single worktree directory, language-agnostically. Tries a clean
/// `git worktree remove --force` first; if git cannot resolve it as a worktree
/// (metadata drift, already-detached dir), falls back to a raw recursive
/// delete. Always prunes stale worktree metadata and best-effort deletes the
/// linked branch afterwards.
fn reclaim_worktree(repo_root: &Path, dir: &Path) -> Result<(), OrbitError> {
    let branch = detect_worktree_branch(repo_root, dir);

    let removed_via_git = git_success(
        repo_root,
        &["worktree", "remove", "--force", &dir.to_string_lossy()],
    );
    if !removed_via_git && dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|error| {
            OrbitError::Io(format!(
                "failed to remove worktree directory '{}': {error}",
                dir.display()
            ))
        })?;
    }
    // Prune the dangling administrative entry left by a raw delete (no-op after
    // a clean `worktree remove`).
    let _ = git_success(repo_root, &["worktree", "prune"]);
    if let Some(branch) = branch {
        let _ = git_success(repo_root, &["branch", "-D", &branch]);
    }
    Ok(())
}

/// Best-effort branch name for a linked worktree, read from
/// `git worktree list --porcelain` before removal.
fn detect_worktree_branch(repo_root: &Path, dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let listing = String::from_utf8(output.stdout).ok()?;
    let target = dir.to_string_lossy();
    let mut matching = false;
    for line in listing.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            matching = path == target;
            continue;
        }
        if matching && let Some(branch) = line.strip_prefix("branch refs/heads/") {
            return Some(branch.to_string());
        }
        if line.is_empty() {
            matching = false;
        }
    }
    None
}

fn git_success(repo_root: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn reclaim_action(dry_run: bool) -> WorktreeGcAction {
    if dry_run {
        WorktreeGcAction::WouldReclaim
    } else {
        WorktreeGcAction::Reclaimed
    }
}

/// The canonical repo root the run worktrees are linked to. Falls back to the
/// workspace's recorded `repo_root` when the runtime cannot resolve a live git
/// checkout.
fn current_repo_root(runtime: &OrbitRuntime) -> Result<String, OrbitError> {
    use orbit_engine::RuntimeHost;
    RuntimeHost::repo_root(runtime)
}

/// The caller's own worktree (canonicalized), so GC never reclaims the tree it
/// is running inside.
fn current_worktree_guard() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    Some(cwd.canonicalize().unwrap_or(cwd))
}

pub(super) fn is_protected(dir: &Path, protected: Option<&Path>) -> bool {
    let Some(protected) = protected else {
        return false;
    };
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    protected.starts_with(&dir)
}

/// Extract the `jrun-…` run id embedded in a worktree directory name. Worktrees
/// are named `<prefix>-<sanitized_run_id>` (e.g. `orbit-jrun-20260712-2021-4`,
/// `parallel-batch-jrun-…`); run ids are always `jrun-`-prefixed and contain
/// only characters the worktree-token sanitizer preserves, so the id is the
/// substring from the first `jrun-`. Directories with no `jrun-` token are
/// left untouched.
pub(super) fn extract_run_id(dir: &Path) -> Option<String> {
    let name = dir.file_name()?.to_str()?;
    let idx = name.find("jrun-")?;
    Some(name[idx..].to_string())
}
