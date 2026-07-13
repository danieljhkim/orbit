//! GC collector for Orbit-owned diagnostics telemetry streams [ORB-10185].
//!
//! `orbit gc diagnostics` bounds the append-only JSONL partitions written under
//! `<root>/state/diagnostics/{metrics,friction}`. Both streams are day
//! partitioned (`<category>/YYYY-MM/DD.jsonl`); the writer only ever appends to
//! the partition for the current UTC day. This collector therefore treats each
//! `DD.jsonl` file as a partition and reclaims only *closed* partitions — those
//! whose calendar day is strictly in the past — once they age past the
//! category-specific retention window. The current-day partition (and any file
//! whose date is today or in the future) is never a candidate, so a live writer
//! holding it open is unaffected without any cross-process locking.
//!
//! Explicitly out of scope, so this stream GC can never be confused with
//! durable knowledge:
//! - Canonical friction records under `<root>/frictions` (a sibling of, not
//!   under, `state/diagnostics/friction`).
//! - Tasks, ADRs, learnings, and audit evidence — none live under
//!   `state/diagnostics`, so scoping the scan to the two telemetry directories
//!   excludes them by construction.
//!
//! Malformed or ambiguously named entries (a month directory that is not
//! `YYYY-MM`, a partition file that is not `DD.jsonl`, a stray file or nested
//! directory) are diagnostic evidence in their own right: they are reported as
//! skips and always retained.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{Datelike, NaiveDate};
use orbit_common::types::OrbitError;
use serde::{Deserialize, Serialize};

use super::gc::{
    GcCandidate, GcCollector, GcContext, GcMutation, GcPlan, GcRevalidation, GcScope, GcSkip,
    GcTarget,
};

const SECONDS_PER_DAY: u64 = 86_400;

/// A diagnostics telemetry stream. Each maps to one directory under
/// `state/diagnostics` and carries its own retention window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiagnosticsCategory {
    Metrics,
    Friction,
}

impl DiagnosticsCategory {
    const ALL: [DiagnosticsCategory; 2] = [Self::Metrics, Self::Friction];

    fn dir(self) -> &'static str {
        match self {
            Self::Metrics => "metrics",
            Self::Friction => "friction",
        }
    }
}

/// Per-category retention windows (in days) for closed diagnostics partitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticsGcPolicy {
    pub metrics_retention_days: u64,
    pub friction_retention_days: u64,
}

impl DiagnosticsGcPolicy {
    fn retention_days(&self, category: DiagnosticsCategory) -> u64 {
        match category {
            DiagnosticsCategory::Metrics => self.metrics_retention_days,
            DiagnosticsCategory::Friction => self.friction_retention_days,
        }
    }
}

/// Collector for the append-only metrics/friction diagnostics streams.
pub struct DiagnosticsGcCollector {
    /// `<scope root>/state/diagnostics`.
    diagnostics_root: PathBuf,
    policy: DiagnosticsGcPolicy,
    /// Uniform `--retention` override (pre-parsed by the CLI). When set it
    /// replaces the configured per-category window for *both* streams, floored
    /// to whole days.
    retention_override: Option<Duration>,
}

impl DiagnosticsGcCollector {
    /// Manage the diagnostics streams under `scope_root/state/diagnostics`.
    pub fn from_scope(
        scope: &GcScope,
        policy: DiagnosticsGcPolicy,
        retention_override: Option<Duration>,
    ) -> Self {
        Self::new(scope.root(), policy, retention_override)
    }

    /// Explicit constructor (test seam): manage the streams under
    /// `scope_root/state/diagnostics`.
    pub fn new(
        scope_root: &Path,
        policy: DiagnosticsGcPolicy,
        retention_override: Option<Duration>,
    ) -> Self {
        Self {
            diagnostics_root: scope_root.join("state").join("diagnostics"),
            policy,
            retention_override,
        }
    }

    fn retention_days(&self, category: DiagnosticsCategory) -> u64 {
        match self.retention_override {
            Some(window) => window.as_secs() / SECONDS_PER_DAY,
            None => self.policy.retention_days(category),
        }
    }

    /// Classify one candidate day-partition against `today`. Returns a candidate
    /// when the partition is closed and aged past retention; otherwise a skip
    /// describing why it is retained (open/current-day or within retention).
    fn classify_partition(
        &self,
        category: DiagnosticsCategory,
        partition_date: NaiveDate,
        path: &Path,
        today: NaiveDate,
    ) -> Result<Classified, OrbitError> {
        let id = self.partition_id(path);
        // Partition-closure rule: only strictly-past calendar days are closed.
        // The current-day writer (and any future-dated file) is protected here
        // without file locking.
        if partition_date >= today {
            return Ok(Classified::Skip(GcSkip {
                id,
                code: "open_partition".to_string(),
                reason: format!(
                    "partition {partition_date} is the current-day (or future) writer target"
                ),
            }));
        }
        let age_days = (today - partition_date).num_days().max(0) as u64;
        let retention_days = self.retention_days(category);
        if age_days <= retention_days {
            return Ok(Classified::Skip(GcSkip {
                id,
                code: "retained".to_string(),
                reason: format!(
                    "closed {} partition {partition_date} is {age_days}d old; within {retention_days}d retention",
                    category.dir()
                ),
            }));
        }
        let bytes = fs::symlink_metadata(path).map(|meta| meta.len()).ok();
        let expected = ExpectedPartition {
            category: category.dir().to_string(),
            partition_date,
            bytes,
        };
        Ok(Classified::Candidate(GcCandidate {
            id,
            action: "delete".to_string(),
            path: Some(path.to_path_buf()),
            bytes,
            ownership_evidence: format!(
                "append-only diagnostics {} day-partition under state/diagnostics/{}; canonical .orbit/frictions, tasks, and audit evidence excluded",
                category.dir(),
                category.dir()
            ),
            retention_evidence: format!(
                "closed {} partition dated {partition_date}; age {age_days}d exceeds {retention_days}d retention",
                category.dir()
            ),
            expected_state: serde_json::to_string(&expected)
                .map_err(|error| OrbitError::Execution(error.to_string()))?,
            allow_owned_symlink: false,
        }))
    }

    /// Stable `<category>/<month>/<DD>.jsonl` identifier for a partition path
    /// (the path relative to the diagnostics root already leads with the
    /// category directory).
    fn partition_id(&self, path: &Path) -> String {
        path.strip_prefix(&self.diagnostics_root)
            .map(|relative| relative.display().to_string())
            .unwrap_or_else(|_| path.display().to_string())
    }

    fn scan_category(
        &self,
        category: DiagnosticsCategory,
        today: NaiveDate,
        plan: &mut GcPlan,
    ) -> Result<(), OrbitError> {
        let category_root = self.diagnostics_root.join(category.dir());
        if !category_root.exists() {
            return Ok(());
        }
        let mut month_dirs = Vec::new();
        for entry in fs::read_dir(&category_root)? {
            let path = entry?.path();
            let name = file_name(&path);
            if path.is_dir() {
                if parse_year_month(&name).is_some() {
                    month_dirs.push((name, path));
                } else {
                    plan.scanned = plan.scanned.saturating_add(1);
                    plan.skipped.push(GcSkip {
                        id: format!("{}/{name}", category.dir()),
                        code: "malformed_partition".to_string(),
                        reason: "month directory name is not YYYY-MM; retained as evidence"
                            .to_string(),
                    });
                }
            } else {
                // A stray file directly under the category root is ambiguous:
                // partitions always live inside a month directory.
                plan.scanned = plan.scanned.saturating_add(1);
                plan.skipped.push(GcSkip {
                    id: format!("{}/{name}", category.dir()),
                    code: "malformed_partition".to_string(),
                    reason:
                        "unexpected file outside a YYYY-MM month directory; retained as evidence"
                            .to_string(),
                });
            }
        }
        // Deterministic order so plan/report output is stable across runs.
        month_dirs.sort_by(|left, right| left.0.cmp(&right.0));
        for (month_name, month_dir) in month_dirs {
            self.scan_month(category, &month_name, &month_dir, today, plan)?;
        }
        Ok(())
    }

    fn scan_month(
        &self,
        category: DiagnosticsCategory,
        month_name: &str,
        month_dir: &Path,
        today: NaiveDate,
        plan: &mut GcPlan,
    ) -> Result<(), OrbitError> {
        let mut files = Vec::new();
        for entry in fs::read_dir(month_dir)? {
            files.push(entry?.path());
        }
        files.sort();
        for path in files {
            let name = file_name(&path);
            plan.scanned = plan.scanned.saturating_add(1);
            if path.is_dir() {
                plan.skipped.push(GcSkip {
                    id: format!("{}/{month_name}/{name}", category.dir()),
                    code: "malformed_partition".to_string(),
                    reason: "nested directory where a DD.jsonl partition was expected; retained as evidence"
                        .to_string(),
                });
                continue;
            }
            let Some(partition_date) = parse_partition_date(month_name, &name) else {
                plan.skipped.push(GcSkip {
                    id: format!("{}/{month_name}/{name}", category.dir()),
                    code: "malformed_partition".to_string(),
                    reason: "file name is not a valid DD.jsonl day partition; retained as evidence"
                        .to_string(),
                });
                continue;
            };
            if let Ok(meta) = fs::symlink_metadata(&path) {
                plan.scanned_bytes = plan.scanned_bytes.map(|sum| sum.saturating_add(meta.len()));
            }
            match self.classify_partition(category, partition_date, &path, today)? {
                Classified::Candidate(candidate) => plan.candidates.push(candidate),
                Classified::Skip(skip) => plan.skipped.push(skip),
            }
        }
        Ok(())
    }

    fn revalidate_partition(
        &self,
        candidate: &GcCandidate,
        context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError> {
        let expected: ExpectedPartition =
            serde_json::from_str(&candidate.expected_state).map_err(|error| {
                OrbitError::Execution(format!("invalid frozen diagnostics partition: {error}"))
            })?;
        let Some(path) = candidate.path.as_deref() else {
            return Ok(GcRevalidation::Skip {
                code: "missing_path".to_string(),
                reason: "diagnostics GC candidate has no path".to_string(),
            });
        };
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(GcRevalidation::Skip {
                    code: "state_changed".to_string(),
                    reason: "partition disappeared before apply".to_string(),
                });
            }
            Err(error) => return Err(OrbitError::from(error)),
        };
        if !meta.file_type().is_file() {
            return Ok(GcRevalidation::Skip {
                code: "not_a_file".to_string(),
                reason: "candidate is no longer a regular file".to_string(),
            });
        }
        // A size change means a writer touched the partition after planning —
        // fail closed rather than deleting freshly appended telemetry.
        if expected.bytes.is_some_and(|frozen| frozen != meta.len()) {
            return Ok(GcRevalidation::Skip {
                code: "state_changed".to_string(),
                reason: "partition size changed after planning".to_string(),
            });
        }
        let today = context.clock.now().date_naive();
        if expected.partition_date >= today {
            return Ok(GcRevalidation::Skip {
                code: "open_partition".to_string(),
                reason: "partition is no longer closed relative to the current day".to_string(),
            });
        }
        Ok(GcRevalidation::Ready)
    }
}

impl GcCollector for DiagnosticsGcCollector {
    fn target(&self) -> GcTarget {
        GcTarget::Diagnostics
    }

    fn plan(&self, context: &GcContext<'_>) -> Result<GcPlan, OrbitError> {
        let today = context.clock.now().date_naive();
        let mut plan = GcPlan::empty(GcTarget::Diagnostics);
        plan.config_source = "gc.diagnostics".to_string();
        for category in DiagnosticsCategory::ALL {
            self.scan_category(category, today, &mut plan)?;
        }
        Ok(plan)
    }

    fn revalidate(
        &self,
        candidate: &GcCandidate,
        context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError> {
        self.revalidate_partition(candidate, context)
    }

    fn apply(
        &self,
        candidate: &GcCandidate,
        _context: &GcContext<'_>,
    ) -> Result<GcMutation, OrbitError> {
        let path = candidate.path.as_ref().ok_or_else(|| {
            OrbitError::Execution("diagnostics GC candidate has no path".to_string())
        })?;
        let bytes = fs::symlink_metadata(path)
            .map(|meta| meta.len())
            .ok()
            .or(candidate.bytes);
        fs::remove_file(path)?;
        Ok(GcMutation {
            reclaimed_bytes: bytes,
        })
    }
}

#[derive(Debug)]
enum Classified {
    Candidate(GcCandidate),
    Skip(GcSkip),
}

/// Frozen partition identity carried from plan to apply so revalidation can
/// detect drift (a writer touching the file, or the day rolling over).
#[derive(Debug, Serialize, Deserialize)]
struct ExpectedPartition {
    category: String,
    partition_date: NaiveDate,
    bytes: Option<u64>,
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| path.display().to_string(), ToString::to_string)
}

/// Strict `YYYY-MM` validator. Returns the first-of-month date only when the
/// name is exactly a zero-padded year-month, so ambiguously named directories
/// are reported rather than swept.
fn parse_year_month(name: &str) -> Option<NaiveDate> {
    let bytes = name.as_bytes();
    let well_formed = bytes.len() == 7
        && bytes[4] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..].iter().all(u8::is_ascii_digit);
    if !well_formed {
        return None;
    }
    NaiveDate::parse_from_str(&format!("{name}-01"), "%Y-%m-%d").ok()
}

/// Strict `DD.jsonl` validator composed against its month directory. Returns the
/// full partition date only for a zero-padded two-digit day that forms a real
/// calendar date (so `32.jsonl` or `30.jsonl` in February are rejected).
fn parse_partition_date(month_name: &str, file_name: &str) -> Option<NaiveDate> {
    let stem = file_name.strip_suffix(".jsonl")?;
    if stem.len() != 2 || !stem.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let month = parse_year_month(month_name)?;
    NaiveDate::parse_from_str(
        &format!("{:04}-{:02}-{stem}", month.year(), month.month()),
        "%Y-%m-%d",
    )
    .ok()
}
