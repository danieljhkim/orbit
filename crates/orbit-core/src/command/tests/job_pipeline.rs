use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;
use orbit_common::types::{AuditEventStatus, JobRunState, OrbitError};
use orbit_store::sqlite::migration::SUPPORTED_SCHEMA_VERSION;
use tempfile::TempDir;

use crate::OrbitRuntime;
use crate::command::job::JobRunListParams;
use crate::command::job::pipeline::{
    configure_pipeline_worker_command, configure_pipeline_worker_stdio, pipeline_worker_log_path,
    resolve_pipeline_worker_executable,
};
use crate::command::task::TaskAddParams;
use crate::command::workflow::ShipMode;

fn test_runtime() -> (TempDir, OrbitRuntime) {
    let root = TempDir::new().expect("tempdir");
    let global_root = root.path().join("global");
    let workspace_root = root.path().join("repo/.orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
    (root, runtime)
}

fn add_backlog_task(runtime: &OrbitRuntime) -> String {
    runtime
        .add_task(TaskAddParams {
            title: "Ship submission fixture".to_string(),
            description: "A task selected by a ship-submission test.".to_string(),
            ..Default::default()
        })
        .expect("create backlog task")
        .id
}

#[test]
fn pipeline_worker_command_discovers_registered_workspace_from_cwd() {
    let workspace = Path::new("/registered/workspace");
    let mut command = Command::new("orbit");

    configure_pipeline_worker_command(&mut command, workspace, "jrun-child");

    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        vec![
            OsStr::new("job"),
            OsStr::new("run-pipeline-worker"),
            OsStr::new("jrun-child"),
        ],
        "an explicit --root pins the worker to the wrong global store"
    );
    assert_eq!(command.get_current_dir(), Some(workspace));
}

#[cfg(unix)]
#[test]
fn worker_exit_before_claim_terminalizes_persisted_run_with_diagnostic() {
    let (_root, runtime) = test_runtime();
    let run = runtime
        .stores()
        .jobs()
        .insert_job_run("task_gate_pipeline", 1, Utc::now(), None, None)
        .expect("insert pending run");
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "printf 'worker stdout context\\n'; \
         printf 'action registration missing: routine_dispatch\\n' >&2; \
         exit 23",
    ]);
    let log_path =
        configure_pipeline_worker_stdio(&mut command, &runtime.paths().logs_dir, &run.run_id)
            .expect("configure worker log");

    runtime
        .spawn_pipeline_worker_process(&run.run_id, Some("test"), command, log_path.clone())
        .expect("spawn detached failing worker fixture");

    let stored = wait_for_worker_ownership_outcome(&runtime, &run.run_id);
    assert_eq!(stored.state, JobRunState::Interrupted);
    assert!(stored.finished_at.is_some());
    assert!(stored.pid.is_none());
    let diagnostic = stored.steps.last().expect("startup diagnostic step");
    let message = diagnostic
        .error_message
        .as_deref()
        .expect("startup diagnostic message");
    assert!(
        message.contains("before claiming the persisted run"),
        "{message}"
    );
    assert!(message.contains("exit status: 23"), "{message}");
    assert!(message.contains("registered workspace"), "{message}");
    assert!(
        message.contains("action registration missing: routine_dispatch"),
        "{message}"
    );
    assert!(
        message.contains(&log_path.display().to_string()),
        "{message}"
    );

    assert_eq!(
        log_path,
        pipeline_worker_log_path(&runtime.paths().logs_dir, &run.run_id)
    );
    let durable_output = std::fs::read_to_string(&log_path).expect("read durable worker log");
    assert!(durable_output.contains("worker stdout context"));
    assert!(durable_output.contains("action registration missing: routine_dispatch"));

    wait_for_pipeline_audit_event(
        &runtime,
        Some(AuditEventStatus::Failure),
        "startup failure audit",
        |audit| {
            audit.tool_name.as_deref() == Some("pipeline.worker.startup")
                && audit.target_id.as_deref() == Some(run.run_id.as_str())
                && audit
                    .error_message
                    .as_deref()
                    .is_some_and(|error| error.contains("before claiming"))
        },
    );
}

#[cfg(unix)]
#[test]
fn routine_style_detached_worker_is_claimed_within_ownership_window() {
    let (_root, runtime) = test_runtime();
    let run = runtime
        .stores()
        .jobs()
        .insert_job_run("auto_task_scheduler_pipeline", 1, Utc::now(), None, None)
        .expect("insert routine-dispatched run");
    let mut command = Command::new("sh");
    command.args(["-c", "printf 'routine worker startup\\n' >&2; sleep 0.25"]);
    let log_path =
        configure_pipeline_worker_stdio(&mut command, &runtime.paths().logs_dir, &run.run_id)
            .expect("configure routine worker log");

    let started = Instant::now();
    let worker_pid = runtime
        .spawn_pipeline_worker_process(
            &run.run_id,
            Some("routine-sweep"),
            command,
            log_path.clone(),
        )
        .expect("spawn detached routine worker fixture");
    runtime
        .stores()
        .jobs()
        .claim_pending_job_run_owner(&run.run_id, worker_pid)
        .expect("claim routine run");

    let stored = wait_for_worker_ownership_outcome(&runtime, &run.run_id);
    assert_eq!(stored.state, JobRunState::Pending);
    assert_eq!(stored.pid, Some(worker_pid));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "detached routine worker exceeded ownership window"
    );

    wait_for_pipeline_audit_event(&runtime, None, "claimed-worker audit", |audit| {
        audit.tool_name.as_deref() == Some("pipeline.worker.claimed")
            && audit.target_id.as_deref() == Some(run.run_id.as_str())
    });

    assert_eq!(
        log_path,
        pipeline_worker_log_path(&runtime.paths().logs_dir, &run.run_id)
    );
    let durable_output = wait_for_log_contains(&log_path, "routine worker startup");
    assert!(durable_output.contains("routine worker startup"));
}

fn wait_for_worker_ownership_outcome(
    runtime: &OrbitRuntime,
    run_id: &str,
) -> orbit_common::types::JobRun {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let stored = runtime.show_job_run(run_id).expect("show worker run");
        if stored.pid.is_some() || stored.state != JobRunState::Pending {
            return stored;
        }
        assert!(
            Instant::now() < deadline,
            "worker remained pending and unclaimed beyond ownership window"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// Retry a pipeline audit lookup instead of asserting on a single snapshot:
/// the audit event is written after the run's terminal state and diagnostic
/// step, so an observer that only waits for those can still race the audit.
fn wait_for_pipeline_audit_event(
    runtime: &OrbitRuntime,
    status: Option<AuditEventStatus>,
    description: &str,
    predicate: impl Fn(&orbit_common::types::AuditEvent) -> bool,
) -> orbit_common::types::AuditEvent {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let audits = runtime
            .list_audit_events(None, None, status, None, 20)
            .expect("list pipeline audit events");
        if let Some(audit) = audits.into_iter().find(|audit| predicate(audit)) {
            return audit;
        }
        assert!(
            Instant::now() < deadline,
            "expected {description} was not persisted within the window"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_log_contains(path: &Path, expected: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let output = std::fs::read_to_string(path).expect("read durable worker log");
        if output.contains(expected) {
            return output;
        }
        assert!(
            Instant::now() < deadline,
            "worker log did not contain expected output: {expected}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn long_lived_worker_reopens_and_applies_compatible_pending_schema() {
    let (_root, runtime) = test_runtime();
    let store = runtime.sqlite_store().expect("open store fixture");
    {
        let connection = store.connection();
        let conn = connection.lock().expect("store connection");
        // Rewind past the newest ledger entry as well: the recorded version is
        // the highest key present, so leaving a later key behind would report
        // the store as current and apply nothing. Every entry from v0011 up is
        // therefore dropped, not a fixed list that a new migration would
        // silently outrun.
        conn.execute(
            "DELETE FROM schema_meta WHERE key LIKE 'migration.v%' AND key >= 'migration.v0011'",
            [],
        )
        .expect("rewind routine migration ledger");
        conn.execute_batch(
            "DROP TABLE routine_pauses;
             DROP TABLE routine_fires;
             DROP TABLE routine_cursors;
             DROP TABLE friction_import_state;
             DROP TABLE friction_record_tags;
             DROP TABLE friction_records;",
        )
        .expect("rewind routine schema");
    }

    runtime
        .preflight_pipeline_worker_store()
        .expect("compatible worker preflight");

    assert_eq!(
        store.schema_version().expect("schema after preflight"),
        SUPPORTED_SCHEMA_VERSION
    );
    let connection = store.connection();
    let conn = connection.lock().expect("store connection");
    let restored_tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name IN (
                 'routine_cursors', 'routine_fires', 'routine_pauses',
                 'friction_records', 'friction_record_tags', 'friction_import_state'
             )",
            [],
            |row| row.get(0),
        )
        .expect("count restored tables");
    assert_eq!(restored_tables, 6);
}

#[test]
fn newer_schema_fails_before_worker_claims_or_executes() {
    let (_root, runtime) = test_runtime();
    let run = runtime
        .stores()
        .jobs()
        .insert_job_run("task_gate_pipeline", 1, Utc::now(), None, None)
        .expect("insert pending run");
    let store = runtime.sqlite_store().expect("open store fixture");
    {
        let connection = store.connection();
        let conn = connection.lock().expect("store connection");
        conn.execute(
            "INSERT INTO schema_meta(key, value, updated_at)
             VALUES (?1, 'future_schema', '2099-01-01T00:00:00Z')",
            [format!(
                "migration.v{:04}",
                SUPPORTED_SCHEMA_VERSION.saturating_add(1)
            )],
        )
        .expect("advance store beyond worker");
    }

    let error = runtime
        .execute_pipeline_run_worker(&run.run_id)
        .expect_err("newer schema must fail worker preflight");
    assert!(matches!(error, OrbitError::Migration(_)), "{error:?}");

    let stored = runtime.show_job_run(&run.run_id).expect("show pending run");
    assert_eq!(stored.state, JobRunState::Pending);
    assert_eq!(stored.pid, None);
    assert!(stored.steps.is_empty());
}

#[test]
fn existing_pipeline_worker_executable_path_is_preserved() {
    let dir = TempDir::new().expect("tempdir");
    let executable = dir.path().join("orbit (deleted)");
    std::fs::write(&executable, "replacement").expect("write executable fixture");

    assert_eq!(
        resolve_pipeline_worker_executable(executable.clone()),
        executable
    );
}

#[cfg(target_os = "linux")]
#[test]
fn deleted_current_executable_resolves_to_replaced_installed_path() {
    let dir = TempDir::new().expect("tempdir");
    let installed = dir.path().join("orbit");
    std::fs::write(&installed, "replacement").expect("write replacement executable");
    let deleted_inode_path = installed.with_file_name("orbit (deleted)");

    assert!(
        !deleted_inode_path.exists(),
        "the kernel-style deleted-inode pseudo-path must be absent"
    );
    assert_eq!(
        resolve_pipeline_worker_executable(deleted_inode_path),
        installed,
        "the worker must launch through the replacement at the installed path"
    );
}

/// ORB-10544: the duplicate-dispatch guard lives in the shared submission path,
/// so it cannot be bypassed by a future adapter that calls `submit_ship_run`
/// directly instead of going through the dashboard endpoint or the MCP tool.
/// Asserted here against the shared entry point itself, with no HTTP or tool
/// surface in the picture.
#[test]
fn ship_submission_refuses_a_task_already_carried_by_a_non_terminal_run() {
    let (_root, runtime) = test_runtime();
    let selected_task_id = add_backlog_task(&runtime);
    let in_flight = runtime
        .stores()
        .jobs()
        .insert_job_run(
            "task_auto_pipeline",
            1,
            Utc::now(),
            Some(serde_json::json!({"mode": "local", "task_ids": [selected_task_id]})),
            None,
        )
        .expect("insert in-flight run");
    assert!(!in_flight.state.is_terminal());

    let error = runtime
        .submit_ship_run(
            ShipMode::Local,
            Some("main"),
            std::slice::from_ref(&selected_task_id),
            Some("test"),
            None,
        )
        .expect_err("a task with a run in flight must not dispatch a second run");

    let OrbitError::ShipRunInFlight {
        task_id: guarded_task_id,
        run_id,
    } = &error
    else {
        panic!("expected ShipRunInFlight, got {error:?}");
    };
    assert_eq!(guarded_task_id, &selected_task_id);
    assert_eq!(run_id, &in_flight.run_id);

    let runs = runtime
        .list_job_runs(JobRunListParams::default())
        .expect("list job runs");
    assert_eq!(
        runs.iter().map(|run| &run.run_id).collect::<Vec<_>>(),
        vec![&in_flight.run_id],
        "the refused submission must not persist another run"
    );
}

/// The shared guard is keyed on the explicit selection: an unrelated task is
/// still shippable while another one is in flight, and auto (backlog-discovery)
/// mode — which names no tasks — is never keyed and so never refused.
///
/// Neither call is expected to dispatch: this fixture seeds no job asset, so
/// both fall through the guard to the same job-not-found refusal. That the
/// refusal is *not* `ShipRunInFlight` is exactly the assertion, and it keeps the
/// test from spawning a detached pipeline worker.
#[test]
fn ship_submission_guard_is_scoped_to_the_selected_tasks() {
    let (_root, runtime) = test_runtime();
    let in_flight_task_id = add_backlog_task(&runtime);
    let unrelated_task_id = add_backlog_task(&runtime);
    runtime
        .stores()
        .jobs()
        .insert_job_run(
            "task_auto_pipeline",
            1,
            Utc::now(),
            Some(serde_json::json!({"mode": "local", "task_ids": [in_flight_task_id]})),
            None,
        )
        .expect("insert in-flight run");

    for (label, task_ids) in [
        ("an unrelated explicit task", vec![unrelated_task_id]),
        ("auto discovery", Vec::new()),
    ] {
        let error = runtime
            .submit_ship_run(ShipMode::Local, Some("main"), &task_ids, Some("test"), None)
            .expect_err("no job asset is deployed in this fixture");
        assert!(
            !matches!(error, OrbitError::ShipRunInFlight { .. }),
            "{label} must pass the in-flight guard: {error:?}"
        );
        assert!(
            matches!(error, OrbitError::NotFound { .. }),
            "{label} must fail on the missing job asset instead: {error:?}"
        );
    }
}

/// Explicit task validation belongs in the shared runtime path, ahead of
/// pipeline persistence, so every submission surface reports a typo directly
/// and cannot leave an orphaned worker/run behind.
#[test]
fn ship_submission_refuses_a_missing_explicit_task_before_persisting_a_run() {
    let (_root, runtime) = test_runtime();
    let missing_id = "ORB-99999".to_string();

    let error = runtime
        .submit_ship_run(
            ShipMode::Local,
            Some("main"),
            std::slice::from_ref(&missing_id),
            Some("test"),
            None,
        )
        .expect_err("a missing explicit task must be rejected before dispatch");

    assert!(matches!(
        error,
        OrbitError::NotFound {
            kind: orbit_common::types::NotFoundKind::Task,
            id,
        } if id == missing_id
    ));
    assert!(
        runtime
            .list_job_runs(JobRunListParams::default())
            .expect("list job runs")
            .is_empty(),
        "the refusal must not persist a run or spawn a worker"
    );
}

#[test]
fn ship_submission_mixed_explicit_selection_identifies_the_missing_task() {
    let (_root, runtime) = test_runtime();
    let existing_id = add_backlog_task(&runtime);
    let missing_id = "ORB-99999".to_string();

    let error = runtime
        .submit_ship_run(
            ShipMode::Local,
            Some("main"),
            &[existing_id, missing_id.clone()],
            Some("test"),
            None,
        )
        .expect_err("mixed selections must refuse their missing task before dispatch");

    assert!(matches!(
        error,
        OrbitError::NotFound {
            kind: orbit_common::types::NotFoundKind::Task,
            id,
        } if id == missing_id
    ));
    assert!(
        runtime
            .list_job_runs(JobRunListParams::default())
            .expect("list job runs")
            .is_empty(),
        "mixed-selection refusal must not persist a run"
    );
}
