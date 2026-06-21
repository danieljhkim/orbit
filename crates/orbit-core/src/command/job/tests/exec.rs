use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use orbit_common::types::{ExecutorDef, ExecutorType, JobRunState, TaskStatus};
use serde_json::json;
use tempfile::tempdir;

use crate::OrbitRuntime;
use crate::command::activity::seed_default_activities;
use crate::command::task::TaskAddParams;

fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf) {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    fs::create_dir_all(&global_root).expect("create global root");
    fs::create_dir_all(&workspace_root).expect("create workspace root");
    seed_default_activities(&global_root.join("resources/activities"), true)
        .expect("seed default activities");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
    (root, runtime, repo_root)
}

fn init_git_repo(repo: &Path) {
    git(repo, &["init"]);
    git(repo, &["checkout", "-b", "main"]);
    git(repo, &["config", "user.name", "Orbit Test"]);
    git(repo, &["config", "user.email", "orbit-test@example.com"]);
    fs::write(repo.join("README.md"), "test repo\n").expect("write README");
    git(repo, &["add", "README.md"]);
    git(repo, &["commit", "-m", "initial commit"]);
}

fn git(current_dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(current_dir)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {} failed in {}:\nstdout: {}\nstderr: {}",
        args.join(" "),
        current_dir.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
fn write_executable(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, contents).expect("write script");
    let mut permissions = fs::metadata(path).expect("script metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("script permissions");
}

fn add_backlog_task(runtime: &OrbitRuntime) -> String {
    let task = runtime
        .add_task(TaskAddParams {
            title: "Provider auth rollback fixture".to_string(),
            description: "Exercise failed run rollback behavior.".to_string(),
            workspace_path: Some(".".to_string()),
            ..TaskAddParams::default()
        })
        .expect("add task");
    runtime
        .approve_task(&task.id, Some("approve for workflow".to_string()), None)
        .expect("approve task");
    task.id
}

fn upsert_claude_executor(runtime: &OrbitRuntime, command: &Path) {
    let now = Utc::now();
    runtime
        .upsert_executor_def(&ExecutorDef {
            name: "claude".to_string(),
            executor_type: ExecutorType::DirectAgent,
            command: Some(command.display().to_string()),
            args: Vec::new(),
            stdout_format: None,
            model_pair_override: None,
            model_flag: None,
            timeout_seconds: None,
            env: Default::default(),
            sandbox: None,
            allow_fallback: false,
            created_at: now,
            updated_at: now,
        })
        .expect("seed claude executor");
}

fn write_auth_failure_job(path: &Path) {
    let yaml = r#"schemaVersion: 2
kind: Job
metadata:
  name: qa_provider_auth_pipeline
spec:
  state: enabled
  kind: workflow
  steps:
    - id: worktree
      target: activity:worktree_setup
      default_input:
        task_ids: "{{ input.task_ids }}"
        base: main
        base_sync: local
        branch_prefix: orbit-test
    - id: implement_one
      spec:
        type: agent_loop
        instruction: "exercise provider auth classification"
        tools: []
        on_denial: terminate
        max_iterations: 1
        model: claude-opus-4-7
        backend: cli
        provider: claude
        wall_clock_timeout_seconds: 30
      default_input:
        task_id: "{{ input.task_id }}"
        workspace_path: "{{ steps.worktree.output.workspace_path }}"
"#;
    fs::write(path, yaml).expect("write job");
}

fn auth_failure_stdout() -> &'static str {
    r#"{"type":"result","subtype":"error","is_error":true,"api_error_status":401,"result":"Failed to authenticate. API Error: 401 Invalid authentication credentials","usage":{"input_tokens":0,"output_tokens":0}}"#
}

#[cfg(unix)]
#[test]
fn provider_auth_failure_persists_error_code_and_rolls_back_pre_work_admission() {
    let (_root, runtime, repo_root) = test_runtime();
    init_git_repo(&repo_root);
    let task_id = add_backlog_task(&runtime);
    let claude = repo_root.join("claude");
    write_executable(
        &claude,
        &format!(
            "#!/bin/sh\ncat > /dev/null\nprintf '%s\\n' '{}'\nexit 1\n",
            auth_failure_stdout()
        ),
    );
    upsert_claude_executor(&runtime, &claude);
    let yaml_path = repo_root.join("qa_provider_auth_pipeline.yaml");
    write_auth_failure_job(&yaml_path);

    let result = runtime
        .run_job_v2_from_yaml(
            &yaml_path,
            json!({"task_id": task_id.clone(), "task_ids": [task_id.clone()]}),
            None,
        )
        .expect("job records a failed result");

    assert!(!result.success);
    assert_eq!(result.error_code.as_deref(), Some("provider_auth"));
    let run = runtime.show_job_run(&result.run_id).expect("show run");
    assert_eq!(run.state, JobRunState::Failed);
    assert!(run.steps.iter().any(|step| {
        step.error_code.as_deref() == Some("provider_auth")
            && step
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("credentials"))
    }));

    let task = runtime.get_task(&task_id).expect("reload task");
    assert_eq!(task.status, TaskStatus::Backlog);
    assert_eq!(task.job_run_id, None);
    let history = runtime.get_task_history(&task_id).expect("task history");
    assert!(history.iter().any(|entry| {
        entry.event == "workflow_admission_rolled_back"
            && entry.from_status == Some(TaskStatus::InProgress)
            && entry.to_status == Some(TaskStatus::Backlog)
    }));
}

#[cfg(unix)]
#[test]
fn provider_auth_failure_after_committed_work_does_not_roll_back_task() {
    let (_root, runtime, repo_root) = test_runtime();
    init_git_repo(&repo_root);
    let task_id = add_backlog_task(&runtime);
    let claude = repo_root.join("claude");
    write_executable(
        &claude,
        &format!(
            r#"#!/bin/sh
cat > /dev/null
printf '%s\n' 'post-work fixture' > post-work.txt
git add post-work.txt
git commit -m 'post-work fixture' >/dev/null
printf '%s\n' '{}'
exit 1
"#,
            auth_failure_stdout()
        ),
    );
    upsert_claude_executor(&runtime, &claude);
    let yaml_path = repo_root.join("qa_provider_auth_pipeline.yaml");
    write_auth_failure_job(&yaml_path);

    let result = runtime
        .run_job_v2_from_yaml(
            &yaml_path,
            json!({"task_id": task_id.clone(), "task_ids": [task_id.clone()]}),
            None,
        )
        .expect("job records a failed result");

    assert!(!result.success);
    let task = runtime.get_task(&task_id).expect("reload task");
    assert_eq!(task.status, TaskStatus::InProgress);
    assert_eq!(task.job_run_id.as_deref(), Some(result.run_id.as_str()));
    let history = runtime.get_task_history(&task_id).expect("task history");
    assert!(
        !history
            .iter()
            .any(|entry| entry.event == "workflow_admission_rolled_back")
    );
}
