//! [ORB-10184] Tests for the Orbit-owned operational log GC collector.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, TimeZone, Utc};
use orbit_common::utility::log_rotation::LogRotationConfig;
use tempfile::TempDir;

use crate::command::gc::{
    GcClock, GcItemStatus, GcMode, GcOutcome, GcRequest, GcScope, execute_gc,
};
use crate::command::gc_logs::LogsGcCollector;

struct FakeClock(DateTime<Utc>);

impl GcClock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: OsString) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

fn fixed_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0)
        .single()
        .expect("fixed time")
}

fn now_sys() -> SystemTime {
    SystemTime::from(fixed_now())
}

fn config(retention_days: u64, max_total_bytes: u64) -> LogRotationConfig {
    LogRotationConfig {
        retention_days,
        max_total_bytes,
        max_file_bytes: 10_000_000,
    }
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    logs_dir: PathBuf,
    active: PathBuf,
    state: PathBuf,
    clock: FakeClock,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("temp");
        // Canonicalize so candidate containment (which canonicalizes the root)
        // holds even when TMPDIR is a symlink (e.g. macOS /var -> /private/var).
        let base = temp.path().canonicalize().expect("canonical temp");
        let root = base.join("global");
        let logs_dir = root.join("state").join("logs");
        fs::create_dir_all(&logs_dir).expect("logs dir");
        let active = logs_dir.join("orbit.jsonl");
        fs::write(&active, "active\n").expect("active file");
        let state = root.join("state");
        Self {
            _temp: temp,
            root,
            logs_dir,
            active,
            state,
            clock: FakeClock(fixed_now()),
        }
    }

    fn write_archive(&self, name: &str, contents: &str, age: Duration) -> PathBuf {
        write_archive_in(&self.logs_dir, name, contents, age)
    }

    fn collector(&self, config: LogRotationConfig, retention: Option<Duration>) -> LogsGcCollector {
        LogsGcCollector::with_config(vec![self.active.clone()], config, retention)
    }

    fn request(&self, apply: bool) -> GcRequest<'_> {
        GcRequest {
            apply,
            scope: GcScope::Global {
                root: self.root.clone(),
            },
            retention_override: None,
            global_state_dir: &self.state,
            clock: &self.clock,
        }
    }
}

fn write_archive_in(dir: &std::path::Path, name: &str, contents: &str, age: Duration) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, contents).expect("archive");
    let when = now_sys().checked_sub(age).expect("mtime in range");
    fs::File::options()
        .write(true)
        .open(&path)
        .expect("open for mtime")
        .set_modified(when)
        .expect("set mtime");
    path
}

fn eligible_ids(report: &crate::command::gc::GcReport) -> BTreeSet<String> {
    report.targets[0]
        .items
        .iter()
        .filter(|item| item.status == GcItemStatus::Eligible)
        .map(|item| item.id.clone())
        .collect()
}

fn reclaimed_ids(report: &crate::command::gc::GcReport) -> BTreeSet<String> {
    report.targets[0]
        .items
        .iter()
        .filter(|item| item.status == GcItemStatus::Reclaimed)
        .map(|item| item.id.clone())
        .collect()
}

#[test]
fn plan_is_noop_and_apply_prunes_only_the_age_budget() {
    let fixture = Fixture::new();
    let old = fixture.write_archive(
        "orbit.jsonl.OLD",
        "old-data",
        Duration::from_secs(10 * 86_400),
    );
    let recent = fixture.write_archive("orbit.jsonl.RECENT", "recent", Duration::from_secs(3_600));
    let collector = fixture.collector(config(7, 10_000_000), None);

    let plan = execute_gc(&collector, fixture.request(false)).expect("plan");
    assert_eq!(plan.mode, GcMode::Plan);
    assert_eq!(
        eligible_ids(&plan),
        BTreeSet::from(["orbit.jsonl.OLD".to_string()])
    );
    assert_eq!(plan.targets[0].counts.reclaimed, 0);
    assert!(old.exists(), "plan must not delete anything");
    // Age selection is surfaced distinctly in the report.
    assert_eq!(plan.targets[0].items[0].action, "delete-age");

    let apply = execute_gc(&collector, fixture.request(true)).expect("apply");
    assert_eq!(apply.outcome, GcOutcome::Clean);
    assert_eq!(apply.targets[0].counts.reclaimed, 1);
    assert!(
        !old.exists(),
        "archive older than the age budget is deleted"
    );
    assert!(recent.exists(), "recent archive is kept");
    assert!(fixture.active.exists(), "active file is never touched");
    assert_eq!(
        fs::read_to_string(&fixture.active).expect("active readable"),
        "active\n"
    );
}

#[test]
fn apply_prunes_oldest_first_beyond_the_size_budget() {
    let fixture = Fixture::new();
    let a = fixture.write_archive("orbit.jsonl.A", &"x".repeat(100), Duration::from_secs(30));
    let b = fixture.write_archive("orbit.jsonl.B", &"x".repeat(100), Duration::from_secs(20));
    let c = fixture.write_archive("orbit.jsonl.C", &"x".repeat(100), Duration::from_secs(10));
    // 300 bytes total, budget 150: delete A -> 200, then B -> 100.
    let collector = fixture.collector(config(3_650, 150), None);

    let apply = execute_gc(&collector, fixture.request(true)).expect("apply");
    assert_eq!(apply.outcome, GcOutcome::Clean);
    assert_eq!(apply.targets[0].counts.reclaimed, 2);
    assert!(!a.exists(), "oldest archive is deleted first");
    assert!(!b.exists(), "second-oldest is deleted to fit the budget");
    assert!(c.exists(), "newest archive is kept");
    assert!(
        apply.targets[0]
            .items
            .iter()
            .all(|item| item.action == "delete-size"),
        "size selection is surfaced distinctly"
    );
}

#[test]
fn active_file_is_never_deleted_even_with_open_writer() {
    let fixture = Fixture::new();
    // Simulate a long-running writer holding the active inode open.
    let _writer = fs::File::options()
        .append(true)
        .open(&fixture.active)
        .expect("hold active open");
    let old = fixture.write_archive("orbit.jsonl.OLD", "old", Duration::from_secs(10 * 86_400));
    let collector = fixture.collector(config(7, 10_000_000), None);

    let apply = execute_gc(&collector, fixture.request(true)).expect("apply");
    assert_eq!(apply.outcome, GcOutcome::Clean);
    assert!(!old.exists(), "archive is reclaimed");
    assert!(fixture.active.exists(), "active file survives");
    assert_eq!(
        fs::read_to_string(&fixture.active).expect("active readable"),
        "active\n"
    );
}

#[test]
fn dry_run_and_apply_select_the_same_files() {
    let fixture = Fixture::new();
    fixture.write_archive("orbit.jsonl.OLD1", "a", Duration::from_secs(9 * 86_400));
    fixture.write_archive("orbit.jsonl.OLD2", "bb", Duration::from_secs(8 * 86_400));
    fixture.write_archive("orbit.jsonl.RECENT", "c", Duration::from_secs(60));
    let collector = fixture.collector(config(7, 10_000_000), None);

    let plan = execute_gc(&collector, fixture.request(false)).expect("plan");
    let apply = execute_gc(&collector, fixture.request(true)).expect("apply");
    assert_eq!(
        eligible_ids(&plan),
        reclaimed_ids(&apply),
        "apply reclaims exactly the frozen plan's eligible set"
    );
}

#[test]
fn second_apply_is_idempotent() {
    let fixture = Fixture::new();
    fixture.write_archive("orbit.jsonl.OLD", "data", Duration::from_secs(10 * 86_400));
    let collector = fixture.collector(config(7, 10_000_000), None);

    let first = execute_gc(&collector, fixture.request(true)).expect("first apply");
    let second = execute_gc(&collector, fixture.request(true)).expect("second apply");
    assert_eq!(first.targets[0].counts.reclaimed, 1);
    assert_eq!(second.targets[0].counts.eligible, 0);
    assert_eq!(second.targets[0].counts.reclaimed, 0);
    assert_eq!(second.outcome, GcOutcome::Clean);
}

#[test]
fn non_archive_entries_are_ignored() {
    let fixture = Fixture::new();
    let old = fixture.write_archive("orbit.jsonl.OLD", "old", Duration::from_secs(10 * 86_400));
    // A directory whose name matches the archive prefix, and an unrelated file.
    let masquerade = fixture.logs_dir.join("orbit.jsonl.DIR");
    fs::create_dir_all(&masquerade).expect("masquerade dir");
    let unrelated = fixture.logs_dir.join("other.log");
    fs::write(&unrelated, "unrelated").expect("unrelated file");
    let collector = fixture.collector(config(7, 10_000_000), None);

    let apply = execute_gc(&collector, fixture.request(true)).expect("apply");
    assert_eq!(apply.outcome, GcOutcome::Clean);
    assert_eq!(
        apply.targets[0].counts.scanned, 1,
        "only the real archive is scanned"
    );
    assert!(!old.exists(), "the real archive is reclaimed");
    assert!(
        masquerade.is_dir(),
        "a prefix-matching directory is left untouched"
    );
    assert!(unrelated.exists(), "an unrelated file is left untouched");
}

#[test]
fn custom_path_via_orbit_log_path_is_honored() {
    let fixture = Fixture::new();
    let custom_dir = fixture.root.join("custom");
    fs::create_dir_all(&custom_dir).expect("custom dir");
    let custom_active = custom_dir.join("feed.jsonl");
    fs::write(&custom_active, "active\n").expect("custom active");
    let custom_old = write_archive_in(
        &custom_dir,
        "feed.jsonl.OLD",
        "old",
        Duration::from_secs(10 * 86_400),
    );

    let _guard = EnvVarGuard::set("ORBIT_LOG_PATH", custom_active.clone().into_os_string());
    // `--retention 1d` overrides the age window; the 10-day-old archive is
    // selected regardless of the machine's configured retention.
    let collector = LogsGcCollector::from_scope(
        &GcScope::Global {
            root: fixture.root.clone(),
        },
        Some(Duration::from_secs(86_400)),
    );

    let apply = execute_gc(&collector, fixture.request(true)).expect("apply");
    assert_eq!(apply.outcome, GcOutcome::Clean);
    assert!(
        !custom_old.exists(),
        "archive beside ORBIT_LOG_PATH is reclaimed"
    );
    assert!(
        custom_active.exists(),
        "the custom active file is never deleted"
    );
    assert!(
        fixture.active.exists(),
        "the default feed is not managed when overridden"
    );
}

#[test]
fn custom_path_outside_default_root_is_honored() {
    // [ORB-10184] An explicitly configured ORBIT_LOG_PATH outside the default
    // global root is an owned active log: its archives must be planned and
    // reclaimed (with canonical no-follow containment against the configured
    // parent as an allowlisted owned root), not skipped.
    let fixture = Fixture::new();
    // A directory OUTSIDE the scope root entirely (sibling of `global`).
    let base = fixture.root.parent().expect("base dir");
    let external_dir = base.join("external-logs");
    fs::create_dir_all(&external_dir).expect("external dir");
    let external_active = external_dir.join("feed.jsonl");
    fs::write(&external_active, "active\n").expect("external active");
    let external_old = write_archive_in(
        &external_dir,
        "feed.jsonl.OLD",
        "old",
        Duration::from_secs(10 * 86_400),
    );

    let _guard = EnvVarGuard::set("ORBIT_LOG_PATH", external_active.clone().into_os_string());
    let collector = LogsGcCollector::from_scope(
        &GcScope::Global {
            root: fixture.root.clone(),
        },
        Some(Duration::from_secs(86_400)), // --retention 1d
    );

    // Plan: the external archive is eligible and nothing is deleted.
    let plan = execute_gc(&collector, fixture.request(false)).expect("plan");
    assert_eq!(plan.mode, GcMode::Plan);
    assert!(
        eligible_ids(&plan).contains("feed.jsonl.OLD"),
        "an outside-root configured log's archive must be planned, not skipped"
    );
    assert!(
        plan.targets[0].skipped.is_empty(),
        "the configured external path is honored, not skipped as out_of_scope"
    );
    assert!(external_old.exists(), "plan must not delete anything");

    // Apply: the external archive is reclaimed; both active files survive.
    let apply = execute_gc(&collector, fixture.request(true)).expect("apply");
    assert_eq!(apply.outcome, GcOutcome::Clean);
    assert!(
        reclaimed_ids(&apply).contains("feed.jsonl.OLD"),
        "the external archive is reclaimed under canonical containment"
    );
    assert!(!external_old.exists(), "the external archive is deleted");
    assert!(
        external_active.exists(),
        "the configured external active file is never deleted"
    );
    assert!(
        fixture.active.exists(),
        "the default feed is not managed when overridden"
    );

    // Idempotent: a second apply reclaims nothing.
    let second = execute_gc(&collector, fixture.request(true)).expect("second apply");
    assert_eq!(second.targets[0].counts.eligible, 0);
    assert_eq!(second.targets[0].counts.reclaimed, 0);
    assert_eq!(second.outcome, GcOutcome::Clean);
}
