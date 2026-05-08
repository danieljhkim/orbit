use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{
    ExecutorDef, ExecutorSandboxKind, ExecutorType, Task, TaskPriority, TaskStatus, TaskType,
};
use tempfile::tempdir;

use crate::OrbitRuntime;
use crate::command::task::TaskAddParams;

pub(crate) fn seed_executor(
    runtime: &OrbitRuntime,
    name: &str,
    sandbox: Option<ExecutorSandboxKind>,
) {
    let now = Utc::now();
    runtime
        .upsert_executor_def(&ExecutorDef {
            name: name.to_string(),
            executor_type: ExecutorType::DirectAgent,
            command: Some(name.to_string()),
            args: vec!["exec".to_string(), "--json".to_string()],
            stdout_format: None,
            models: HashMap::new(),
            timeout_seconds: None,
            env: HashMap::new(),
            sandbox,
            allow_fallback: false,
            created_at: now,
            updated_at: now,
        })
        .expect("seed executor");
}

pub(crate) fn seeded_runtime_with_executor(sandbox: Option<ExecutorSandboxKind>) -> OrbitRuntime {
    let runtime = OrbitRuntime::in_memory().expect("build runtime");
    seed_executor(&runtime, "codex", sandbox);
    runtime
}

pub(crate) fn runtime_with_workspace_layout() -> (tempfile::TempDir, OrbitRuntime, PathBuf) {
    let root = tempdir().expect("create tempdir");
    let global = root.path().join("home/.orbit");
    let workspace = root.path().join("repo/.orbit");
    std::fs::create_dir_all(&global).expect("global orbit dir");
    std::fs::create_dir_all(&workspace).expect("workspace orbit dir");
    let runtime = OrbitRuntime::from_roots(&global, &workspace).expect("build runtime");
    let repo_root = root.path().join("repo");
    (root, runtime, repo_root)
}

pub(crate) fn write_workspace_file(repo_root: &Path, relative_path: &str) {
    let path = repo_root.join(relative_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dir");
    }
    std::fs::write(
        path,
        "test fixture
",
    )
    .expect("write workspace file");
}

pub(crate) fn seed_list_backlog_task(
    runtime: &OrbitRuntime,
    title: &str,
    status: TaskStatus,
    priority: TaskPriority,
    task_type: TaskType,
    parent_id: Option<String>,
    context_files: Vec<&str>,
) -> Task {
    runtime
        .add_task(TaskAddParams {
            parent_id,
            title: title.to_string(),
            description: format!("Fixture task: {title}"),
            acceptance_criteria: vec!["Fixture task is observable.".to_string()],
            plan: "Fixture plan.".to_string(),
            context_files: context_files.into_iter().map(str::to_string).collect(),
            workspace_path: Some(".".to_string()),
            priority,
            task_type: Some(task_type),
            status: Some(status),
            ..Default::default()
        })
        .expect("seed task")
}

pub(crate) fn seed_accepted_friction_task(
    runtime: &OrbitRuntime,
    title: &str,
    priority: TaskPriority,
    context_files: Vec<&str>,
) -> Task {
    let report = seed_list_backlog_task(
        runtime,
        title,
        TaskStatus::Friction,
        priority,
        TaskType::Friction,
        None,
        context_files,
    );
    runtime
        .approve_task(
            &report.id,
            Some("Accepted friction report.".to_string()),
            None,
        )
        .expect("accept friction task")
}
