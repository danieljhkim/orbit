use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

use chrono::Utc;
use orbit_common::types::{AuditEventStatus, JobRunState};
use tempfile::TempDir;

use crate::OrbitRuntime;
use crate::command::pipeline_run::{
    configure_pipeline_worker_command, resolve_pipeline_worker_executable,
};

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
        .insert_run("task_gate_pipeline", 1, Utc::now(), None, None)
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
