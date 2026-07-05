//! Sweep-orchestration tests [ORB-00421]: exercise the fire / idempotency /
//! overlap / retry / outcome-sync logic in `routines/sweep.rs` that shipped
//! untested in [ORB-10021].
//!
//! Two layers:
//! - `run_sweep_core` against an in-memory store, a hand-built
//!   [`RoutineCollection`], a fake [`RoutineDispatch`], and an explicit `now`
//!   — deterministic, no pipeline workers spawned.
//! - one `run_sweep_at` integration pass over a seeded global root, covering
//!   discovery + fail-closed loading end-to-end via `--dry-run` (which records
//!   and dispatches nothing).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, TimeZone, Utc};
use orbit_common::types::{
    JobRunState, OrbitError, RoutineDefinition, Workspace, WorkspaceRegistry, WorkspaceStatus,
    parse_routine_yaml,
};
use orbit_store::{RoutineFireIntentParams, RoutineFireState, Store};
use tempfile::tempdir;

use crate::routines::loader::{LoadedRoutine, RoutineCollection};
use crate::routines::sweep::{RoutineDispatch, SweepOptions, run_sweep_at, run_sweep_core};
use crate::workspace_registry;

const HOST: &str = "test-host";
const SOURCE_DIR: &str = "/ws/.orbit";

// ---- fixtures -------------------------------------------------------------

/// Build a validated routine with the common knobs the tests vary.
fn routine(
    name: &str,
    cron: &str,
    enabled: bool,
    overlap: &str,
    retries_max: u32,
) -> LoadedRoutine {
    let yaml = format!(
        "schemaVersion: 1\n\
         name: {name}\n\
         enabled: {enabled}\n\
         hosts: [{HOST}]\n\
         trigger:\n  cron: \"{cron}\"\n\
         target: job:noop\n\
         policy:\n  timeout_minutes: 10\n  overlap: {overlap}\n  \
         retries: {{ max: {retries_max}, backoff_minutes: 1 }}\n"
    );
    loaded(parse_routine_yaml(&yaml).expect("valid routine yaml"))
}

fn loaded(definition: RoutineDefinition) -> LoadedRoutine {
    let name = definition.name.clone();
    LoadedRoutine {
        definition,
        source_workspace: "polaris".to_string(),
        source_orbit_dir: PathBuf::from(SOURCE_DIR),
        path: PathBuf::from(format!("{SOURCE_DIR}/routines/{name}.yaml")),
    }
}

fn collection(routines: Vec<LoadedRoutine>) -> RoutineCollection {
    RoutineCollection {
        routines,
        errors: Vec::new(),
    }
}

fn store() -> Store {
    Store::open_in_memory().expect("in-memory store")
}

fn ts(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, mo, d, h, mi, s)
        .single()
        .expect("valid ts")
}

/// A scriptable dispatch double: records submissions, hands back deterministic
/// run ids, and answers `run_state` from a table the test primes.
#[derive(Default)]
struct FakeDispatch {
    fail_submit: bool,
    counter: Cell<u32>,
    submits: RefCell<Vec<(PathBuf, String)>>,
    states: RefCell<HashMap<String, JobRunState>>,
}

impl FakeDispatch {
    fn submit_count(&self) -> usize {
        self.submits.borrow().len()
    }
    fn set_state(&self, run_id: &str, state: JobRunState) {
        self.states.borrow_mut().insert(run_id.to_string(), state);
    }
}

impl RoutineDispatch for FakeDispatch {
    fn submit(&self, dir: &Path, job: &str, _actor: &str) -> Result<String, OrbitError> {
        if self.fail_submit {
            return Err(OrbitError::Execution("dispatch boom".to_string()));
        }
        let n = self.counter.get() + 1;
        self.counter.set(n);
        self.submits
            .borrow_mut()
            .push((dir.to_path_buf(), job.to_string()));
        Ok(format!("run-{n}"))
    }

    fn run_state(&self, _dir: &Path, run_id: &str) -> Option<JobRunState> {
        self.states.borrow().get(run_id).cloned()
    }
}

fn fires(store: &Store, name: &str) -> Vec<orbit_store::RoutineFireRecord> {
    store.routine_recent_fires(name, 32).expect("recent fires")
}

// ---- baseline & fire ------------------------------------------------------

#[test]
fn first_sweep_baselines_and_fires_nothing_then_next_slot_fires() {
    let store = store();
    let dispatch = FakeDispatch::default();
    let coll = collection(vec![routine("nightly", "* * * * *", true, "allow", 0)]);

    // First observation: baseline is recorded, nothing fires.
    let reports = run_sweep_core(
        &store,
        HOST,
        &coll,
        &dispatch,
        SweepOptions::default(),
        ts(2026, 1, 1, 0, 0, 30),
    )
    .expect("sweep 1");
    assert_eq!(reports[0].action, "baselined");
    assert!(store.routine_cursor("nightly").unwrap().is_some());
    assert!(fires(&store, "nightly").is_empty());
    assert_eq!(dispatch.submit_count(), 0);

    // A later natural slot fires exactly once.
    let reports = run_sweep_core(
        &store,
        HOST,
        &coll,
        &dispatch,
        SweepOptions::default(),
        ts(2026, 1, 1, 0, 1, 20),
    )
    .expect("sweep 2");
    assert_eq!(reports[0].action, "fired");
    let rows = fires(&store, "nightly");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, RoutineFireState::Dispatched);
    assert_eq!(dispatch.submit_count(), 1);
}

#[test]
fn same_slot_second_sweep_does_not_double_fire() {
    let store = store();
    let dispatch = FakeDispatch::default();
    let coll = collection(vec![routine("nightly", "* * * * *", true, "allow", 0)]);
    // Baseline in the past so the target minute is already fireable.
    store
        .routine_record_baseline("nightly", &ts(2026, 1, 1, 0, 0, 0).to_rfc3339())
        .unwrap();

    let opts = SweepOptions::default();
    let first = run_sweep_core(
        &store,
        HOST,
        &coll,
        &dispatch,
        opts,
        ts(2026, 1, 1, 0, 1, 10),
    )
    .unwrap();
    assert_eq!(first[0].action, "fired");

    // Second sweep in the SAME minute: the consumed slot is not re-fired.
    let second = run_sweep_core(
        &store,
        HOST,
        &coll,
        &dispatch,
        opts,
        ts(2026, 1, 1, 0, 1, 50),
    )
    .unwrap();
    assert_eq!(second[0].action, "skipped");
    assert_eq!(second[0].reason.as_deref(), Some("not_due"));

    assert_eq!(
        fires(&store, "nightly").len(),
        1,
        "exactly one fire for the slot"
    );
    assert_eq!(dispatch.submit_count(), 1);
}

// ---- toggles --------------------------------------------------------------

#[test]
fn toggles_suppress_the_fire_with_the_right_reason() {
    let store = store();
    let dispatch = FakeDispatch::default();
    store
        .routine_record_baseline("disabled", &ts(2026, 1, 1, 0, 0, 0).to_rfc3339())
        .unwrap();
    store
        .routine_record_baseline("paused", &ts(2026, 1, 1, 0, 0, 0).to_rfc3339())
        .unwrap();
    store
        .routine_record_baseline("elsewhere", &ts(2026, 1, 1, 0, 0, 0).to_rfc3339())
        .unwrap();
    store.routine_pause("paused", "test").unwrap();

    let mut off = routine("elsewhere", "* * * * *", true, "allow", 0);
    off.definition.hosts = vec!["other-host".to_string()];
    let coll = collection(vec![
        routine("disabled", "* * * * *", false, "allow", 0),
        routine("paused", "* * * * *", true, "allow", 0),
        off,
    ]);

    let reports = run_sweep_core(
        &store,
        HOST,
        &coll,
        &dispatch,
        SweepOptions::default(),
        ts(2026, 1, 1, 0, 5, 10),
    )
    .unwrap();

    let reason = |name: &str| {
        reports
            .iter()
            .find(|r| r.routine == name)
            .and_then(|r| r.reason.clone())
    };
    assert_eq!(
        reason("disabled").as_deref(),
        Some("disabled_in_definition")
    );
    assert_eq!(reason("paused").as_deref(), Some("paused_locally"));
    assert_eq!(reason("elsewhere").as_deref(), Some("host_not_pinned"));
    assert_eq!(dispatch.submit_count(), 0);
}

// ---- overlap: forbid ------------------------------------------------------

#[test]
fn overlap_forbid_skips_while_in_flight_then_fires_once_terminal() {
    let store = store();
    let dispatch = FakeDispatch::default();
    let coll = collection(vec![routine("job", "* * * * *", true, "forbid", 0)]);

    // Seed a dispatched, still-in-flight fire at 00:05 and point the cursor at it.
    store
        .routine_record_baseline("job", &ts(2026, 1, 1, 0, 0, 0).to_rfc3339())
        .unwrap();
    let slot_in_flight = ts(2026, 1, 1, 0, 5, 0).to_rfc3339();
    store
        .routine_record_fire_intent(&RoutineFireIntentParams {
            routine_name: "job".to_string(),
            slot: slot_in_flight.clone(),
            attempt: 1,
            source_workspace: "polaris".to_string(),
        })
        .unwrap();
    store
        .routine_mark_fire_dispatched("job", &slot_in_flight, 1, "inflight")
        .unwrap();

    // A new slot comes due while the prior fire is non-terminal -> skipped.
    // `now` sits before the seeded fire's real-clock created_at, so the outcome
    // sync cannot reclaim it as stale — it is genuinely in flight.
    let reports = run_sweep_core(
        &store,
        HOST,
        &coll,
        &dispatch,
        SweepOptions::default(),
        ts(2026, 1, 1, 0, 6, 20),
    )
    .unwrap();
    assert_eq!(reports[0].action, "skipped");
    assert_eq!(reports[0].reason.as_deref(), Some("overlap_in_flight"));
    assert_eq!(dispatch.submit_count(), 0);

    // Once the in-flight run reaches a terminal state, the next slot fires.
    dispatch.set_state("inflight", JobRunState::Success);
    let reports = run_sweep_core(
        &store,
        HOST,
        &coll,
        &dispatch,
        SweepOptions::default(),
        ts(2026, 1, 1, 0, 7, 20),
    )
    .unwrap();
    assert_eq!(reports[0].action, "fired");
    assert_eq!(dispatch.submit_count(), 1);
    // The reclaimed fire is now terminal.
    let seeded = fires(&store, "job")
        .into_iter()
        .find(|f| f.slot == slot_in_flight)
        .unwrap();
    assert_eq!(seeded.state, RoutineFireState::Succeeded);
}

// ---- outcome sync / staleness horizon -------------------------------------

#[test]
fn sync_reclaims_stale_intent_and_dispatched_past_timeout() {
    let store = store();
    let dispatch = FakeDispatch::default();
    // enabled:false so the per-routine pass is a no-op and only the outcome
    // sync at the top of the pass touches these fires.
    let coll = collection(vec![routine("job", "* * * * *", false, "forbid", 0)]);

    let now = Utc::now();
    store
        .routine_record_baseline("job", &now.to_rfc3339())
        .unwrap();
    // Stale intent (sweep died before dispatch).
    store
        .routine_record_fire_intent(&RoutineFireIntentParams {
            routine_name: "job".to_string(),
            slot: "2026-01-01T00:01:00+00:00".to_string(),
            attempt: 1,
            source_workspace: "polaris".to_string(),
        })
        .unwrap();
    // Dispatched but the run is unqueryable (fake returns no state).
    store
        .routine_record_fire_intent(&RoutineFireIntentParams {
            routine_name: "job".to_string(),
            slot: "2026-01-01T00:02:00+00:00".to_string(),
            attempt: 1,
            source_workspace: "polaris".to_string(),
        })
        .unwrap();
    store
        .routine_mark_fire_dispatched("job", "2026-01-01T00:02:00+00:00", 1, "lost-run")
        .unwrap();

    // Advance `now` past the 10-minute policy timeout so both are reclaimable.
    let reclaim_at = now + Duration::hours(2);
    run_sweep_core(
        &store,
        HOST,
        &coll,
        &dispatch,
        SweepOptions::default(),
        reclaim_at,
    )
    .unwrap();

    let by_slot: HashMap<String, RoutineFireState> = fires(&store, "job")
        .into_iter()
        .map(|f| (f.slot, f.state))
        .collect();
    assert_eq!(
        by_slot.get("2026-01-01T00:01:00+00:00"),
        Some(&RoutineFireState::Error),
        "stale intent reclaimed as error"
    );
    assert_eq!(
        by_slot.get("2026-01-01T00:02:00+00:00"),
        Some(&RoutineFireState::TimedOut),
        "stale dispatched reclaimed as timed_out"
    );
}

// ---- dry-run --------------------------------------------------------------

#[test]
fn dry_run_records_no_state() {
    let store = store();
    let dispatch = FakeDispatch::default();
    let coll = collection(vec![routine("nightly", "* * * * *", true, "allow", 0)]);
    store
        .routine_record_baseline("seen", &ts(2026, 1, 1, 0, 0, 0).to_rfc3339())
        .unwrap();

    let reports = run_sweep_core(
        &store,
        HOST,
        &coll,
        &dispatch,
        SweepOptions { dry_run: true },
        ts(2026, 1, 1, 0, 5, 10),
    )
    .unwrap();

    // First-observation routine reports would_baseline but records no cursor.
    assert_eq!(reports[0].action, "would_baseline");
    assert!(store.routine_cursor("nightly").unwrap().is_none());
    assert!(fires(&store, "nightly").is_empty());
    assert_eq!(dispatch.submit_count(), 0);
}

// ---- run_sweep_at end-to-end (discovery + fail-closed loading) ------------

const NOOP_JOB: &str = "schemaVersion: 2\n\
kind: Job\n\
metadata:\n  name: noop\n\
spec:\n  state: enabled\n  kind: workflow\n  max_active_runs: 1\n  \
steps:\n    - id: noop\n      target: activity:worktree_setup\n      \
default_input:\n        task_id: \"qa\"\n";

fn source_routine_yaml(name: &str, target: &str, hosts: &str) -> String {
    format!(
        "schemaVersion: 1\nname: {name}\nenabled: true\nhosts: {hosts}\n\
         trigger: {{ cron: \"* * * * *\" }}\ntarget: {target}\n"
    )
}

#[test]
fn run_sweep_at_dry_run_discovers_loads_and_fails_closed() {
    let tmp = tempdir().unwrap();
    let global = tmp.path().join("global");
    let ws_root = tmp.path().join("polaris");
    let ws_orbit = ws_root.join(".orbit");
    fs::create_dir_all(global.join("state")).unwrap();
    fs::create_dir_all(ws_orbit.join("routines")).unwrap();
    fs::create_dir_all(ws_orbit.join("resources/jobs")).unwrap();

    fs::write(global.join("host.toml"), format!("host_id = \"{HOST}\"\n")).unwrap();
    fs::write(
        ws_orbit.join("config.toml"),
        "[routines]\nrole = \"source\"\n",
    )
    .unwrap();
    fs::write(ws_orbit.join("resources/jobs/noop.yaml"), NOOP_JOB).unwrap();

    let ph = format!("[{HOST}]");
    fs::write(
        ws_orbit.join("routines/minutely.yaml"),
        source_routine_yaml("qa-minutely", "job:noop", &ph),
    )
    .unwrap();
    fs::write(
        ws_orbit.join("routines/otherhost.yaml"),
        source_routine_yaml("qa-otherhost", "job:noop", "[dk-server-1]"),
    )
    .unwrap();
    fs::write(
        ws_orbit.join("routines/badtarget.yaml"),
        source_routine_yaml("qa-bad", "job:does_not_exist", &ph),
    )
    .unwrap();
    fs::write(
        ws_orbit.join("routines/activity.yaml"),
        source_routine_yaml("qa-activity", "activity:worktree_setup", &ph),
    )
    .unwrap();

    let mut registry = WorkspaceRegistry::default();
    registry.workspaces.push(Workspace {
        id: "ws-1".to_string(),
        name: "polaris".to_string(),
        root: ws_root.clone(),
        orbit_dir: ws_orbit.clone(),
        git_remote: None,
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    });
    workspace_registry::save_registry_to(
        &registry,
        &workspace_registry::registry_path_for(&global),
    )
    .unwrap();

    let outcome = run_sweep_at(&global, SweepOptions { dry_run: true }).expect("sweep ok");

    assert_eq!(outcome.host_id, HOST);
    assert!(!outcome.lock_busy);

    let report = |name: &str| outcome.reports.iter().find(|r| r.routine == name);
    // Valid, pinned routine: first observation -> would_baseline (dry-run).
    assert_eq!(
        report("qa-minutely").map(|r| r.action),
        Some("would_baseline")
    );
    // Pinned elsewhere -> skipped.
    assert_eq!(
        report("qa-otherhost")
            .and_then(|r| r.reason.clone())
            .as_deref(),
        Some("host_not_pinned")
    );
    // Fail-closed load errors, not fires: unresolvable target + reserved activity.
    assert!(
        outcome
            .load_errors
            .iter()
            .any(|e| e.message.contains("does not resolve")),
        "unresolvable target is a load error"
    );
    assert!(
        outcome
            .load_errors
            .iter()
            .any(|e| e.message.contains("not dispatchable")),
        "activity target rejected at parse"
    );
    // dry-run recorded nothing.
    let store = Store::open(&global.join("orbit.db")).unwrap();
    assert!(store.routine_cursor("qa-minutely").unwrap().is_none());
}
