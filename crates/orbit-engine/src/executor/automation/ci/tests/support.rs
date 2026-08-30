//! Scripted GitHub state and a minimal task host.
//!
//! The CI stages are pure functions of what GitHub says plus what they write
//! back to task records, so the tests script both ends and never spawn `gh`.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::Utc;
use orbit_common::OrbitError;
use orbit_types::task::{Task, TaskComment, TaskPriority, TaskStatus, TaskType};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate};

use super::super::query::{AuthStatus, CiQueries, LogScope, RunLog};
use super::super::verify::Waiter;

pub(super) fn task(id: &str, status: TaskStatus) -> Task {
    let now = Utc::now();
    Task {
        id: id.to_string(),
        title: "Remediate current GitHub Actions failures".to_string(),
        description: String::new(),
        acceptance_criteria: Vec::new(),
        tags: vec!["ci-failure-remediation".to_string()],
        required_tools: Vec::new(),
        plan: String::new(),
        execution_summary: String::new(),
        context_files: Vec::new(),
        created_by: None,
        planned_by: None,
        implemented_by: None,
        status,
        priority: TaskPriority::High,
        complexity: None,
        task_type: TaskType::Bug,
        pr_status: None,
        external_refs: Vec::new(),
        relations: Vec::new(),
        job_run_id: Some("jrun-ci".to_string()),
        crew: None,
        orchestrator: None,
        created_at: now,
        updated_at: now,
    }
}

pub(super) struct TestHost {
    tasks: Mutex<Vec<Task>>,
    comments: Mutex<HashMap<String, Vec<TaskComment>>>,
    updates: Mutex<Vec<(String, TaskAutomationUpdate)>>,
}

impl TestHost {
    pub(super) fn new(tasks: Vec<Task>) -> Self {
        Self {
            tasks: Mutex::new(tasks),
            comments: Mutex::new(HashMap::new()),
            updates: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn updates(&self) -> Vec<(String, TaskAutomationUpdate)> {
        self.updates.lock().expect("updates lock").clone()
    }

    pub(super) fn get_task_status(&self, task_id: &str) -> TaskStatus {
        self.tasks
            .lock()
            .expect("tasks lock")
            .iter()
            .find(|task| task.id == task_id)
            .map(|task| task.status)
            .expect("known task")
    }

    pub(super) fn comments_for(&self, task_id: &str) -> Vec<TaskComment> {
        self.comments
            .lock()
            .expect("comments lock")
            .get(task_id)
            .cloned()
            .unwrap_or_default()
    }
}

impl RuntimeHost for TestHost {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        self.tasks
            .lock()
            .expect("tasks lock")
            .iter()
            .find(|task| task.id == task_id)
            .cloned()
            .ok_or_else(|| OrbitError::Execution(format!("unknown task {task_id}")))
    }

    fn get_task_comments(&self, task_id: &str) -> Result<Vec<TaskComment>, OrbitError> {
        Ok(self.comments_for(task_id))
    }

    fn apply_task_automation_update(
        &self,
        task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        if let Some(task) = self
            .tasks
            .lock()
            .expect("tasks lock")
            .iter_mut()
            .find(|task| task.id == task_id)
        {
            if let Some(status) = update.status {
                task.status = status;
            }
            if let Some(summary) = &update.execution_summary {
                task.execution_summary = summary.clone();
            }
        }
        if !update.append_comments.is_empty() {
            self.comments
                .lock()
                .expect("comments lock")
                .entry(task_id.to_string())
                .or_default()
                .extend(update.append_comments.clone());
        }
        self.updates
            .lock()
            .expect("updates lock")
            .push((task_id.to_string(), update));
        Ok(())
    }
}

/// Scripted GitHub answers. Anything not scripted is an empty result, which is
/// itself a case worth exercising.
#[derive(Default)]
pub(super) struct FakeQueries {
    pub(super) auth: Option<AuthStatus>,
    pub(super) repo: Value,
    pub(super) pull_requests: Vec<Value>,
    pub(super) branch_heads: HashMap<String, String>,
    /// Runs per branch. A `Vec` of pages: each `runs_for_branch` call pops the
    /// next page, so a test can make CI progress between polls.
    pub(super) runs: Mutex<HashMap<String, Vec<Vec<Value>>>>,
    pub(super) run_views: HashMap<String, Value>,
    pub(super) logs: HashMap<(String, bool), String>,
}

impl FakeQueries {
    pub(super) fn authenticated() -> Self {
        Self {
            auth: Some(AuthStatus {
                available: true,
                authenticated: true,
                detail: "GitHub CLI is authenticated on this host".to_string(),
            }),
            repo: json!({
                "name": "orbit",
                "full_name": "acme/orbit",
                "default_branch": "main",
            }),
            ..Self::default()
        }
    }

    pub(super) fn unauthenticated(detail: &str) -> Self {
        Self {
            auth: Some(AuthStatus {
                available: true,
                authenticated: false,
                detail: detail.to_string(),
            }),
            ..Self::default()
        }
    }

    pub(super) fn with_head(mut self, branch: &str, sha: &str) -> Self {
        self.branch_heads
            .insert(branch.to_string(), sha.to_string());
        self
    }

    pub(super) fn with_runs(self, branch: &str, pages: Vec<Vec<Value>>) -> Self {
        self.runs
            .lock()
            .expect("runs lock")
            .insert(branch.to_string(), pages);
        self
    }

    pub(super) fn with_run_view(mut self, run_id: &str, view: Value) -> Self {
        self.run_views.insert(run_id.to_string(), view);
        self
    }

    pub(super) fn with_log(mut self, run_id: &str, all_scope: bool, log: &str) -> Self {
        self.logs
            .insert((run_id.to_string(), all_scope), log.to_string());
        self
    }
}

impl CiQueries for FakeQueries {
    fn auth_status(&self) -> AuthStatus {
        self.auth.clone().unwrap_or(AuthStatus {
            available: false,
            authenticated: false,
            detail: "no GitHub CLI on this host".to_string(),
        })
    }

    fn repo_view(&self) -> Result<Value, OrbitError> {
        Ok(self.repo.clone())
    }

    fn open_pull_requests(&self, limit: u64) -> Result<Vec<Value>, OrbitError> {
        Ok(self
            .pull_requests
            .iter()
            .take(limit as usize)
            .cloned()
            .collect())
    }

    fn runs_for_branch(&self, branch: &str, _limit: u64) -> Result<Vec<Value>, OrbitError> {
        let mut runs = self.runs.lock().expect("runs lock");
        let Some(pages) = runs.get_mut(branch) else {
            return Ok(Vec::new());
        };
        if pages.len() > 1 {
            Ok(pages.remove(0))
        } else {
            Ok(pages.first().cloned().unwrap_or_default())
        }
    }

    fn run_view(&self, run_id: &str) -> Result<Value, OrbitError> {
        Ok(self
            .run_views
            .get(run_id)
            .cloned()
            .unwrap_or_else(|| json!({"failed_jobs": []})))
    }

    fn run_logs(
        &self,
        run_id: &str,
        scope: LogScope,
        max_bytes: usize,
    ) -> Result<RunLog, OrbitError> {
        let raw = self
            .logs
            .get(&(run_id.to_string(), scope == LogScope::All))
            .cloned()
            .unwrap_or_default();
        Ok(super::super::query::bounded_run_log(&raw, max_bytes))
    }

    fn remote_branch_head(&self, branch: &str) -> Result<Option<String>, OrbitError> {
        Ok(self.branch_heads.get(branch).cloned())
    }
}

/// A waiter that consumes the budget without spending wall-clock time.
pub(super) struct InstantWaiter;

impl Waiter for InstantWaiter {
    fn wait(&self, seconds: u64) -> u64 {
        seconds
    }
}

pub(super) fn run(
    run_id: u64,
    workflow: &str,
    sha: &str,
    status: &str,
    conclusion: Option<&str>,
    created_at: &str,
) -> Value {
    json!({
        "run_id": run_id,
        "workflow": workflow,
        "title": format!("{workflow} on {sha}"),
        "status": status,
        "conclusion": conclusion,
        "event": "push",
        "head_branch": "topic",
        "reported_head_sha": sha,
        "created_at": created_at,
        "url": format!("https://github.com/acme/orbit/actions/runs/{run_id}"),
    })
}
