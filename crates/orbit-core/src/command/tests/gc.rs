use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, TimeZone, Utc};
use fs2::FileExt;
use orbit_common::types::{JobRun, JobRunState, OrbitError, PipelineState};
use tempfile::TempDir;

use crate::OrbitRuntime;
use crate::command::gc::{
    EmptyGcCollector, GcCandidate, GcClock, GcCollector, GcContext, GcMode, GcMutation, GcOutcome,
    GcPlan, GcRequest, GcRevalidation, GcScope, GcTarget, RunGcCollector, RunGcPolicy, execute_gc,
    validate_candidate_path,
};
use crate::command::task::{TaskAddParams, TaskUpdateParams};

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

#[test]
fn run_gc_stages_archive_then_purge_and_protects_active_failed_and_task_linked_runs() {
    let temp = TempDir::new().expect("tempdir");
    let global = temp.path().join("global");
    let orbit = temp.path().join("repo/.orbit");
    fs::create_dir_all(&global).expect("global root");
    fs::create_dir_all(&orbit).expect("workspace root");
    fs::write(
        orbit.join("config.toml"),
        "[gc.runs]\narchive_after_days = 0\npurge_after_days = 0\nfailure_archive_after_days = 30\nfailure_purge_after_days = 90\n",
    )
    .expect("config");
    let runtime = OrbitRuntime::from_roots(&global, &orbit).expect("runtime");
    let terminal_at = Utc
        .with_ymd_and_hms(2026, 6, 1, 0, 0, 0)
        .single()
        .expect("terminal time");
    let clock = FakeClock(
        Utc.with_ymd_and_hms(2026, 6, 11, 0, 0, 0)
            .single()
            .expect("collection time"),
    );
    let insert = |state: JobRunState| {
        let run = runtime
            .stores()
            .jobs()
            .insert_run("job", 1, terminal_at, None, None)
            .expect("insert run");
        if state != JobRunState::Pending {
            runtime
                .stores()
                .jobs()
                .mark_run_running(&run.run_id, terminal_at, std::process::id())
                .expect("start run");
        }
        if state.is_terminal() {
            runtime
                .stores()
                .jobs()
                .finalize_run(&run.run_id, state, terminal_at, Some(0))
                .expect("finalize run");
        }
        runtime
            .stores()
            .jobs()
            .get_run(&run.run_id)
            .expect("read run")
            .expect("stored run")
    };
    let success = insert(JobRunState::Success);
    let failed = insert(JobRunState::Failed);
    let active = insert(JobRunState::Pending);
    let resumable = insert(JobRunState::Interrupted);
    let mut checkpoint = PipelineState::new(
        resumable.run_id.clone(),
        resumable.job_id.clone(),
        serde_json::json!({}),
    );
    checkpoint.record_step(
        0,
        JobRunState::Success,
        Some(serde_json::json!({"ok": true})),
        None,
    );
    runtime
        .stores()
        .jobs()
        .write_run_state(&resumable.run_id, &checkpoint)
        .expect("write resumable checkpoint");
    let mut live_owner = std::process::Command::new("sh")
        .args(["-c", "sleep 30"])
        .spawn()
        .expect("spawn live owner");
    let live_terminal = runtime
        .stores()
        .jobs()
        .insert_run("job", 1, terminal_at, None, None)
        .expect("insert live terminal run");
    runtime
        .stores()
        .jobs()
        .mark_run_running(&live_terminal.run_id, terminal_at, live_owner.id())
        .expect("mark live-owned run running");
    runtime
        .stores()
        .jobs()
        .finalize_run(
            &live_terminal.run_id,
            JobRunState::Success,
            terminal_at,
            Some(0),
        )
        .expect("finalize live-owned run");
    let linked = insert(JobRunState::Success);
    let task = runtime
        .add_task(TaskAddParams {
            title: "linked".to_string(),
            description: "linked".to_string(),
            ..Default::default()
        })
        .expect("add task");
    runtime
        .update_task(
            task.id.as_str(),
            TaskUpdateParams {
                job_run_id: Some(Some(linked.run_id.clone())),
                ..Default::default()
            },
        )
        .expect("link task");

    let collector = RunGcCollector::new(&runtime, RunGcPolicy::from_runtime(&runtime));
    let global_state = global.join("state");
    let request = |apply| GcRequest {
        apply,
        scope: GcScope::Workspace {
            workspace_id: None,
            root: orbit.clone(),
        },
        retention_override: None,
        global_state_dir: &global_state,
        clock: &clock,
    };
    let plan = execute_gc(&collector, request(false)).expect("plan");
    assert_eq!(plan.targets[0].counts.eligible, 1);
    assert_eq!(plan.targets[0].items[0].id, success.run_id);
    for code in [
        "retained",
        "active_run",
        "resumable",
        "live_or_inconclusive",
        "task_linked",
    ] {
        assert!(
            plan.targets[0].skipped.iter().any(|skip| skip.code == code),
            "missing skip {code}: {:?}",
            plan.targets[0].skipped
        );
    }

    let archive = execute_gc(&collector, request(true)).expect("archive");
    assert_eq!(archive.targets[0].counts.reclaimed, 1);
    assert!(runtime.show_job_run(&success.run_id).is_err());
    let purge = execute_gc(&collector, request(true)).expect("purge");
    assert_eq!(purge.targets[0].counts.reclaimed, 1);
    assert!(
        runtime
            .stores()
            .jobs()
            .list_runs_for_gc()
            .expect("inventory")
            .iter()
            .all(|record| record.run.run_id != success.run_id)
    );
    let idempotent = execute_gc(&collector, request(true)).expect("idempotent");
    assert_eq!(idempotent.targets[0].counts.reclaimed, 0);

    let legacy = orbit
        .join("state/job-runs")
        .join(&success.job_id)
        .join(&success.run_id);
    fs::create_dir_all(&legacy).expect("legacy bundle");
    fs::write(
        legacy.join("jrun.yaml"),
        serde_yaml::to_string(&serde_json::json!({
            "schema_version": 1,
            "run": success,
        }))
        .expect("legacy yaml"),
    )
    .expect("legacy run document");
    let corrupt = orbit.join("state/job-runs/job/jrun-corrupt");
    fs::create_dir_all(&corrupt).expect("corrupt legacy bundle");
    fs::write(corrupt.join("jrun.yaml"), "not: [valid").expect("corrupt document");
    let stale_archive = execute_gc(&collector, request(true)).expect("archive stale bundle");
    assert_eq!(stale_archive.targets[0].counts.reclaimed, 1);
    assert_eq!(stale_archive.outcome, GcOutcome::Partial);
    assert!(stale_archive.has_errors());
    let stale_purge = execute_gc(&collector, request(true)).expect("purge stale bundle");
    assert_eq!(stale_purge.targets[0].counts.reclaimed, 1);
    assert!(!legacy.exists());
    assert!(
        runtime
            .stores()
            .jobs()
            .get_run(&failed.run_id)
            .expect("failed retained")
            .is_some()
    );
    assert!(
        runtime
            .stores()
            .jobs()
            .get_run(&active.run_id)
            .expect("active retained")
            .is_some()
    );
    live_owner.kill().expect("stop live owner");
    live_owner.wait().expect("wait for live owner");
}

/// Build a runtime whose run GC ages are all zero (every terminal run is
/// immediately eligible) so tests exercise protection holds, not the clock.
fn run_gc_runtime(temp: &TempDir) -> (OrbitRuntime, PathBuf) {
    let global = temp.path().join("global");
    let orbit = temp.path().join("repo/.orbit");
    fs::create_dir_all(&global).expect("global root");
    fs::create_dir_all(&orbit).expect("workspace root");
    fs::write(
        orbit.join("config.toml"),
        "[gc.runs]\narchive_after_days = 0\npurge_after_days = 0\nfailure_archive_after_days = 0\nfailure_purge_after_days = 0\n",
    )
    .expect("config");
    let runtime = OrbitRuntime::from_roots(&global, &orbit).expect("runtime");
    (runtime, orbit)
}

/// Write a rowless legacy job-run bundle (`state/job-runs/<job>/<run>/jrun.yaml`)
/// describing `run`, mirroring the on-disk layout the collector inventories.
fn write_legacy_bundle(orbit: &Path, run: &JobRun) -> PathBuf {
    let dir = orbit
        .join("state/job-runs")
        .join(&run.job_id)
        .join(&run.run_id);
    fs::create_dir_all(&dir).expect("legacy bundle dir");
    fs::write(
        dir.join("jrun.yaml"),
        serde_yaml::to_string(&serde_json::json!({ "schema_version": 1, "run": run }))
            .expect("legacy yaml"),
    )
    .expect("legacy run document");
    dir
}

/// Insert a terminal (`Success`) run owned by `pid` and return the captured
/// `JobRun` with its recorded owner identity intact. The row is left in place so
/// that a batch of runs can be inserted with distinct ids before any are deleted
/// (deleting between inserts would let `next_run_id` reuse a just-freed id).
fn insert_terminal(runtime: &OrbitRuntime, terminal_at: DateTime<Utc>, pid: u32) -> JobRun {
    let run = runtime
        .stores()
        .jobs()
        .insert_run("job", 1, terminal_at, None, None)
        .expect("insert run");
    runtime
        .stores()
        .jobs()
        .mark_run_running(&run.run_id, terminal_at, pid)
        .expect("start run");
    runtime
        .stores()
        .jobs()
        .finalize_run(&run.run_id, JobRunState::Success, terminal_at, Some(0))
        .expect("finalize run");
    runtime
        .stores()
        .jobs()
        .get_run(&run.run_id)
        .expect("read run")
        .expect("stored run")
}

/// Delete a run's authoritative row so only a rowless legacy bundle can remain.
fn delete_row(runtime: &OrbitRuntime, run: &JobRun) {
    runtime
        .delete_job_run(&run.run_id)
        .expect("delete authoritative row");
}

// ORB-10183 P1: a rowless legacy bundle must fail closed on the SAME
// ownership/liveness/reference protections as an authoritative row — never
// eligible from terminal state and age alone.
#[test]
fn run_gc_rowless_legacy_bundle_fails_closed_on_live_owner_task_and_retry_references() {
    let temp = TempDir::new().expect("tempdir");
    let (runtime, orbit) = run_gc_runtime(&temp);
    let terminal_at = Utc
        .with_ymd_and_hms(2026, 6, 1, 0, 0, 0)
        .single()
        .expect("terminal time");
    let clock = FakeClock(
        Utc.with_ymd_and_hms(2026, 6, 11, 0, 0, 0)
            .single()
            .expect("collection time"),
    );

    // Insert every terminal run while they coexist so each gets a distinct id,
    // then delete the four that must become rowless legacy bundles.
    let mut live_owner = std::process::Command::new("sh")
        .args(["-c", "sleep 30"])
        .spawn()
        .expect("spawn live owner");
    // Recorded owner is still alive: liveness is proven, so the bundle is held.
    let live = insert_terminal(&runtime, terminal_at, live_owner.id());
    // Owner permits reclaim (it is this process): only the task link can hold it.
    let task_owned = insert_terminal(&runtime, terminal_at, std::process::id());
    // Owner permits reclaim: only a retained retry run pointing here can hold it.
    let retry_source = insert_terminal(&runtime, terminal_at, std::process::id());
    // Reclaimable owner, no references: stays eligible, proving the protections
    // gate rather than blanket-skip the legacy path.
    let clean = insert_terminal(&runtime, terminal_at, std::process::id());

    // A retained retry run still points back to `retry_source` as its source.
    runtime
        .stores()
        .jobs()
        .insert_run(
            "job",
            2,
            terminal_at,
            None,
            Some(retry_source.run_id.clone()),
        )
        .expect("insert retry run");
    // A retained task still references `task_owned`.
    let task = runtime
        .add_task(TaskAddParams {
            title: "linked".to_string(),
            description: "linked".to_string(),
            ..Default::default()
        })
        .expect("add task");
    runtime
        .update_task(
            task.id.as_str(),
            TaskUpdateParams {
                job_run_id: Some(Some(task_owned.run_id.clone())),
                ..Default::default()
            },
        )
        .expect("link task");

    for run in [&live, &task_owned, &retry_source, &clean] {
        delete_row(&runtime, run);
        write_legacy_bundle(&orbit, run);
    }

    let collector = RunGcCollector::new(&runtime, RunGcPolicy::from_runtime(&runtime));
    let scope = GcScope::Workspace {
        workspace_id: None,
        root: orbit.clone(),
    };
    let context = GcContext {
        scope: &scope,
        retention_override: None,
        clock: &clock,
    };
    let plan = collector.plan(&context).expect("plan");

    let candidate_ids: Vec<&str> = plan
        .candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect();
    assert!(
        candidate_ids.contains(&clean.run_id.as_str()),
        "reclaimable rowless bundle must be eligible: {candidate_ids:?}"
    );
    for held in [&live.run_id, &task_owned.run_id, &retry_source.run_id] {
        assert!(
            !candidate_ids.contains(&held.as_str()),
            "protected rowless bundle {held} must not be a candidate: {candidate_ids:?}"
        );
    }
    for (run_id, code) in [
        (&live.run_id, "live_or_inconclusive"),
        (&task_owned.run_id, "task_linked"),
        (&retry_source.run_id, "retry_linked"),
    ] {
        assert!(
            plan.skipped
                .iter()
                .any(|skip| &skip.id == run_id && skip.code == code),
            "missing {code} hold for {run_id}: {:?}",
            plan.skipped
        );
    }

    live_owner.kill().expect("stop live owner");
    live_owner.wait().expect("wait for live owner");
}

// ORB-10183 P1: if an authoritative row materializes between plan and apply, the
// rowless legacy path must fail closed — the persisted collector owns the run —
// holding the per-run claim guard across revalidation and refusing to mutate.
#[test]
fn run_gc_rowless_legacy_bundle_skips_and_holds_guard_when_row_appears_before_apply() {
    let temp = TempDir::new().expect("tempdir");
    let (runtime, orbit) = run_gc_runtime(&temp);
    let terminal_at = Utc
        .with_ymd_and_hms(2026, 6, 1, 0, 0, 0)
        .single()
        .expect("terminal time");
    let clock = FakeClock(
        Utc.with_ymd_and_hms(2026, 6, 11, 0, 0, 0)
            .single()
            .expect("collection time"),
    );

    // Freeze a legacy candidate while the run is rowless and reclaimable.
    let stored = insert_terminal(&runtime, terminal_at, std::process::id());
    delete_row(&runtime, &stored);
    let bundle_dir = write_legacy_bundle(&orbit, &stored);
    let collector = RunGcCollector::new(&runtime, RunGcPolicy::from_runtime(&runtime));
    let scope = GcScope::Workspace {
        workspace_id: None,
        root: orbit.clone(),
    };
    let context = GcContext {
        scope: &scope,
        retention_override: None,
        clock: &clock,
    };
    let plan = collector.plan(&context).expect("plan");
    let candidate = plan
        .candidates
        .iter()
        .find(|candidate| candidate.id == stored.run_id)
        .expect("rowless legacy candidate")
        .clone();

    // The authoritative row reappears with the same id, a resumable checkpoint,
    // and an interrupted (recoverable) state — as if a worker reclaimed the run.
    let mut appeared = stored.clone();
    appeared.state = JobRunState::Interrupted;
    let mut checkpoint = PipelineState::new(
        appeared.run_id.clone(),
        appeared.job_id.clone(),
        serde_json::json!({}),
    );
    checkpoint.record_step(
        0,
        JobRunState::Success,
        Some(serde_json::json!({"ok": true})),
        None,
    );
    let store = runtime.sqlite_store().expect("store");
    let workspace_id = runtime.workspace_id().expect("workspace id");
    store
        .upsert_job_run_for_workspace(&workspace_id, &appeared, Some(&checkpoint))
        .expect("plant reappeared row");

    // Revalidation fails closed: the persisted collector now owns this run.
    match collector
        .revalidate(&candidate, &context)
        .expect("revalidate")
    {
        GcRevalidation::Skip { code, .. } => assert_eq!(code, "row_appeared"),
        other => panic!("expected row_appeared skip, got {other:?}"),
    }

    // Apply holds the per-run claim guard across the recheck and refuses to
    // mutate — nothing purged, nothing stranded, the reappeared row survives.
    let error = collector
        .apply(&candidate, &context)
        .expect_err("apply must fail closed once a row appears");
    assert!(
        matches!(error, OrbitError::Execution(_)),
        "unexpected apply error: {error:?}"
    );
    assert!(bundle_dir.exists(), "legacy bundle must not be mutated");
    assert!(bundle_dir.join("jrun.yaml").exists());
    assert!(
        runtime
            .stores()
            .jobs()
            .get_run(&appeared.run_id)
            .expect("read reappeared run")
            .is_some(),
        "reappeared authoritative row must survive"
    );
}
