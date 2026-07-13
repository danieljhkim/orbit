//! [ORB-10185] Tests for the diagnostics telemetry stream GC collector.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use tempfile::TempDir;

use crate::command::gc::{
    GcClock, GcItemStatus, GcMode, GcOutcome, GcReport, GcRequest, GcScope, execute_gc,
};
use crate::command::gc_diagnostics::{DiagnosticsGcCollector, DiagnosticsGcPolicy};

struct FakeClock(DateTime<Utc>);

impl GcClock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

/// `today` for every fixture below is 2026-07-12 (UTC). Partition ages are
/// derived from the file's calendar name, not its mtime, so tests never touch
/// filesystem timestamps.
fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0)
        .single()
        .expect("fixed time")
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    state: PathBuf,
    clock: FakeClock,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("temp");
        // Canonicalize so candidate containment (which canonicalizes the owned
        // root) holds even when TMPDIR is a symlink (macOS /var -> /private/var).
        let base = temp.path().canonicalize().expect("canonical temp");
        let root = base.join("workspace");
        let state = root.join("state");
        fs::create_dir_all(&state).expect("state dir");
        Self {
            _temp: temp,
            root,
            state,
            clock: FakeClock(fixed_now()),
        }
    }

    fn write_partition(&self, category: &str, month: &str, day: &str, contents: &str) -> PathBuf {
        write_partition_in(&self.root, category, month, day, contents)
    }

    fn collector(&self, policy: DiagnosticsGcPolicy) -> DiagnosticsGcCollector {
        DiagnosticsGcCollector::new(&self.root, policy, None)
    }

    fn request(&self, apply: bool) -> GcRequest<'_> {
        GcRequest {
            apply,
            scope: GcScope::Workspace {
                workspace_id: None,
                root: self.root.clone(),
            },
            retention_override: None,
            global_state_dir: &self.state,
            clock: &self.clock,
        }
    }
}

fn write_partition_in(
    root: &Path,
    category: &str,
    month: &str,
    day: &str,
    contents: &str,
) -> PathBuf {
    let dir = root
        .join("state")
        .join("diagnostics")
        .join(category)
        .join(month);
    fs::create_dir_all(&dir).expect("partition dir");
    let path = dir.join(format!("{day}.jsonl"));
    fs::write(&path, contents).expect("partition file");
    path
}

fn policy(metrics: u64, friction: u64) -> DiagnosticsGcPolicy {
    DiagnosticsGcPolicy {
        metrics_retention_days: metrics,
        friction_retention_days: friction,
    }
}

fn eligible_ids(report: &GcReport) -> BTreeSet<String> {
    report.targets[0]
        .items
        .iter()
        .filter(|item| item.status == GcItemStatus::Eligible)
        .map(|item| item.id.clone())
        .collect()
}

fn reclaimed_ids(report: &GcReport) -> BTreeSet<String> {
    report.targets[0]
        .items
        .iter()
        .filter(|item| item.status == GcItemStatus::Reclaimed)
        .map(|item| item.id.clone())
        .collect()
}

fn skip_codes(report: &GcReport) -> BTreeSet<String> {
    report.targets[0]
        .skipped
        .iter()
        .map(|skip| skip.code.clone())
        .collect()
}

#[test]
fn plan_lists_closed_old_partitions_without_deleting_and_apply_reclaims_them() {
    let fixture = Fixture::new();
    // 2026-04-01 is 102 days before 2026-07-12 → past the 90-day window.
    let old = fixture.write_partition("metrics", "2026-04", "01", "{\"ts\":\"x\"}\n");
    // 2026-06-20 is only 22 days old → retained.
    let recent = fixture.write_partition("metrics", "2026-06", "20", "{\"ts\":\"y\"}\n");
    let collector = fixture.collector(policy(90, 90));

    let plan = execute_gc(&collector, fixture.request(false)).expect("plan");
    assert_eq!(plan.mode, GcMode::Plan);
    assert_eq!(
        eligible_ids(&plan),
        BTreeSet::from(["metrics/2026-04/01.jsonl".to_string()])
    );
    assert_eq!(plan.targets[0].counts.reclaimed, 0);
    assert!(old.exists(), "dry-run must not delete anything");
    assert_eq!(plan.targets[0].items[0].action, "delete");

    let apply = execute_gc(&collector, fixture.request(true)).expect("apply");
    assert_eq!(apply.mode, GcMode::Apply);
    assert_eq!(apply.outcome, GcOutcome::Clean);
    assert_eq!(
        reclaimed_ids(&apply),
        BTreeSet::from(["metrics/2026-04/01.jsonl".to_string()])
    );
    assert!(!old.exists(), "closed partition past retention is deleted");
    assert!(recent.exists(), "within-retention partition is kept");
}

#[test]
fn current_day_partition_is_protected_from_the_active_writer() {
    let fixture = Fixture::new();
    // The writer appends to today's partition; even with a zero-day window it
    // must never be eligible.
    let today = fixture.write_partition("metrics", "2026-07", "12", "{\"ts\":\"live\"}\n");
    let collector = fixture.collector(policy(0, 0));

    let apply = execute_gc(&collector, fixture.request(true)).expect("apply");
    assert_eq!(apply.outcome, GcOutcome::Clean);
    assert!(eligible_ids(&apply).is_empty());
    assert!(reclaimed_ids(&apply).is_empty());
    assert!(today.exists(), "current-day partition survives");
    assert!(
        skip_codes(&apply).contains("open_partition"),
        "current-day partition is reported as an open partition skip"
    );
}

#[test]
fn category_retention_overrides_are_independent() {
    let fixture = Fixture::new();
    // 2026-05-01 is 72 days before today: past friction's 30-day window but
    // within metrics' 90-day window.
    let metrics = fixture.write_partition("metrics", "2026-05", "01", "{\"ts\":\"m\"}\n");
    let friction = fixture.write_partition("friction", "2026-05", "01", "{\"ts\":\"f\"}\n");
    let collector = fixture.collector(policy(90, 30));

    let apply = execute_gc(&collector, fixture.request(true)).expect("apply");
    assert_eq!(apply.outcome, GcOutcome::Clean);
    assert_eq!(
        reclaimed_ids(&apply),
        BTreeSet::from(["friction/2026-05/01.jsonl".to_string()]),
        "only the diagnostic-friction partition ages out under its shorter window"
    );
    assert!(metrics.exists(), "metrics partition retained under 90d");
    assert!(!friction.exists(), "friction partition reclaimed under 30d");
}

#[test]
fn malformed_and_ambiguous_entries_are_reported_and_retained() {
    let fixture = Fixture::new();
    let diagnostics = fixture.root.join("state").join("diagnostics");
    // Ambiguous month directory name.
    let bad_month = diagnostics.join("metrics").join("2026-13");
    fs::create_dir_all(&bad_month).expect("bad month");
    let bad_month_file = bad_month.join("01.jsonl");
    fs::write(&bad_month_file, "x").expect("bad month file");
    // Stray file directly under the category root.
    let stray = diagnostics.join("metrics").join("orphan.jsonl");
    fs::create_dir_all(diagnostics.join("metrics")).expect("metrics dir");
    fs::write(&stray, "x").expect("stray file");
    // Non day-partition file name inside a valid month.
    let bad_day = fixture.write_partition("metrics", "2026-04", "99", "x");
    // A genuinely eligible partition still gets collected alongside the noise.
    let good = fixture.write_partition("metrics", "2026-04", "01", "x");
    let collector = fixture.collector(policy(90, 90));

    let apply = execute_gc(&collector, fixture.request(true)).expect("apply");
    // Malformed entries do not fail the run (they are evidence, not errors).
    assert!(apply.targets[0].errors.is_empty());
    assert!(
        skip_codes(&apply).contains("malformed_partition"),
        "malformed entries are reported as skips"
    );
    assert!(bad_month_file.exists(), "malformed month dir file retained");
    assert!(stray.exists(), "stray file retained");
    assert!(bad_day.exists(), "non-DD.jsonl file retained");
    assert!(
        !good.exists(),
        "the valid eligible partition is still reclaimed"
    );
}

#[test]
fn apply_is_idempotent() {
    let fixture = Fixture::new();
    fixture.write_partition("metrics", "2026-04", "01", "x");
    let collector = fixture.collector(policy(90, 90));

    let first = execute_gc(&collector, fixture.request(true)).expect("first apply");
    assert_eq!(first.targets[0].counts.reclaimed, 1);

    let second = execute_gc(&collector, fixture.request(true)).expect("second apply");
    assert_eq!(second.outcome, GcOutcome::Clean);
    assert_eq!(second.targets[0].counts.reclaimed, 0);
    assert!(eligible_ids(&second).is_empty());
}

#[test]
fn collection_is_isolated_to_its_own_scope_root() {
    let fixture = Fixture::new();
    fixture.write_partition("metrics", "2026-04", "01", "x");
    // A second, independent workspace root with an equally-aged partition.
    let other_root = fixture.root.parent().expect("base").join("other-workspace");
    fs::create_dir_all(other_root.join("state")).expect("other state");
    let other = write_partition_in(&other_root, "metrics", "2026-04", "01", "x");
    let collector = fixture.collector(policy(90, 90));

    let apply = execute_gc(&collector, fixture.request(true)).expect("apply");
    assert_eq!(apply.targets[0].counts.reclaimed, 1);
    assert!(
        other.exists(),
        "a partition under a different scope root is never touched"
    );
}

#[test]
fn reader_stays_correct_after_older_partitions_are_removed() {
    let fixture = Fixture::new();
    fixture.write_partition("metrics", "2026-04", "01", "{\"ts\":\"old\"}\n");
    let kept = fixture.write_partition("metrics", "2026-06", "20", "{\"ts\":\"kept\"}\n");
    let collector = fixture.collector(policy(90, 90));

    execute_gc(&collector, fixture.request(true)).expect("apply");

    // The retained partition is untouched and its month directory still reads
    // back cleanly; the reclaimed month simply has no files left.
    assert!(kept.exists());
    assert_eq!(
        fs::read_to_string(&kept).expect("kept readable"),
        "{\"ts\":\"kept\"}\n"
    );
    let removed_month = fixture
        .root
        .join("state")
        .join("diagnostics")
        .join("metrics")
        .join("2026-04");
    let remaining: Vec<_> = fs::read_dir(&removed_month)
        .map(|dir| dir.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    assert!(
        remaining.is_empty(),
        "reclaimed month directory has no partition files left"
    );
}
