use std::cell::{Cell, RefCell};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{
    Activity, AgentModelPair, ExternalRef, Job, JobTargetType, OrbitError, OrbitEvent,
    ReviewMessage, ReviewThread, ReviewThreadStatus, Role, Task, TaskArtifact, TaskPriority,
    TaskStatus, TaskType,
};
use orbit_store::task_review_scoreboard;
use orbit_tools::ToolContext;
use serde_json::Value;
use tempfile::tempdir;

use crate::context::{
    JobRunResult, RuntimeHost, TaskActivityUpdate, TaskAutomationUpdate, TaskReadHost,
    TaskWriteHost,
};
use crate::executor::registry::ActivityExecutorRegistry;

use super::super::client::GhClient;
use super::super::patch_match::PrFilePatchMap;
use super::super::thread_sync::{scoreable_review_model, sync_task_review_to_github_with_client};

struct TestHost {
    task: RefCell<Task>,
    review_threads: RefCell<Vec<ReviewThread>>,
    data_root: PathBuf,
    scoreboard_dir: PathBuf,
    registry: ActivityExecutorRegistry,
}

impl TestHost {
    fn new(task: Task, data_root: PathBuf, scoreboard_dir: PathBuf) -> Self {
        Self {
            task: RefCell::new(task),
            review_threads: RefCell::new(fixture_review_threads(Utc::now())),
            data_root,
            scoreboard_dir,
            registry: ActivityExecutorRegistry::default(),
        }
    }
}

impl TaskReadHost for TestHost {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        let task = self.task.borrow();
        if task.id == task_id {
            Ok(task.clone())
        } else {
            Err(OrbitError::InvalidInput(format!(
                "unknown task '{task_id}'"
            )))
        }
    }

    fn get_task_artifacts(&self, _task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
        Ok(Vec::new())
    }

    fn get_task_review_threads(&self, _task_id: &str) -> Result<Vec<ReviewThread>, OrbitError> {
        Ok(self.review_threads.borrow().clone())
    }

    fn list_tasks_filtered(
        &self,
        _status: Option<TaskStatus>,
        _priority: Option<TaskPriority>,
        _parent_id: Option<&str>,
        _batch_id: Option<&str>,
        _external_ref: Option<&orbit_common::types::ExternalRef>,
        _has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        Ok(vec![self.task.borrow().clone()])
    }
}

impl TaskWriteHost for TestHost {
    fn start_task(
        &self,
        _task_id: &str,
        _note: Option<String>,
        _comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        unimplemented!("not needed by review sync tests")
    }

    fn admit_task_for_workflow(&self, _task_id: &str, _workflow: &str) -> Result<Task, OrbitError> {
        unimplemented!("not needed by review sync tests")
    }

    fn update_task_from_activity(
        &self,
        _task_id: &str,
        _update: TaskActivityUpdate,
    ) -> Result<Task, OrbitError> {
        unimplemented!("not needed by review sync tests")
    }

    fn apply_task_automation_update(
        &self,
        _task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        if let Some(review_threads) = update.review_threads {
            *self.review_threads.borrow_mut() = review_threads;
        }
        Ok(())
    }
}

impl RuntimeHost for TestHost {
    fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
        Ok(())
    }

    fn repo_root(&self) -> Result<String, OrbitError> {
        Ok(self.data_root.to_str().expect("utf-8 temp dir").to_string())
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
        unimplemented!("not needed by review sync tests")
    }

    fn validate_activity_target_exists(
        &self,
        _target_type: JobTargetType,
        _target_id: &str,
    ) -> Result<Activity, OrbitError> {
        unimplemented!("not needed by review sync tests")
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
        unimplemented!("not needed by review sync tests")
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

    fn resolved_agent_model_pair(&self, agent_cli: &str) -> Option<AgentModelPair> {
        match agent_cli {
            "codex" => Some(AgentModelPair::new("gpt-5.4", "gpt-5.4-mini")),
            "claude" => Some(AgentModelPair::new("opus-4.6", "sonnet-4.6")),
            "gemini" => Some(AgentModelPair::new(
                "gemini-3.1-pro-preview",
                "gemini-3-flash-preview",
            )),
            "grok" => Some(AgentModelPair::new("grok-4", "grok-3")),
            _ => None,
        }
    }

    fn scoring_enabled(&self) -> bool {
        true
    }

    fn graph_editing(&self) -> bool {
        false
    }

    fn scoreboard_dir(&self) -> &Path {
        &self.scoreboard_dir
    }
}

struct TestGhClient {
    next_id: Cell<u64>,
}

impl TestGhClient {
    fn new() -> Self {
        Self {
            next_id: Cell::new(10),
        }
    }

    fn next_comment_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }
}

impl GhClient for TestGhClient {
    fn get_owner_repo(&self, _repo_root: &str) -> Result<String, OrbitError> {
        Ok("owner/repo".to_string())
    }

    fn get_pr_head_sha(&self, _repo_root: &str, _pr_number: &str) -> Result<String, OrbitError> {
        Ok("abc123".to_string())
    }

    fn load_pr_file_patches(
        &self,
        _repo_root: &str,
        _owner_repo: &str,
        _pr_number: &str,
    ) -> Result<PrFilePatchMap, OrbitError> {
        Ok(PrFilePatchMap::default())
    }

    fn create_inline_review_comment(
        &self,
        _repo_root: &str,
        _owner_repo: &str,
        _pr_number: &str,
        _commit_id: &str,
        _path: &str,
        _line: u64,
        _body: &str,
    ) -> Result<u64, OrbitError> {
        Ok(self.next_comment_id())
    }

    fn create_general_comment(
        &self,
        _repo_root: &str,
        _pr_number: &str,
        _body: &str,
    ) -> Result<u64, OrbitError> {
        Ok(self.next_comment_id())
    }

    fn create_reply_comment(
        &self,
        _repo_root: &str,
        _owner_repo: &str,
        _pr_number: &str,
        _parent_comment_id: u64,
        _body: &str,
    ) -> Result<u64, OrbitError> {
        Ok(self.next_comment_id())
    }
}

fn fixture_task(_repo_root: &Path) -> Task {
    let now = Utc::now();
    Task {
        id: "T-review-sync".to_string(),
        title: "Review sync".to_string(),
        description: String::new(),
        acceptance_criteria: Vec::new(),
        tags: Vec::new(),
        plan: String::new(),
        execution_summary: String::new(),
        context_files: Vec::new(),
        created_by: Some("gpt-5.4".to_string()),
        planned_by: None,
        implemented_by: None,
        status: TaskStatus::Review,
        priority: TaskPriority::Medium,
        complexity: None,
        task_type: TaskType::Chore,
        pr_status: None,
        external_refs: vec![ExternalRef::github_pr("42").expect("github pr ref")],
        relations: Vec::new(),
        job_run_id: None,
        crew: None,
        created_at: now,
        updated_at: now,
    }
}

fn fixture_review_threads(now: chrono::DateTime<Utc>) -> Vec<ReviewThread> {
    vec![ReviewThread {
        thread_id: "rt-test".to_string(),
        path: None,
        line: None,
        status: ReviewThreadStatus::Open,
        messages: vec![ReviewMessage {
            message_id: "rm-test".to_string(),
            at: now,
            by: "gpt-5.4".to_string(),
            body: "Review note.".to_string(),
            github_comment_id: None,
        }],
        github_thread_id: None,
    }]
}

fn read_scoreboard(scoreboard_dir: &Path, file_name: &str) -> Value {
    let raw = fs::read_to_string(scoreboard_dir.join(file_name)).expect("read scoreboard");
    serde_json::from_str(&raw).expect("parse scoreboard")
}

#[test]
fn github_sync_counts_pr_review_without_incrementing_task_review_again() {
    let temp = tempdir().expect("create tempdir");
    let scoreboard_dir = temp.path().join("scoreboard");
    fs::create_dir_all(&scoreboard_dir).expect("create scoreboard dir");
    // orbit-core covers `add_review_thread` persisting this model-only,
    // pending-sync shape. orbit-engine starts from the persisted shape to
    // avoid a reverse dependency on orbit-core in this crate.
    task_review_scoreboard::record_task_review_thread(&scoreboard_dir, "gpt-5.4")
        .expect("seed local review score");

    let task = fixture_task(temp.path());
    let host = TestHost::new(task, temp.path().to_path_buf(), scoreboard_dir.clone());
    let gh = TestGhClient::new();

    let synced = sync_task_review_to_github_with_client(&host, &gh, "T-review-sync").expect("sync");
    assert_eq!(synced, 1);

    let task_review = read_scoreboard(&scoreboard_dir, "task_review.json");
    assert_eq!(
        task_review["task-review-threads"]["gpt-5.4"],
        Value::from(1)
    );
    let pr = read_scoreboard(&scoreboard_dir, "pr.json");
    assert_eq!(pr["pr-review-comments"]["gpt-5.4"], Value::from(1));

    let synced_again =
        sync_task_review_to_github_with_client(&host, &gh, "T-review-sync").expect("resync");
    assert_eq!(synced_again, 0);
    let task_review = read_scoreboard(&scoreboard_dir, "task_review.json");
    assert_eq!(
        task_review["task-review-threads"]["gpt-5.4"],
        Value::from(1)
    );
    let pr = read_scoreboard(&scoreboard_dir, "pr.json");
    assert_eq!(pr["pr-review-comments"]["gpt-5.4"], Value::from(1));
}

#[test]
fn scoreable_review_model_only_scores_configured_models() {
    let temp = tempdir().expect("create tempdir");
    let host = TestHost::new(
        fixture_task(temp.path()),
        temp.path().to_path_buf(),
        temp.path().join("scoreboard"),
    );

    assert_eq!(
        scoreable_review_model(&host, "gpt-5.4").as_deref(),
        Some("gpt-5.4")
    );
    assert_eq!(
        scoreable_review_model(&host, "codex / gpt-5.4").as_deref(),
        Some("gpt-5.4")
    );
    assert_eq!(scoreable_review_model(&host, "gpt-typo"), None);
    assert_eq!(scoreable_review_model(&host, "opus-handle"), None);
    assert_eq!(scoreable_review_model(&host, "codex / gpt-typo"), None);
    assert_eq!(scoreable_review_model(&host, "human"), None);
    assert_eq!(scoreable_review_model(&host, "system"), None);
    assert_eq!(scoreable_review_model(&host, "daniel"), None);
}

#[test]
fn scoreable_review_model_scores_grok_threads() {
    let temp = tempdir().expect("create tempdir");
    let host = TestHost::new(
        fixture_task(temp.path()),
        temp.path().to_path_buf(),
        temp.path().join("scoreboard"),
    );

    assert_eq!(
        scoreable_review_model(&host, "grok-4").as_deref(),
        Some("grok-4")
    );
    assert_eq!(
        scoreable_review_model(&host, "grok / grok-4").as_deref(),
        Some("grok-4")
    );
    assert_eq!(
        scoreable_review_model(&host, "grok / grok-3").as_deref(),
        Some("grok-3")
    );
}
