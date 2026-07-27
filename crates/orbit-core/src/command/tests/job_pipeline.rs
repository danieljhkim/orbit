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

fn review_test_runtime() -> (TempDir, OrbitRuntime) {
    let root = TempDir::new().expect("tempdir");
    let global_root = root.path().join("global");
    let workspace_root = root.path().join("repo/.orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    std::fs::write(
        workspace_root.join("config.toml"),
        r#"
[workflow]
base_branch = "main"
default_crew = "sol"

[crews.sol]
model = "gpt-5.6-sol"
provider = "codex"
backend = "cli"

[crews.opus]
model = "opus"
provider = "claude"
backend = "cli"

[crews.opus_alias]
model = "opus"
provider = "claude"
backend = "cli"

[crews.opus_vendor_alias]
model = "opus"
provider = "anthropic"
backend = "cli"

[crews.unmaterializable]
model = "mystery"
provider = "not-a-provider"
backend = "cli"
"#,
    )
    .expect("write config");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
    (root, runtime)
}

fn add_review_test_task(runtime: &OrbitRuntime, crew: &str) -> String {
    runtime
        .add_task(TaskAddParams {
            title: "review preflight fixture".to_string(),
            description: "prove review validation occurs before submission".to_string(),
            plan: "validate without implementation side effects".to_string(),
            status: Some(orbit_common::types::TaskStatus::Backlog),
            crew: Some(crew.to_string()),
            ..TaskAddParams::default()
        })
        .expect("add task")
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

    let audits = runtime
        .list_audit_events(None, None, Some(AuditEventStatus::Failure), None, 20)
        .expect("list startup failure audit");
    assert!(audits.iter().any(|audit| {
        audit.tool_name.as_deref() == Some("pipeline.worker.startup")
            && audit.target_id.as_deref() == Some(run.run_id.as_str())
            && audit
                .error_message
                .as_deref()
                .is_some_and(|error| error.contains("before claiming"))
    }));
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

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let audits = runtime
            .list_audit_events(None, None, None, None, 20)
            .expect("list claimed-worker audit");
        if audits.iter().any(|audit| {
            audit.tool_name.as_deref() == Some("pipeline.worker.claimed")
                && audit.target_id.as_deref() == Some(run.run_id.as_str())
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "ownership observer did not persist a claimed audit"
        );
        thread::sleep(Duration::from_millis(10));
    }

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
        conn.execute("DELETE FROM schema_meta WHERE key = 'migration.v0011'", [])
            .expect("rewind routine migration ledger");
        conn.execute_batch(
            "DROP TABLE routine_pauses;
             DROP TABLE routine_fires;
             DROP TABLE routine_cursors;",
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
    let routine_tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name IN (
                 'routine_cursors', 'routine_fires', 'routine_pauses'
             )",
            [],
            |row| row.get(0),
        )
        .expect("count routine tables");
    assert_eq!(routine_tables, 3);
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

#[test]
fn review_submission_rejects_unknown_same_or_unmaterializable_crews_before_run_insert() {
    for (review_crew, expected) in [
        ("missing", "is not defined"),
        ("sol", "is not independent"),
        ("opus_alias", "is not independent"),
        ("opus_vendor_alias", "is not independent"),
        ("unmaterializable", "unmaterializable provider"),
    ] {
        let (_root, runtime) = review_test_runtime();
        let implementation_crew = if matches!(review_crew, "opus_alias" | "opus_vendor_alias") {
            "opus"
        } else {
            "sol"
        };
        let task_id = add_review_test_task(&runtime, implementation_crew);

        let error = runtime
            .submit_ship_run(
                ShipMode::Pr,
                Some("main"),
                &[task_id],
                true,
                Some(review_crew),
                Some("test"),
            )
            .expect_err("invalid review crew must fail before submission");

        assert!(
            error.to_string().contains(expected),
            "unexpected error for {review_crew}: {error}"
        );
        assert!(
            runtime
                .list_job_runs(JobRunListParams::default())
                .expect("list job runs")
                .is_empty(),
            "validation failure must not persist an implementation run"
        );
    }
}

#[test]
fn review_submission_fails_closed_when_deployed_review_assets_are_missing() {
    let (_root, runtime) = review_test_runtime();
    let task_id = add_review_test_task(&runtime, "sol");

    let error = runtime
        .submit_ship_run(
            ShipMode::Pr,
            Some("main"),
            &[task_id],
            true,
            Some("opus"),
            Some("test"),
        )
        .expect_err("missing review assets must fail before submission");

    assert!(error.to_string().contains("job not found"), "{error}");
    assert!(
        runtime
            .list_job_runs(JobRunListParams::default())
            .expect("list job runs")
            .is_empty()
    );
}
