use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use orbit_common::types::{OrbitError, Task, TaskPriority, TaskStatus, TaskType};
use orbit_store::TaskCreateParams;
use tempfile::tempdir;

use crate::OrbitRuntime;

pub(crate) fn test_runtime() -> (tempfile::TempDir, OrbitRuntime, PathBuf) {
    let root = tempdir().expect("create tempdir");
    let global_root = root.path().join("global");
    let repo_root = root.path().join("repo");
    let workspace_root = repo_root.join(".orbit");
    std::fs::create_dir_all(&global_root).expect("create global root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let runtime =
        OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
    (root, runtime, repo_root)
}

pub(super) fn create_task(
    runtime: &OrbitRuntime,
    workspace_path: &Path,
    title: &str,
    description: &str,
    status: TaskStatus,
    context_files: &[&str],
) -> Task {
    runtime
        .stores()
        .tasks()
        .create(TaskCreateParams {
            actor: "test".to_string(),
            parent_id: None,
            title: title.to_string(),
            description: description.to_string(),
            acceptance_criteria: Vec::new(),
            dependencies: Vec::new(),
            relations: Vec::new(),
            tags: Vec::new(),
            plan: String::new(),
            execution_summary: String::new(),
            context_files: context_files
                .iter()
                .map(|path| (*path).to_string())
                .collect(),
            workspace_path: Some(workspace_path.to_string_lossy().into_owned()),
            repo_root: None,
            created_by: Some("test".to_string()),
            planned_by: None,
            implemented_by: None,
            status,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Chore,
            external_refs: Vec::new(),
            source_task_id: None,
            crew: None,
            comments: Vec::new(),
        })
        .expect("create task")
}

pub(super) fn create_context_task(
    runtime: &OrbitRuntime,
    workspace_path: &Path,
    status: TaskStatus,
    context_files: &[&str],
) -> Task {
    create_task(
        runtime,
        workspace_path,
        "test task",
        "test",
        status,
        context_files,
    )
}

pub(super) fn invalid_input_message<T>(result: Result<T, OrbitError>) -> String {
    match result {
        Err(OrbitError::InvalidInput(message)) => message,
        Err(error) => panic!("expected invalid input, got {error:?}"),
        Ok(_) => panic!("expected invalid input"),
    }
}

pub(super) struct UnmanagedToolEnvGuard {
    _lock: MutexGuard<'static, ()>,
    vars: Vec<(&'static str, Option<String>)>,
}

pub(super) fn unmanaged_tool_env_guard() -> UnmanagedToolEnvGuard {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let names = [
        "ORBIT_MANAGED_RUN_CONTEXT",
        "ORBIT_TASK_ID",
        "ORBIT_RUN_ID",
        "ORBIT_ACTIVITY_ID",
        "ORBIT_STEP_INDEX",
    ];
    let vars = names
        .into_iter()
        .map(|name| (name, std::env::var(name).ok()))
        .collect::<Vec<_>>();
    // SAFETY: the guard serializes these test env mutations and restores values on drop.
    unsafe {
        for (name, _) in &vars {
            std::env::remove_var(name);
        }
    }
    UnmanagedToolEnvGuard { _lock: lock, vars }
}

impl Drop for UnmanagedToolEnvGuard {
    fn drop(&mut self) {
        // SAFETY: the guard holds the serialization lock for the full mutation window.
        unsafe {
            for (name, value) in &self.vars {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

pub(super) struct EnvVarGuard {
    _lock: MutexGuard<'static, ()>,
    vars: Vec<(&'static str, Option<String>)>,
}

pub(super) fn env_var_guard(updates: &[(&'static str, Option<&str>)]) -> EnvVarGuard {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let vars = updates
        .iter()
        .map(|(name, _)| (*name, std::env::var(name).ok()))
        .collect::<Vec<_>>();
    // SAFETY: the guard serializes these test env mutations and restores values on drop.
    unsafe {
        for (name, value) in updates {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
    EnvVarGuard { _lock: lock, vars }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        // SAFETY: the guard holds the serialization lock for the full mutation window.
        unsafe {
            for (name, value) in &self.vars {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}
