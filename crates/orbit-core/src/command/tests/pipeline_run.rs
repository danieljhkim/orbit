use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

use chrono::Utc;
use orbit_common::types::{AuditEventStatus, JobRunState};
use tempfile::TempDir;

use crate::OrbitRuntime;
use crate::command::job::JobRunListParams;
use crate::command::pipeline_run::{
    configure_pipeline_worker_command, resolve_pipeline_worker_executable,
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
    let child = Command::new("sh")
        .args(["-c", "exit 23"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn failing worker fixture");

    runtime
        .monitor_pipeline_worker_startup(
            &run.run_id,
            child,
            &runtime.paths().repo_root,
            Some("test"),
        )
        .expect("observe worker exit");

    let stored = runtime.show_job_run(&run.run_id).expect("show failed run");
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
