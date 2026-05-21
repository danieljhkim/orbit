use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use orbit_common::types::{
    Activity, ExternalRef, Job, JobTargetType, NotFoundKind, OrbitError, OrbitEvent, Role, Task,
    TaskArtifact, TaskPriority, TaskStatus, TaskType,
};
use orbit_tools::ToolContext;
use serde_json::Value;
use tempfile::tempdir;

use crate::context::{
    JobRunResult, RuntimeHost, TaskActivityUpdate, TaskAutomationUpdate, TaskReadHost,
    TaskWriteHost,
};
use crate::executor::registry::ActivityExecutorRegistry;

use super::super::super::git::git_success;

pub struct CommitTestHost {
    tasks: Vec<Task>,
    repo_root: PathBuf,
    data_root: PathBuf,
    scoreboard_dir: PathBuf,
    registry: ActivityExecutorRegistry,
}

impl CommitTestHost {
    pub fn new(tasks: Vec<Task>, repo_root: PathBuf) -> Self {
        let data_root = repo_root.join(".orbit-test-data");
        let scoreboard_dir = data_root.join("scoreboard");
        Self {
            tasks,
            repo_root,
            data_root,
            scoreboard_dir,
            registry: ActivityExecutorRegistry::default(),
        }
    }
}

impl TaskReadHost for CommitTestHost {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        self.tasks
            .iter()
            .find(|task| task.id == task_id)
            .cloned()
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, task_id.to_string()))
    }

    fn get_task_artifacts(&self, _task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
        Ok(Vec::new())
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
        external_ref: Option<&orbit_common::types::ExternalRef>,
        has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        Ok(self
            .tasks
            .iter()
            .filter(|task| status.is_none_or(|status| task.status == status))
            .filter(|task| priority.is_none_or(|priority| task.priority == priority))
            .filter(|task| parent_id.is_none_or(|parent_id| task.parent_id() == Some(parent_id)))
            .filter(|task| {
                batch_id.is_none_or(|batch_id| task.job_run_id.as_deref() == Some(batch_id))
            })
            .filter(|task| {
                external_ref.is_none_or(|external_ref| {
                    task.external_refs.iter().any(|candidate| {
                        candidate.system == external_ref.system && candidate.id == external_ref.id
                    })
                })
            })
            .filter(|task| {
                has_external_ref_system.is_none_or(|system| {
                    task.external_refs
                        .iter()
                        .any(|candidate| candidate.system == system)
                })
            })
            .cloned()
            .collect())
    }
}

impl TaskWriteHost for CommitTestHost {
    fn start_task(
        &self,
        _task_id: &str,
        _note: Option<String>,
        _comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        Err(OrbitError::Execution(
            "start_task is not needed by commit tests".to_string(),
        ))
    }

    fn admit_task_for_workflow(&self, _task_id: &str, _workflow: &str) -> Result<Task, OrbitError> {
        Err(OrbitError::Execution(
            "admit_task_for_workflow is not needed by commit tests".to_string(),
        ))
    }

    fn update_task_from_activity(
        &self,
        _task_id: &str,
        _update: TaskActivityUpdate,
    ) -> Result<Task, OrbitError> {
        Err(OrbitError::Execution(
            "update_task_from_activity is not needed by commit tests".to_string(),
        ))
    }

    fn apply_task_automation_update(
        &self,
        _task_id: &str,
        _update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        Err(OrbitError::Execution(
            "apply_task_automation_update is not needed by commit tests".to_string(),
        ))
    }
}

impl RuntimeHost for CommitTestHost {
    fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
        Ok(())
    }

    fn repo_root(&self) -> Result<String, OrbitError> {
        Ok(self.repo_root.to_string_lossy().to_string())
    }

    fn data_root(&self) -> &Path {
        &self.data_root
    }

    fn activity_executor_registry(&self) -> &ActivityExecutorRegistry {
        &self.registry
    }

    fn run_job_now_with_input_debug(
        &self,
        _job_id: &str,
        _input: Value,
        _debug: bool,
    ) -> Result<JobRunResult, OrbitError> {
        Err(OrbitError::Execution(
            "run_job_now_with_input_debug is not needed by commit tests".to_string(),
        ))
    }

    fn validate_activity_target_exists(
        &self,
        _target_type: JobTargetType,
        _target_id: &str,
    ) -> Result<Activity, OrbitError> {
        Err(OrbitError::Execution(
            "validate_activity_target_exists is not needed by commit tests".to_string(),
        ))
    }

    fn get_job(&self, _job_id: &str) -> Result<Option<Job>, OrbitError> {
        Ok(None)
    }

    fn run_tool_with_context_and_role(
        &self,
        _name: &str,
        _input: Value,
        _role: Role,
        _tool_context: ToolContext,
    ) -> Result<Value, OrbitError> {
        Err(OrbitError::Execution(
            "run_tool_with_context_and_role is not needed by commit tests".to_string(),
        ))
    }

    fn maybe_create_failure_task(
        &self,
        _job_id: &str,
        _run_id: &str,
        _error_code: &str,
        _error_message: &str,
        _agent: Option<&str>,
        _model: Option<&str>,
    ) -> Result<(), OrbitError> {
        Ok(())
    }

    fn scoring_enabled(&self) -> bool {
        false
    }

    fn graph_editing(&self) -> bool {
        false
    }

    fn scoreboard_dir(&self) -> &Path {
        &self.scoreboard_dir
    }
}

pub fn initialized_git_repo() -> tempfile::TempDir {
    let temp = tempdir().unwrap();
    let repo = temp.path();
    git_success(repo, &["init"]).expect("git init");
    git_success(repo, &["config", "user.name", "Local User"]).expect("config user.name");
    git_success(repo, &["config", "user.email", "local@example.test"]).expect("config user.email");
    fs::write(repo.join("README.md"), "base\n").unwrap();
    git_success(repo, &["add", "README.md"]).expect("git add");
    git_success(repo, &["commit", "-m", "initial commit"]).expect("initial commit");
    temp
}

pub fn initialized_git_repo_without_local_user_config() -> tempfile::TempDir {
    let temp = tempdir().unwrap();
    let repo = temp.path();
    git_success(repo, &["init"]).expect("git init");
    fs::write(repo.join("README.md"), "base\n").unwrap();
    git_success(repo, &["add", "README.md"]).expect("git add");
    git_success(
        repo,
        &[
            "-c",
            "user.name=Initial User",
            "-c",
            "user.email=initial@example.test",
            "commit",
            "-m",
            "initial commit",
        ],
    )
    .expect("initial commit");
    assert_eq!(
        local_user_config_snapshot(repo),
        CommandSnapshot {
            code: Some(1),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    );
    temp
}

#[derive(Debug, Eq, PartialEq)]
pub struct CommandSnapshot {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub fn local_user_config_snapshot(repo: &Path) -> CommandSnapshot {
    git_command_snapshot(repo, &["config", "--local", "--get-regexp", "^user\\."])
}

pub fn git_stdout_bytes(repo: &Path, args: &[&str], context: &str) -> Vec<u8> {
    let snapshot = git_command_snapshot(repo, args);
    assert_eq!(
        snapshot.code,
        Some(0),
        "{context}: stderr={}",
        String::from_utf8_lossy(&snapshot.stderr)
    );
    snapshot.stdout
}

fn git_command_snapshot(repo: &Path, args: &[&str]) -> CommandSnapshot {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run git command");
    CommandSnapshot {
        code: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

pub fn task_with_file(id: &str, title: &str, path: &str, implemented_by: &str) -> Task {
    let now = Utc::now();
    Task {
        id: id.to_string(),
        title: title.to_string(),
        description: String::new(),
        acceptance_criteria: Vec::new(),
        tags: Vec::new(),
        plan: String::new(),
        execution_summary: String::new(),
        context_files: vec![format!("file:{path}")],
        created_by: None,
        planned_by: None,
        implemented_by: Some(implemented_by.to_string()),
        status: TaskStatus::InProgress,
        priority: TaskPriority::Medium,
        complexity: None,
        task_type: TaskType::Chore,
        pr_status: None,
        external_refs: Vec::new(),
        relations: Vec::new(),
        job_run_id: Some("batch-1".to_string()),
        crew: None,
        created_at: now,
        updated_at: now,
    }
}

pub fn external_ref(system: &str, id: &str) -> ExternalRef {
    ExternalRef::try_new(system.to_string(), id.to_string(), None)
        .expect("external ref fixture is valid")
}
