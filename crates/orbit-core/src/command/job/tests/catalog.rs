use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{JobRunState, PipelineState};
use serde_json::json;
use tempfile::tempdir;

use crate::OrbitRuntime;

fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf, PathBuf) {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
    (root, runtime, global_root, workspace_root)
}

fn write_empty_job(path: &Path, name: &str) {
    let yaml = format!(
        r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  max_active_runs: 1
  steps: []
"#
    );
    std::fs::create_dir_all(path.parent().expect("job path has parent")).expect("create job dir");
    std::fs::write(path, yaml).expect("write job yaml");
}

fn write_shell_marker_job(path: &Path, name: &str, marker_path: &Path) {
    let marker_arg = yaml_single_quoted(&marker_path.to_string_lossy());
    let yaml = format!(
        r#"schemaVersion: 2
kind: Job
metadata:
  name: {name}
spec:
  state: enabled
  kind: workflow
  max_active_runs: 1
  steps:
    - id: exploit
      spec:
        type: shell
        program: /bin/sh
        args:
          - -c
          - 'printf pwned > "$1"'
          - sh
          - {marker_arg}
        allowed_programs:
          - /bin/sh
"#
    );
    std::fs::create_dir_all(path.parent().expect("job path has parent")).expect("create job dir");
    std::fs::write(path, yaml).expect("write job yaml");
}

fn yaml_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[test]
fn workspace_default_named_shell_job_does_not_run_when_workflow_invoked_by_name() {
    let (_root, runtime, global_root, workspace_root) = test_runtime();
    let job_name = "task_auto_pipeline";
    let global_job = global_root.join("resources/jobs/task_auto_pipeline.yaml");
    let workspace_job = workspace_root.join("resources/jobs/task_auto_pipeline.yaml");
    let marker_path = workspace_root.join("shell-shadow-marker");
    write_empty_job(&global_job, job_name);
    write_shell_marker_job(&workspace_job, job_name, &marker_path);

    let run = runtime
        .stores()
        .jobs()
        .insert_run(job_name, 1, Utc::now(), Some(json!({})), None)
        .expect("insert named pipeline run");
    runtime
        .stores()
        .jobs()
        .write_run_state(
            &run.run_id,
            &PipelineState::new(run.run_id.clone(), run.job_id.clone(), json!({})),
        )
        .expect("write initial pipeline state");

    runtime
        .execute_pipeline_run_worker(&run.run_id)
        .expect("execute named pipeline run");

    assert!(
        !marker_path.exists(),
        "workspace-local shell job shadow must not execute"
    );
    let finished = runtime
        .show_job_run(&run.run_id)
        .expect("show finished run");
    assert_eq!(finished.state, JobRunState::Success);
}
