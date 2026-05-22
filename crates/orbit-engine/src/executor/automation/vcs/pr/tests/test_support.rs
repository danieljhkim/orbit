use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use chrono::Utc;
use orbit_common::types::{
    Activity, ExternalRef, Job, JobTargetType, NotFoundKind, OrbitError, OrbitEvent, Role, Task,
    TaskArtifact, TaskComment, TaskPriority, TaskStatus, TaskType, push_external_ref_if_missing,
};
use orbit_tools::ToolContext;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

use crate::context::{
    JobRunResult, PrConfig, RuntimeHost, TaskActivityUpdate, TaskAutomationUpdate, TaskReadHost,
    TaskWriteHost,
};
use crate::executor::registry::ActivityExecutorRegistry;

use super::super::super::freshness::BranchFreshness;

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub name: String,
    pub input: Value,
}

pub struct PrOpenTestHost {
    tasks: Mutex<Vec<Task>>,
    comments: Mutex<HashMap<String, Vec<TaskComment>>>,
    tool_calls: Mutex<Vec<ToolCall>>,
    automation_updates: Mutex<Vec<(String, TaskAutomationUpdate)>>,
    activity_implementer: Option<(String, String)>,
    repo_root: PathBuf,
    data_root: PathBuf,
    scoreboard_dir: PathBuf,
    registry: ActivityExecutorRegistry,
    tool_errors: Mutex<HashMap<String, String>>,
}

impl PrOpenTestHost {
    pub fn new(tasks: Vec<Task>, repo_root: PathBuf) -> Self {
        let data_root = repo_root.join(".orbit-test-data");
        let scoreboard_dir = data_root.join("scoreboard");
        Self {
            tasks: Mutex::new(tasks),
            comments: Mutex::new(HashMap::new()),
            tool_calls: Mutex::new(Vec::new()),
            automation_updates: Mutex::new(Vec::new()),
            activity_implementer: None,
            repo_root,
            data_root,
            scoreboard_dir,
            registry: ActivityExecutorRegistry::default(),
            tool_errors: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_activity_implementer(mut self, agent: &str, model: &str) -> Self {
        self.activity_implementer = Some((agent.to_string(), model.to_string()));
        self
    }

    pub fn fail_tool(&self, name: &str, message: &str) {
        self.tool_errors
            .lock()
            .expect("tool errors lock")
            .insert(name.to_string(), message.to_string());
    }

    pub fn comments_for(&self, task_id: &str) -> Vec<TaskComment> {
        self.comments
            .lock()
            .expect("comments lock")
            .get(task_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn tool_calls(&self) -> Vec<ToolCall> {
        self.tool_calls.lock().expect("tool calls lock").clone()
    }

    pub fn pr_create_body(&self) -> String {
        self.tool_calls()
            .into_iter()
            .find(|call| call.name == "github.pr.create")
            .and_then(|call| {
                call.input
                    .get("body")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .expect("github.pr.create body")
    }

    pub fn automation_updates(&self) -> Vec<(String, TaskAutomationUpdate)> {
        self.automation_updates
            .lock()
            .expect("automation updates lock")
            .clone()
    }
}

impl TaskReadHost for PrOpenTestHost {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        self.tasks
            .lock()
            .expect("tasks lock")
            .iter()
            .find(|task| task.id == task_id)
            .cloned()
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, task_id.to_string()))
    }

    fn get_task_artifacts(&self, _task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
        Ok(Vec::new())
    }

    fn get_task_comments(&self, task_id: &str) -> Result<Vec<TaskComment>, OrbitError> {
        Ok(self
            .comments
            .lock()
            .expect("comments lock")
            .get(task_id)
            .cloned()
            .unwrap_or_default())
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
            .lock()
            .expect("tasks lock")
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

impl TaskWriteHost for PrOpenTestHost {
    fn start_task(
        &self,
        _task_id: &str,
        _note: Option<String>,
        _comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        Err(OrbitError::Execution(
            "start_task is not needed by pr_open tests".to_string(),
        ))
    }

    fn admit_task_for_workflow(&self, _task_id: &str, _workflow: &str) -> Result<Task, OrbitError> {
        Err(OrbitError::Execution(
            "admit_task_for_workflow is not needed by pr_open tests".to_string(),
        ))
    }

    fn update_task_from_activity(
        &self,
        _task_id: &str,
        _update: TaskActivityUpdate,
    ) -> Result<Task, OrbitError> {
        Err(OrbitError::Execution(
            "update_task_from_activity is not needed by pr_open tests".to_string(),
        ))
    }

    fn apply_task_automation_update(
        &self,
        task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        self.automation_updates
            .lock()
            .expect("automation updates lock")
            .push((task_id.to_string(), update.clone()));

        if !update.append_comments.is_empty() {
            self.comments
                .lock()
                .expect("comments lock")
                .entry(task_id.to_string())
                .or_default()
                .extend(update.append_comments.iter().cloned());
        }

        let mut tasks = self.tasks.lock().expect("tasks lock");
        let task = tasks
            .iter_mut()
            .find(|task| task.id == task_id)
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, task_id.to_string()))?;
        let transition_implemented_by =
            if matches!(update.status, Some(TaskStatus::Review | TaskStatus::Done)) {
                Some(
                    update
                        .model
                        .clone()
                        .or(update.agent.clone())
                        .unwrap_or_else(|| "system".to_string()),
                )
            } else {
                None
            };
        if let Some(status) = update.status {
            task.status = status;
        }
        for external_ref in update.external_refs {
            push_external_ref_if_missing(&mut task.external_refs, external_ref);
        }
        if let Some(execution_summary) = update.execution_summary {
            task.execution_summary = execution_summary;
        }
        if let Some(implemented_by) = transition_implemented_by {
            task.implemented_by = Some(implemented_by);
        }
        Ok(())
    }
}

impl RuntimeHost for PrOpenTestHost {
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

    fn activity_implementer_identity(
        &self,
        _input: &Value,
    ) -> Result<(Option<String>, Option<String>), OrbitError> {
        Ok(self
            .activity_implementer
            .clone()
            .map(|(agent, model)| (Some(agent), Some(model)))
            .unwrap_or((None, None)))
    }

    fn run_job_now_with_input_debug(
        &self,
        _job_id: &str,
        _input: Value,
        _debug: bool,
    ) -> Result<JobRunResult, OrbitError> {
        Err(OrbitError::Execution(
            "run_job_now_with_input_debug is not needed by pr_open tests".to_string(),
        ))
    }

    fn validate_activity_target_exists(
        &self,
        _target_type: JobTargetType,
        _target_id: &str,
    ) -> Result<Activity, OrbitError> {
        Err(OrbitError::Execution(
            "validate_activity_target_exists is not needed by pr_open tests".to_string(),
        ))
    }

    fn get_job(&self, _job_id: &str) -> Result<Option<Job>, OrbitError> {
        Ok(None)
    }

    fn run_tool_with_context_and_role(
        &self,
        name: &str,
        input: Value,
        _role: Role,
        _tool_context: ToolContext,
    ) -> Result<Value, OrbitError> {
        self.tool_calls
            .lock()
            .expect("tool calls lock")
            .push(ToolCall {
                name: name.to_string(),
                input: input.clone(),
            });

        if let Some(message) = self
            .tool_errors
            .lock()
            .expect("tool errors lock")
            .get(name)
            .cloned()
        {
            return Err(OrbitError::Execution(message));
        }

        match name {
            "git.push" => Ok(json!({})),
            "github.pr.merge" => Ok(json!({})),
            "github.pr.create" => Ok(json!({
                "url": "https://github.example/orbit/orbit/pull/42"
            })),
            "github.pr.view" => Ok(json!({
                "pull_request": { "number": 42 }
            })),
            other => Err(OrbitError::not_found(NotFoundKind::Tool, other.to_string())),
        }
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

pub fn task(id: &str, title: &str, execution_summary: &str) -> Task {
    let now = Utc::now();
    Task {
        id: id.to_string(),
        title: title.to_string(),
        description: String::new(),
        acceptance_criteria: Vec::new(),
        tags: Vec::new(),
        plan: String::new(),
        execution_summary: execution_summary.to_string(),
        context_files: Vec::new(),
        created_by: Some("gpt-5.5".to_string()),
        planned_by: None,
        implemented_by: None,
        status: TaskStatus::Review,
        priority: TaskPriority::Medium,
        complexity: None,
        task_type: TaskType::Chore,
        pr_status: None,
        external_refs: Vec::new(),
        relations: Vec::new(),
        job_run_id: None,
        crew: None,
        created_at: now,
        updated_at: now,
    }
}

pub fn batch_task(id: &str, title: &str, execution_summary: &str) -> Task {
    let mut task = task(id, title, execution_summary);
    task.status = TaskStatus::InProgress;
    task.job_run_id = Some("batch-1".to_string());
    task
}

pub fn review_batch_task(id: &str, implemented_by: Option<&str>, created_by: Option<&str>) -> Task {
    let mut task = task(
        id,
        "Ship attribution",
        "Outcome: success\n\nChanges:\n- Ready.",
    );
    task.status = TaskStatus::Review;
    task.pr_status = Some("approved".to_string());
    task.job_run_id = Some("batch-1".to_string());
    task.implemented_by = implemented_by.map(ToOwned::to_owned);
    task.created_by = created_by.map(ToOwned::to_owned);
    task.external_refs = vec![ExternalRef::github_pr("42").expect("github pr ref")];
    task
}

pub fn task_with_contract(
    id: &str,
    title: &str,
    execution_summary: &str,
    description: &str,
    acceptance_criteria: &[String],
    task_url: Option<&str>,
) -> Task {
    let mut task = task(id, title, execution_summary);
    task.description = description.to_string();
    task.acceptance_criteria = acceptance_criteria.to_vec();
    if let Some(task_url) = task_url {
        task.external_refs = vec![
            ExternalRef::try_new(
                "orbit-task".to_string(),
                id.to_string(),
                Some(task_url.to_string()),
            )
            .expect("task url external ref"),
        ];
    }
    task
}

pub fn freshness() -> BranchFreshness {
    BranchFreshness {
        base_ref: "main".to_string(),
        head_ref: "feature/task".to_string(),
        commits_behind: 0,
        commits_ahead: 2,
    }
}

pub fn test_pr_config(task_url_template: Option<&str>) -> PrConfig {
    PrConfig {
        task_url_template: task_url_template.map(ToOwned::to_owned),
    }
}

pub struct PrWorkspace {
    _temp: TempDir,
    pub repo: PathBuf,
}

pub fn pr_workspace() -> PrWorkspace {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo dir");
    git(&repo, &["init"]);
    git(&repo, &["checkout", "-b", "agent-main"]);
    git(&repo, &["config", "user.name", "Orbit Test"]);
    git(&repo, &["config", "user.email", "orbit-test@example.com"]);
    fs::write(repo.join("README.md"), "base\n").expect("write readme");
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "base"]);
    git(&repo, &["checkout", "-b", "orbit/test-batch"]);
    fs::create_dir_all(repo.join("src")).expect("create src dir");
    fs::write(repo.join("src/lib.rs"), "pub fn changed() {}\n").expect("write lib");
    git(&repo, &["add", "src/lib.rs"]);
    git(&repo, &["commit", "-m", "change"]);

    PrWorkspace { _temp: temp, repo }
}

pub fn no_diff_pr_workspace() -> PrWorkspace {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo dir");
    git(&repo, &["init"]);
    git(&repo, &["checkout", "-b", "agent-main"]);
    git(&repo, &["config", "user.name", "Orbit Test"]);
    git(&repo, &["config", "user.email", "orbit-test@example.com"]);
    fs::write(repo.join("README.md"), "base\n").expect("write readme");
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "base"]);
    git(&repo, &["checkout", "-b", "orbit/test-batch"]);

    PrWorkspace { _temp: temp, repo }
}

/// Workspace where rebasing `orbit/test-batch` onto `agent-main` conflicts:
/// both branches edited the same file with incompatible content, so
/// `git rebase agent-main` fails and `ensure_branch_rebased_onto_base`
/// returns the original "behind" error.
pub fn rebase_conflict_pr_workspace() -> PrWorkspace {
    let temp = tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).expect("create repo dir");
    git(&repo, &["init"]);
    git(&repo, &["checkout", "-b", "agent-main"]);
    git(&repo, &["config", "user.name", "Orbit Test"]);
    git(&repo, &["config", "user.email", "orbit-test@example.com"]);
    fs::write(repo.join("README.md"), "base\n").expect("write readme");
    fs::create_dir_all(repo.join("src")).expect("create src dir");
    fs::write(repo.join("src/lib.rs"), "pub fn base() {}\n").expect("write lib");
    git(&repo, &["add", "README.md", "src/lib.rs"]);
    git(&repo, &["commit", "-m", "base"]);
    git(&repo, &["checkout", "-b", "orbit/test-batch"]);
    fs::write(repo.join("src/lib.rs"), "pub fn branch() {}\n").expect("write branch lib");
    git(&repo, &["add", "src/lib.rs"]);
    git(&repo, &["commit", "-m", "branch change"]);
    git(&repo, &["checkout", "agent-main"]);
    fs::write(repo.join("src/lib.rs"), "pub fn diverged() {}\n").expect("write base lib");
    git(&repo, &["add", "src/lib.rs"]);
    git(&repo, &["commit", "-m", "diverged base"]);
    git(&repo, &["checkout", "orbit/test-batch"]);

    PrWorkspace { _temp: temp, repo }
}

pub fn pr_open_input(repo: &Path, completed_task_ids: Vec<&str>) -> Value {
    json!({
        "workspace_path": repo.to_string_lossy(),
        "job_run_id": "batch-1",
        "completed_task_ids": completed_task_ids,
        "base": "agent-main",
        "base_sync": "local",
    })
}

pub fn merge_batch_pr_input(repo: &Path) -> Value {
    json!({
        "workspace_path": repo.to_string_lossy(),
        "job_run_id": "batch-1",
        "base": "agent-main",
        "base_sync": "local",
    })
}

pub fn git(current_dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
