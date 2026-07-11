//! Sibling tests for `command/doctor.rs` — workspace self-diagnostics [ORB-10005].

use std::fs;
use std::path::Path;

use chrono::Utc;
use orbit_common::types::{JobRun, JobRunState};

use orbit_core::OrbitRuntime;

use crate::doctor::{
    DoctorCommands, WorkspaceDoctorResult, WorkspaceDoctorStatus, collect_lock_files,
    disk_space_check, process_is_alive,
};

fn status_of<'a>(results: &'a [WorkspaceDoctorResult], name: &str) -> &'a WorkspaceDoctorResult {
    results
        .iter()
        .find(|row| row.check_name == name)
        .unwrap_or_else(|| panic!("check '{name}' missing from {results:?}"))
}

fn workspace_runtime(temp: &tempfile::TempDir) -> OrbitRuntime {
    let global_root = temp.path().join("global");
    let workspace_root = temp.path().join("repo").join(".orbit");
    fs::create_dir_all(&global_root).expect("create global root");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build runtime")
}

#[test]
fn healthy_fresh_workspace_has_no_failures() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let results = runtime.doctor_workspace().expect("doctor");

    assert_eq!(results.len(), 7, "one row per check: {results:?}");
    assert!(
        results
            .iter()
            .all(|row| row.status != WorkspaceDoctorStatus::Error),
        "fresh workspace must not fail any check: {results:?}"
    );
    assert_eq!(
        status_of(&results, "config").status,
        WorkspaceDoctorStatus::Ok
    );
    assert_eq!(
        status_of(&results, "database").status,
        WorkspaceDoctorStatus::Ok
    );
    // Absent subsystems degrade to skip, not error.
    assert_eq!(
        status_of(&results, "semantic-index").status,
        WorkspaceDoctorStatus::Skipped
    );
    assert_eq!(
        status_of(&results, "graph-index").status,
        WorkspaceDoctorStatus::Skipped
    );
    assert_eq!(
        status_of(&results, "stale-locks").status,
        WorkspaceDoctorStatus::Ok
    );
    assert_eq!(
        status_of(&results, "job-runs").status,
        WorkspaceDoctorStatus::Ok
    );
}

#[test]
fn invalid_config_fails_the_config_check() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime = workspace_runtime(&temp);

    // Written after runtime construction (an invalid config would fail the
    // bootstrap itself); doctor re-validates the effective file.
    fs::write(
        temp.path().join("repo").join(".orbit").join("config.toml"),
        "not = [valid toml",
    )
    .expect("write broken config");

    let results = runtime.doctor_workspace().expect("doctor");
    let config = status_of(&results, "config");
    assert_eq!(config.status, WorkspaceDoctorStatus::Error, "{config:?}");
    assert!(
        config.message.contains("invalid"),
        "message names the failure: {}",
        config.message
    );
}

#[test]
fn unopenable_store_database_fails_the_database_check() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime = workspace_runtime(&temp);

    // Make the store database path unopenable for the probe's fresh
    // connection. (Overwriting the file with garbage is not enough: the
    // runtime's live WAL still serves valid pages to new connections.)
    let db_path = temp.path().join("global").join("orbit.db");
    fs::remove_file(&db_path).expect("remove store db");
    fs::create_dir(&db_path).expect("block store db path");

    let results = runtime.doctor_workspace().expect("doctor");
    let database = status_of(&results, "database");
    assert_eq!(
        database.status,
        WorkspaceDoctorStatus::Error,
        "{database:?}"
    );
    assert!(
        database.message.contains("cannot open store database"),
        "message names the failure: {}",
        database.message
    );
}

#[cfg(unix)]
fn reaped_child_pid() -> u32 {
    let mut child = std::process::Command::new("true")
        .spawn()
        .expect("spawn child");
    let pid = child.id();
    child.wait().expect("reap child");
    pid
}

#[cfg(unix)]
fn write_holder_lock(path: &Path, pid: u32, label: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create lock dir");
    }
    fs::write(
        path,
        serde_json::to_string(&serde_json::json!({
            "pid": pid,
            "acquired_at": Utc::now().to_rfc3339(),
            "label": label,
        }))
        .expect("serialize holder"),
    )
    .expect("write lock file");
}

#[cfg(unix)]
#[test]
fn dead_holder_lock_file_is_reported_stale() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime = workspace_runtime(&temp);

    let lock_path = temp
        .path()
        .join("repo")
        .join(".orbit")
        .join("state")
        .join(".dead-holder.lock");
    write_holder_lock(&lock_path, reaped_child_pid(), "crashed op");

    let results = runtime.doctor_workspace().expect("doctor");
    let locks = status_of(&results, "stale-locks");
    assert_eq!(locks.status, WorkspaceDoctorStatus::Warning, "{locks:?}");
    assert!(
        locks.message.contains(".dead-holder.lock") && locks.message.contains("crashed op"),
        "message names the stale lock and its op: {}",
        locks.message
    );
}

#[cfg(unix)]
#[test]
fn live_holder_lock_file_is_not_stale() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime = workspace_runtime(&temp);

    let lock_path = temp
        .path()
        .join("repo")
        .join(".orbit")
        .join("state")
        .join(".live-holder.lock");
    write_holder_lock(&lock_path, std::process::id(), "live op");

    let results = runtime.doctor_workspace().expect("doctor");
    assert_eq!(
        status_of(&results, "stale-locks").status,
        WorkspaceDoctorStatus::Ok,
        "a live holder must not be reported stale"
    );
}

#[cfg(unix)]
#[test]
fn orphaned_running_run_is_reported() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let workspace_id = runtime.workspace_id().expect("workspace id");
    let now = Utc::now();
    let run = JobRun {
        run_id: "run-orphan".to_string(),
        job_id: "demo".to_string(),
        attempt: 1,
        state: JobRunState::Running,
        scheduled_at: now,
        started_at: Some(now),
        finished_at: None,
        duration_ms: None,
        created_at: now,
        // No recorded owner: classified `Missing` — conclusively orphaned.
        pid: None,
        pid_start_time: None,
        input: None,
        retry_source_run_id: None,
        knowledge_metrics: None,
        resolved_crew: None,
        crew_model: None,
        steps: Vec::new(),
    };
    runtime
        .sqlite_store()
        .expect("store")
        .upsert_job_run_for_workspace(&workspace_id, &run, None)
        .expect("seed running run");

    let results = runtime.doctor_workspace().expect("doctor");
    let job_runs = status_of(&results, "job-runs");
    assert_eq!(
        job_runs.status,
        WorkspaceDoctorStatus::Warning,
        "{job_runs:?}"
    );
    assert!(
        job_runs.message.contains("run-orphan"),
        "message names the orphaned run: {}",
        job_runs.message
    );
}

/// [ORB-10070] A `pending` run no worker ever claimed, old enough that the
/// claim grace window has passed, is reported as an orphan.
#[test]
fn orphaned_pending_run_is_reported() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let workspace_id = runtime.workspace_id().expect("workspace id");
    let created_at = Utc::now() - chrono::Duration::days(4);
    let run = JobRun {
        run_id: "run-pending-orphan".to_string(),
        job_id: "task_gate_pipeline".to_string(),
        attempt: 1,
        state: JobRunState::Pending,
        scheduled_at: created_at,
        started_at: None,
        finished_at: None,
        duration_ms: None,
        created_at,
        // Never claimed by a worker; far past the unclaimed grace window.
        pid: None,
        pid_start_time: None,
        input: None,
        retry_source_run_id: None,
        knowledge_metrics: None,
        resolved_crew: None,
        crew_model: None,
        steps: Vec::new(),
    };
    runtime
        .sqlite_store()
        .expect("store")
        .upsert_job_run_for_workspace(&workspace_id, &run, None)
        .expect("seed pending run");

    let results = runtime.doctor_workspace().expect("doctor");
    let job_runs = status_of(&results, "job-runs");
    assert_eq!(
        job_runs.status,
        WorkspaceDoctorStatus::Warning,
        "{job_runs:?}"
    );
    assert!(
        job_runs.message.contains("run-pending-orphan"),
        "message names the orphaned pending run: {}",
        job_runs.message
    );
    assert!(
        job_runs
            .message
            .contains("pending run(s) with no live worker"),
        "message explains the pending orphan class: {}",
        job_runs.message
    );
}

/// A freshly queued run inside the claim grace window is healthy, not an orphan.
#[test]
fn fresh_pending_run_is_not_reported_as_orphan() {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    let workspace_id = runtime.workspace_id().expect("workspace id");
    let now = Utc::now();
    let run = JobRun {
        run_id: "run-pending-fresh".to_string(),
        job_id: "task_gate_pipeline".to_string(),
        attempt: 1,
        state: JobRunState::Pending,
        scheduled_at: now,
        started_at: None,
        finished_at: None,
        duration_ms: None,
        created_at: now,
        pid: None,
        pid_start_time: None,
        input: None,
        retry_source_run_id: None,
        knowledge_metrics: None,
        resolved_crew: None,
        crew_model: None,
        steps: Vec::new(),
    };
    runtime
        .sqlite_store()
        .expect("store")
        .upsert_job_run_for_workspace(&workspace_id, &run, None)
        .expect("seed pending run");

    let results = runtime.doctor_workspace().expect("doctor");
    let job_runs = status_of(&results, "job-runs");
    assert_eq!(job_runs.status, WorkspaceDoctorStatus::Ok, "{job_runs:?}");
}

#[cfg(unix)]
#[test]
fn process_liveness_probe_distinguishes_dead_from_live() {
    assert!(process_is_alive(std::process::id()));
    assert!(!process_is_alive(reaped_child_pid()));
}

#[test]
fn disk_space_check_reports_volume_numbers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let row = disk_space_check(temp.path());
    assert_eq!(row.check_name, "disk-space");
    assert!(
        row.message.contains("free of"),
        "message carries free/total detail: {}",
        row.message
    );
    assert_ne!(
        row.status,
        WorkspaceDoctorStatus::Skipped,
        "disk space is always determinable for an existing path"
    );
}

#[test]
fn collect_lock_files_scans_the_store_lock_locations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime = workspace_runtime(&temp);
    let paths = runtime.paths().clone();

    let expected = [
        paths.state_dir.join(".id_alloc.lock"),
        paths.tasks_dir.join(".ORB-00001.lock"),
        paths.learnings_dir.join(".L-0001.lock"),
        paths.adrs_dir.join(".locks").join("adr-0001.lock"),
    ];
    for path in &expected {
        fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
        fs::write(path, b"{}").expect("write lock file");
    }
    // Non-lock files are ignored.
    fs::write(paths.state_dir.join("notes.txt"), b"x").expect("write non-lock");

    let found = collect_lock_files(&paths);
    for path in &expected {
        assert!(
            found.contains(path),
            "missing {} in {found:?}",
            path.display()
        );
    }
    assert!(
        found
            .iter()
            .all(|path| path.file_name().is_some_and(|n| n != "notes.txt")),
        "non-lock files must be ignored: {found:?}"
    );
}

#[test]
fn graph_index_probe_skips_absent_and_reads_present() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime = workspace_runtime(&temp);

    assert!(
        runtime.health_check_graph_index().is_none(),
        "no graph dir yet → absent"
    );

    // A real (empty) SQLite database in the graph dir is readable.
    let graph_dir = temp.path().join("repo").join(".orbit").join("graph");
    fs::create_dir_all(&graph_dir).expect("create graph dir");
    let db_path = graph_dir.join("main.4.db");
    rusqlite::Connection::open(&db_path).expect("create graph db");
    let probe = runtime
        .health_check_graph_index()
        .expect("index present")
        .expect("index readable");
    assert!(probe.contains("main.4.db"), "probe names the db: {probe}");

    // Garbage in the (sole, hence newest) db is a probe failure, not a panic.
    fs::remove_file(&db_path).expect("remove healthy db");
    let garbage = graph_dir.join("garbage.4.db");
    fs::write(&garbage, b"garbage bytes, not sqlite").expect("write garbage db");
    let probe = runtime.health_check_graph_index().expect("index present");
    assert!(probe.is_err(), "garbage db must fail the probe: {probe:?}");
}
