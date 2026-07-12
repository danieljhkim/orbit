//! GC collector for Orbit-owned operational log archives [ORB-10184].
//!
//! `orbit gc logs` plans and applies the same age + total-size retention
//! policy that the tracing subscriber applies opportunistically at startup.
//! Both delegate to [`log_rotation::plan_prune`] — the single classifier — so
//! the CLI surface and subscriber-init pruning never disagree.
//!
//! Owned surfaces: the global JSONL tracing feed
//! (`<root>/state/logs/orbit.jsonl`, overridable via `ORBIT_LOG_PATH`) and the
//! macOS sweep log (`<root>/logs/sweep.log`). Linux journald and third-party
//! logs are explicitly out of scope. The active file is never a candidate:
//! only dated `<active>.<stamp>` archives are collected, so a writer holding
//! the active inode open is unaffected.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use orbit_common::types::OrbitError;
use orbit_common::utility::log_rotation::{
    LogRotationConfig, PruneCandidate, PruneReason, plan_prune,
};

use super::gc::{
    GcCandidate, GcCollector, GcContext, GcItemError, GcMutation, GcPlan, GcRevalidation, GcScope,
    GcSkip, GcTarget,
};

/// Environment override for the active JSONL tracing feed path. Producers and
/// readers agree on it; the collector honors it as an owned log location.
const ORBIT_LOG_PATH_ENV: &str = "ORBIT_LOG_PATH";

/// Collector for Orbit-owned operational log archives. Reuses the
/// `log_rotation` retention policy rather than duplicating it.
pub struct LogsGcCollector {
    /// Active log files whose dated archives are managed. Never deleted.
    active_paths: Vec<PathBuf>,
    /// Age + total-size budgets (loaded from the global config, best-effort).
    config: LogRotationConfig,
    /// Per-invocation age-window override (`--retention`), pre-parsed by the
    /// CLI. When set it replaces the configured age budget.
    retention_override: Option<Duration>,
}

impl LogsGcCollector {
    /// Resolve owned log locations from the GC scope root, honoring
    /// `ORBIT_LOG_PATH` for the JSONL feed, and load the rotation policy from
    /// the global config (best-effort — the same source subscriber-init uses).
    pub fn from_scope(scope: &GcScope, retention_override: Option<Duration>) -> Self {
        Self::with_config(
            resolve_active_paths(scope.root()),
            LogRotationConfig::load_global_best_effort(),
            retention_override,
        )
    }

    /// Explicit constructor (test seam): manage exactly `active_paths` under
    /// `config`.
    pub fn with_config(
        active_paths: Vec<PathBuf>,
        config: LogRotationConfig,
        retention_override: Option<Duration>,
    ) -> Self {
        Self {
            active_paths,
            config,
            retention_override,
        }
    }

    fn retention_window(&self) -> Duration {
        self.retention_override
            .unwrap_or_else(|| self.config.retention_window())
    }

    fn is_active(&self, path: &Path) -> bool {
        self.active_paths.iter().any(|active| active == path)
    }
}

/// The Orbit-owned active log files under a global state `root`: the JSONL
/// tracing feed (or `ORBIT_LOG_PATH`) and the sweep log.
fn resolve_active_paths(root: &Path) -> Vec<PathBuf> {
    let jsonl = orbit_log_path_override()
        .unwrap_or_else(|| root.join("state").join("logs").join("orbit.jsonl"));
    let sweep = root.join("logs").join("sweep.log");
    let mut paths = vec![jsonl];
    if !paths.contains(&sweep) {
        paths.push(sweep);
    }
    paths
}

fn orbit_log_path_override() -> Option<PathBuf> {
    std::env::var_os(ORBIT_LOG_PATH_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn active_label(active_path: &Path) -> String {
    active_path
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| active_path.display().to_string(), ToString::to_string)
}

impl LogsGcCollector {
    fn to_candidate(&self, active_label: &str, prune: PruneCandidate) -> GcCandidate {
        let id = prune
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .map_or_else(|| prune.path.display().to_string(), ToString::to_string);
        // Encode the selection reason in `action` so plan/apply reports
        // distinguish age-pruned from size-pruned files without a schema change.
        let (action, retention_evidence) = match prune.reason {
            PruneReason::Age => (
                "delete-age",
                format!(
                    "archive mtime older than the {}s retention window",
                    self.retention_window().as_secs()
                ),
            ),
            PruneReason::Size => (
                "delete-size",
                format!(
                    "oldest archive beyond the {}-byte total-size budget",
                    self.config.max_total_bytes
                ),
            ),
        };
        GcCandidate {
            id,
            action: action.to_string(),
            path: Some(prune.path),
            bytes: Some(prune.bytes),
            ownership_evidence: format!("dated archive of Orbit-owned active log `{active_label}`"),
            retention_evidence,
            expected_state: "present".to_string(),
            allow_owned_symlink: false,
        }
    }
}

impl GcCollector for LogsGcCollector {
    fn target(&self) -> GcTarget {
        GcTarget::Logs
    }

    fn plan(&self, context: &GcContext<'_>) -> Result<GcPlan, OrbitError> {
        let now = SystemTime::from(context.clock.now());
        let retention = self.retention_window();
        let root = context.scope.root();

        let mut scanned: u64 = 0;
        let mut scanned_bytes: u64 = 0;
        let mut candidates = Vec::new();
        let mut skipped = Vec::new();
        let mut errors = Vec::new();

        for active in &self.active_paths {
            let label = active_label(active);
            // Mutation is gated on containment within the owned scope root
            // (ADR-0220). Skip — never force-delete — a custom log path that
            // resolves outside it, keeping plan/apply parity (L-0080).
            if !active.starts_with(root) {
                skipped.push(GcSkip {
                    id: label,
                    code: "out_of_scope".to_string(),
                    reason: format!(
                        "active log `{}` is outside the GC scope root `{}`",
                        active.display(),
                        root.display()
                    ),
                });
                continue;
            }
            match plan_prune(active, retention, self.config.max_total_bytes, now) {
                Ok(prune) => {
                    scanned = scanned.saturating_add(prune.scanned);
                    scanned_bytes = scanned_bytes.saturating_add(prune.scanned_bytes);
                    for candidate in prune.candidates {
                        candidates.push(self.to_candidate(&label, candidate));
                    }
                }
                // An absent log directory just means nothing to collect yet.
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => errors.push(GcItemError {
                    id: label,
                    phase: "plan".to_string(),
                    code: "scan_failed".to_string(),
                    message: error.to_string(),
                }),
            }
        }

        Ok(GcPlan {
            target: GcTarget::Logs,
            config_source: "runtime.log_rotation".to_string(),
            scanned,
            scanned_bytes: Some(scanned_bytes),
            candidates,
            skipped,
            errors,
        })
    }

    fn revalidate(
        &self,
        candidate: &GcCandidate,
        _context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError> {
        let Some(path) = &candidate.path else {
            return Ok(GcRevalidation::Skip {
                code: "missing_path".to_string(),
                reason: "log GC candidate has no path".to_string(),
            });
        };
        // Defense in depth: the classifier never emits the active file, but
        // refuse it here too so a stale plan can never delete a live log.
        if self.is_active(path) {
            return Ok(GcRevalidation::Skip {
                code: "active_log".to_string(),
                reason: "refusing to delete the active log file".to_string(),
            });
        }
        match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_file() => Ok(GcRevalidation::Ready),
            Ok(_) => Ok(GcRevalidation::Skip {
                code: "not_a_file".to_string(),
                reason: "candidate is no longer a regular file".to_string(),
            }),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(GcRevalidation::Skip {
                code: "state_changed".to_string(),
                reason: "archive disappeared before apply".to_string(),
            }),
            Err(error) => Err(OrbitError::from(error)),
        }
    }

    fn apply(
        &self,
        candidate: &GcCandidate,
        _context: &GcContext<'_>,
    ) -> Result<GcMutation, OrbitError> {
        let path = candidate
            .path
            .as_ref()
            .ok_or_else(|| OrbitError::Execution("log GC candidate has no path".to_string()))?;
        if self.is_active(path) {
            return Err(OrbitError::PolicyDenied(format!(
                "refusing to delete active log file `{}`",
                path.display()
            )));
        }
        let bytes = std::fs::symlink_metadata(path)
            .map(|meta| meta.len())
            .ok()
            .or(candidate.bytes);
        std::fs::remove_file(path)?;
        Ok(GcMutation {
            reclaimed_bytes: bytes,
        })
    }
}
