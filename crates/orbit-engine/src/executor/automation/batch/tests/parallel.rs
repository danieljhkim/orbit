#![allow(missing_docs)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::Utc;
use orbit_common::types::{
    Activity, ExternalRef, Job, JobTargetType, NotFoundKind, OrbitError, OrbitEvent, Role, Task,
    TaskArtifact, TaskPriority, TaskStatus, TaskType,
};
use orbit_tools::ToolContext;
use serde_json::{Value, json};

use crate::context::{
    RuntimeHost, TaskActivityUpdate, TaskAutomationUpdate, TaskReadHost, TaskWriteHost,
};
use crate::executor::registry::ActivityExecutorRegistry;

use super::super::parallel::{PARALLEL_WORKER_JOB_ID, run_parallel_task_pipeline, tasks_conflict};

#[test]
fn tasks_conflict_uses_selector_anchor_overlap() {
    assert!(tasks_conflict(
        &["symbol:f.rs#a:method".to_string()],
        &["symbol:f.rs#b:method".to_string()]
    ));
    assert!(tasks_conflict(
        &["dir:src".to_string()],
        &["file:src/lib.rs".to_string()]
    ));
    assert!(!tasks_conflict(
        &["file:f.rs".to_string()],
        &["file:g.rs".to_string()]
    ));
}

#[test]
fn parallel_pipeline_timeout_cancels_never_returning_worker_promptly() {
    let repo = init_git_repo();
    let batch_id = "jrun-never-returning-worker";
    let host = ParallelTimeoutTestHost::new(
        repo.path().to_path_buf(),
        vec![task_with_batch("T-timeout", batch_id)],
    );

    let started = Instant::now();
    let result = run_parallel_task_pipeline(
        &host,
        &json!({
            "run_id": batch_id,
            "base": "main",
            "base_sync": "local",
            "parallelism": 1,
            "worker_timeout_seconds": 1,
        }),
        false,
    );
    let elapsed = started.elapsed();

    let error = result.expect_err("hung worker should fail the batch");
    assert!(
        error
            .to_string()
            .contains("parallel task pipeline failed for 1 task(s)")
    );
    assert!(
        elapsed < Duration::from_secs(4),
        "timeout path should return promptly; elapsed={elapsed:?}"
    );
    assert_eq!(host.cancelled_runs(), vec!["worker-run-1".to_string()]);

    let events = host.events();
    let cancel_position = events
        .iter()
        .position(|event| event == "cancel:worker-run-1")
        .expect("worker run should be cancelled");
    let block_position = events
        .iter()
        .position(|event| event.starts_with("block:T-timeout:"))
        .expect("timed-out task should be blocked");
    assert!(
        cancel_position < block_position,
        "timed-out worker should be cancelled before recording failure"
    );

    let task = host.get_task("T-timeout").expect("task after timeout");
    assert_eq!(task.status, TaskStatus::Blocked);
}

struct ParallelTimeoutTestHost {
    repo_root: PathBuf,
    data_root: PathBuf,
    scoreboard_dir: PathBuf,
    registry: ActivityExecutorRegistry,
    tasks: Mutex<Vec<Task>>,
    next_run: Mutex<usize>,
    cancelled_runs: Mutex<Vec<String>>,
    events: Mutex<Vec<String>>,
}

impl ParallelTimeoutTestHost {
    fn new(repo_root: PathBuf, tasks: Vec<Task>) -> Self {
        let data_root = repo_root.join(".orbit");
        let scoreboard_dir = data_root.join("state").join("scoreboard");
        Self {
            repo_root,
            data_root,
            scoreboard_dir,
            registry: ActivityExecutorRegistry::default(),
            tasks: Mutex::new(tasks),
            next_run: Mutex::new(1),
            cancelled_runs: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
        }
    }

    fn cancelled_runs(&self) -> Vec<String> {
        self.cancelled_runs
            .lock()
            .expect("cancelled runs lock")
            .clone()
    }

    fn events(&self) -> Vec<String> {
        self.events.lock().expect("events lock").clone()
    }
}

impl TaskReadHost for ParallelTimeoutTestHost {
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

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
        external_ref: Option<&ExternalRef>,
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

impl TaskWriteHost for ParallelTimeoutTestHost {
    fn start_task(
        &self,
        _task_id: &str,
        _note: Option<String>,
        _comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        Err(OrbitError::Execution(
            "start_task is not needed by parallel timeout tests".to_string(),
        ))
    }

    fn admit_task_for_workflow(&self, _task_id: &str, _workflow: &str) -> Result<Task, OrbitError> {
        Err(OrbitError::Execution(
            "admit_task_for_workflow is not needed by parallel timeout tests".to_string(),
        ))
    }

    fn update_task_from_activity(
        &self,
        _task_id: &str,
        _update: TaskActivityUpdate,
    ) -> Result<Task, OrbitError> {
        Err(OrbitError::Execution(
            "update_task_from_activity is not needed by parallel timeout tests".to_string(),
        ))
    }

    fn apply_task_automation_update(
        &self,
        task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        let mut tasks = self.tasks.lock().expect("tasks lock");
        let task = tasks
            .iter_mut()
            .find(|task| task.id == task_id)
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, task_id.to_string()))?;
        if let Some(status) = update.status {
            task.status = status;
            if status == TaskStatus::Blocked {
                self.events.lock().expect("events lock").push(format!(
                    "block:{task_id}:{}",
                    update.status_note.unwrap_or_default()
                ));
            }
        }
        if let Some(execution_summary) = update.execution_summary {
            task.execution_summary = execution_summary;
        }
        task.updated_at = Utc::now();
        Ok(())
    }
}

impl RuntimeHost for ParallelTimeoutTestHost {
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
    ) -> Result<crate::context::JobRunResult, OrbitError> {
        panic!("parallel timeout path must not use the legacy scoped worker runner")
    }

    fn cancel_job_run(&self, run_id: &str) -> Result<(), OrbitError> {
        self.cancelled_runs
            .lock()
            .expect("cancelled runs lock")
            .push(run_id.to_string());
        self.events
            .lock()
            .expect("events lock")
            .push(format!("cancel:{run_id}"));
        Ok(())
    }

    fn validate_activity_target_exists(
        &self,
        _target_type: JobTargetType,
        _target_id: &str,
    ) -> Result<Activity, OrbitError> {
        Err(OrbitError::Execution(
            "validate_activity_target_exists is not needed by parallel timeout tests".to_string(),
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
        match name {
            "orbit.pipeline.invoke" => {
                assert_eq!(input["job_name"], PARALLEL_WORKER_JOB_ID);
                let mut next_run = self.next_run.lock().expect("next run lock");
                let run_id = format!("worker-run-{next_run}");
                *next_run += 1;
                self.events
                    .lock()
                    .expect("events lock")
                    .push(format!("invoke:{run_id}"));
                Ok(json!({
                    "run_id": run_id,
                    "job_name": PARALLEL_WORKER_JOB_ID,
                }))
            }
            "orbit.pipeline.wait" => {
                std::thread::sleep(Duration::from_millis(25));
                let run_ids = input
                    .get("run_ids")
                    .and_then(Value::as_array)
                    .ok_or_else(|| OrbitError::InvalidInput("missing run_ids".to_string()))?;
                Ok(json!({
                    "results": run_ids.iter().map(|run_id| {
                        json!({
                            "run_id": run_id.as_str().expect("run id string"),
                            "status": "running",
                        })
                    }).collect::<Vec<_>>()
                }))
            }
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

    fn scoreboard_dir(&self) -> &std::path::Path {
        &self.data_root
    }
}

fn task_with_batch(id: &str, batch_id: &str) -> Task {
    let now = Utc::now();
    Task {
        id: id.to_string(),
        title: "Never returning parallel worker".to_string(),
        description: "Exercise timeout handling.".to_string(),
        acceptance_criteria: Vec::new(),
        tags: Vec::new(),
        plan: String::new(),
        execution_summary: String::new(),
        context_files: vec![format!("file:{id}.rs")],
        created_by: Some("test".to_string()),
        planned_by: None,
        implemented_by: None,
        status: TaskStatus::Backlog,
        priority: TaskPriority::Medium,
        complexity: None,
        task_type: TaskType::Bug,
        pr_status: None,
        external_refs: Vec::new(),
        relations: Vec::new(),
        job_run_id: Some(batch_id.to_string()),
        crew: None,
        created_at: now,
        updated_at: now,
    }
}

fn init_git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp repo");
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["checkout", "-b", "main"]);
    fs::write(dir.path().join("README.md"), "parallel timeout test\n").expect("write file");
    run_git(dir.path(), &["add", "README.md"]);
    run_git(
        dir.path(),
        &[
            "-c",
            "user.name=Orbit Test",
            "-c",
            "user.email=orbit-test@example.invalid",
            "commit",
            "-m",
            "init",
        ],
    );
    dir
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git command");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
