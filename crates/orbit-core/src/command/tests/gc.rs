use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, TimeZone, Utc};
use fs2::FileExt;
use orbit_common::types::OrbitError;
use tempfile::TempDir;

use crate::command::gc::{
    EmptyGcCollector, GcCandidate, GcClock, GcCollector, GcContext, GcMode, GcMutation, GcOutcome,
    GcPlan, GcRequest, GcRevalidation, GcScope, GcTarget, execute_gc, validate_candidate_path,
};

struct FakeClock(DateTime<Utc>);

impl GcClock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

struct FileCollector {
    root: PathBuf,
    names: Vec<String>,
    fail: BTreeSet<String>,
    planned: Mutex<Vec<String>>,
    applied: Mutex<Vec<String>>,
}

impl FileCollector {
    fn new(root: &Path, names: &[&str]) -> Self {
        Self {
            root: root.to_path_buf(),
            names: names.iter().map(|name| (*name).to_string()).collect(),
            fail: BTreeSet::new(),
            planned: Mutex::new(Vec::new()),
            applied: Mutex::new(Vec::new()),
        }
    }

    fn failing(mut self, name: &str) -> Self {
        self.fail.insert(name.to_string());
        self
    }
}

impl GcCollector for FileCollector {
    fn target(&self) -> GcTarget {
        GcTarget::Logs
    }

    fn plan(&self, _context: &GcContext<'_>) -> Result<GcPlan, OrbitError> {
        let mut candidates = Vec::new();
        for name in &self.names {
            let path = self.root.join(name);
            if path.exists() {
                candidates.push(GcCandidate {
                    id: name.clone(),
                    action: "delete".to_string(),
                    path: Some(path),
                    bytes: Some(fs::metadata(self.root.join(name))?.len()),
                    ownership_evidence: "fake-owner".to_string(),
                    retention_evidence: "fake-clock".to_string(),
                    expected_state: "present".to_string(),
                    allow_owned_symlink: false,
                });
            }
        }
        *self.planned.lock().expect("planned lock") = candidates
            .iter()
            .map(|candidate| candidate.id.clone())
            .collect();
        Ok(GcPlan {
            target: GcTarget::Logs,
            config_source: "test".to_string(),
            scanned: self.names.len() as u64,
            scanned_bytes: Some(
                candidates
                    .iter()
                    .filter_map(|candidate| candidate.bytes)
                    .sum(),
            ),
            candidates,
            skipped: Vec::new(),
            errors: Vec::new(),
        })
    }

    fn revalidate(
        &self,
        candidate: &GcCandidate,
        _context: &GcContext<'_>,
    ) -> Result<GcRevalidation, OrbitError> {
        if candidate.path.as_ref().is_some_and(|path| path.exists()) {
            Ok(GcRevalidation::Ready)
        } else {
            Ok(GcRevalidation::Skip {
                code: "state_changed".to_string(),
                reason: "candidate disappeared".to_string(),
            })
        }
    }

    fn apply(
        &self,
        candidate: &GcCandidate,
        _context: &GcContext<'_>,
    ) -> Result<GcMutation, OrbitError> {
        self.applied
            .lock()
            .expect("applied lock")
            .push(candidate.id.clone());
        if self.fail.contains(&candidate.id) {
            return Err(OrbitError::Execution("injected failure".to_string()));
        }
        fs::remove_file(candidate.path.as_ref().expect("candidate path"))?;
        Ok(GcMutation {
            reclaimed_bytes: candidate.bytes,
        })
    }
}

struct Fixture {
    temp: TempDir,
    root: PathBuf,
    state: PathBuf,
    clock: FakeClock,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("temp root");
        let root = temp.path().join("owned");
        let state = temp.path().join("global-state");
        fs::create_dir_all(&root).expect("owned root");
        Self {
            temp,
            root,
            state,
            clock: FakeClock(
                Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0)
                    .single()
                    .expect("fixed time"),
            ),
        }
    }

    fn write(&self, name: &str, contents: &str) {
        fs::write(self.root.join(name), contents).expect("fixture file");
    }

    fn request(&self, apply: bool) -> GcRequest<'_> {
        GcRequest {
            apply,
            scope: GcScope::Workspace {
                workspace_id: Some("test".to_string()),
                root: self.root.clone(),
            },
            retention_override: Some("1d"),
            global_state_dir: &self.state,
            clock: &self.clock,
        }
    }
}

#[test]
fn plan_is_a_noop_and_apply_consumes_only_the_frozen_candidates() {
    let fixture = Fixture::new();
    fixture.write("one", "1");
    fixture.write("two", "22");
    let collector = FileCollector::new(&fixture.root, &["one", "two"]);

    let plan = execute_gc(&collector, fixture.request(false)).expect("plan report");
    assert_eq!(plan.mode, GcMode::Plan);
    assert_eq!(plan.targets[0].counts.eligible, 2);
    assert_eq!(plan.targets[0].counts.reclaimed, 0);
    assert!(fixture.root.join("one").exists());
    assert!(collector.applied.lock().expect("applied lock").is_empty());

    let apply = execute_gc(&collector, fixture.request(true)).expect("apply report");
    assert_eq!(apply.targets[0].counts.eligible, 2);
    assert_eq!(apply.targets[0].counts.reclaimed, 2);
    assert_eq!(apply.outcome, GcOutcome::Clean);
    assert_eq!(
        *collector.planned.lock().expect("planned lock"),
        *collector.applied.lock().expect("applied lock")
    );
    let manifest =
        fs::read_to_string(apply.manifest_path.expect("manifest path")).expect("manifest contents");
    assert_eq!(manifest.lines().count(), 4);
    assert_eq!(manifest.matches("\"result\":\"attempting\"").count(), 2);
    assert_eq!(manifest.matches("\"result\":\"reclaimed\"").count(), 2);
}

#[test]
fn partial_failure_preserves_success_and_reports_the_item_error() {
    let fixture = Fixture::new();
    fixture.write("good", "ok");
    fixture.write("bad", "no");
    fixture.write("later", "ok");
    let collector = FileCollector::new(&fixture.root, &["good", "bad", "later"]).failing("bad");

    let report = execute_gc(&collector, fixture.request(true)).expect("partial report");
    assert_eq!(report.outcome, GcOutcome::Partial);
    assert!(report.has_errors());
    assert_eq!(report.targets[0].counts.reclaimed, 2);
    assert!(!fixture.root.join("good").exists());
    assert!(fixture.root.join("bad").exists());
    assert!(!fixture.root.join("later").exists());
    assert_eq!(report.targets[0].errors[0].id, "bad");
}

#[test]
fn second_apply_is_idempotent() {
    let fixture = Fixture::new();
    fixture.write("old", "data");
    let collector = FileCollector::new(&fixture.root, &["old"]);

    let first = execute_gc(&collector, fixture.request(true)).expect("first apply");
    let second = execute_gc(&collector, fixture.request(true)).expect("second apply");
    assert_eq!(first.targets[0].counts.reclaimed, 1);
    assert_eq!(second.targets[0].counts.eligible, 0);
    assert_eq!(second.targets[0].counts.reclaimed, 0);
    assert_eq!(second.outcome, GcOutcome::Clean);
}

#[test]
fn containment_rejects_escape_root_and_parent_traversal() {
    let fixture = Fixture::new();
    fixture.write("safe", "data");
    assert!(validate_candidate_path(&fixture.root, &fixture.root.join("safe"), false).is_ok());
    assert!(validate_candidate_path(&fixture.root, fixture.temp.path(), false).is_err());
    assert!(validate_candidate_path(&fixture.root, Path::new("../outside"), false).is_err());
}

#[test]
fn apply_fails_without_planning_when_host_gc_lock_is_contended() {
    let fixture = Fixture::new();
    fs::create_dir_all(&fixture.state).expect("global state");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(fixture.state.join("gc.lock"))
        .expect("GC lock file");
    lock.lock_exclusive().expect("hold GC lock");

    let error = execute_gc(
        &EmptyGcCollector::new(GcTarget::Logs),
        fixture.request(true),
    )
    .expect_err("contended apply must fail closed");
    assert!(error.to_string().contains("timed out waiting"), "{error}");
    FileExt::unlock(&lock).expect("release GC lock");
}

#[cfg(unix)]
#[test]
fn containment_rejects_symlink_escape_and_can_unlink_owned_final_symlink() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let outside = fixture.temp.path().join("outside");
    fs::create_dir_all(&outside).expect("outside root");
    fs::write(outside.join("secret"), "secret").expect("outside file");
    symlink(&outside, fixture.root.join("escape")).expect("escape symlink");

    assert!(
        validate_candidate_path(&fixture.root, &fixture.root.join("escape/secret"), false).is_err()
    );
    assert!(validate_candidate_path(&fixture.root, &fixture.root.join("escape"), false).is_err());
    assert!(validate_candidate_path(&fixture.root, &fixture.root.join("escape"), true).is_ok());
}
