use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use orbit_types::{JobRunState, OrbitError, Task, TaskStatus};
use serde_json::{Value, json};

use super::git::{
    git_output, git_success, refresh_local_base_branch, resolve_worktree_start_point,
};
use crate::context::{
    RuntimeHost, TaskAutomationUpdate, TaskHost, blocked_workflow_failure_update,
};

const DEFAULT_PARALLEL_BASE: &str = "main";
const DEFAULT_PARALLELISM: usize = 4;
const PARALLEL_WORKER_JOB_ID: &str = "job_parallel_task_worker";
const SHARED_WORKTREE_NAME_PREFIX: &str = "parallel-batch";
const SHARED_WORKTREE_BRANCH_PREFIX: &str = "orbit/parallel-batch";

/// Sanitize a run_id into a token safe to use as a git branch component and
/// filesystem directory segment. Keeps `[A-Za-z0-9._-]`, replaces everything
/// else with `-`, and trims leading/trailing separators.
fn sanitize_run_id_token(run_id: &str) -> Result<String, OrbitError> {
    let sanitized: String = run_id
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = sanitized
        .trim_matches(|c: char| c == '-' || c == '.')
        .to_string();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "cannot derive shared worktree token from run_id '{run_id}'"
        )));
    }
    Ok(trimmed)
}

fn shared_worktree_dir_name(run_id: &str) -> Result<String, OrbitError> {
    Ok(format!(
        "{SHARED_WORKTREE_NAME_PREFIX}-{}",
        sanitize_run_id_token(run_id)?
    ))
}

fn shared_worktree_branch_name(run_id: &str) -> Result<String, OrbitError> {
    // Use a dash separator (not a slash) so the branch does not nest under the
    // legacy `orbit/parallel-batch` ref name. Git refuses to create a child
    // ref like `orbit/parallel-batch/jrun-1` if a leaf ref `orbit/parallel-batch`
    // already exists, which is exactly the collision this task is fixing.
    Ok(format!(
        "{SHARED_WORKTREE_BRANCH_PREFIX}-{}",
        sanitize_run_id_token(run_id)?
    ))
}

/// Extract the `run_id` from an activity input value, returning a trimmed
/// non-empty string. Used by downstream batch activities that need to resolve
/// the same shared worktree as the dispatch step.
pub(super) fn require_run_id<'a>(input: &'a Value, activity: &str) -> Result<&'a str, OrbitError> {
    input
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OrbitError::InvalidInput(format!("{activity} requires input.run_id")))
}

#[derive(Debug, Clone)]
struct PendingTask {
    task_id: String,
    context_files: Vec<String>,
    #[cfg_attr(not(test), allow(dead_code))]
    original_workspace_path: Option<String>,
}

#[derive(Debug)]
struct WorkerOutcome {
    task_id: String,
    result: Result<crate::context::JobRunResult, OrbitError>,
}

fn block_failed_parallel_task<H: TaskHost + ?Sized>(
    host: &H,
    task_id: &str,
    run_id: &str,
    error_code: &str,
    error_message: &str,
) {
    let _ = host.apply_task_automation_update(
        task_id,
        blocked_workflow_failure_update(
            PARALLEL_WORKER_JOB_ID,
            run_id,
            Some(error_code),
            Some(error_message),
        ),
    );
}

pub(super) fn run_parallel_task_pipeline<H: RuntimeHost + TaskHost + Sync + ?Sized>(
    host: &H,
    input: &Value,
    debug: bool,
) -> Result<Value, OrbitError> {
    let base = input
        .get("base")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PARALLEL_BASE)
        .to_string();
    let parallelism = parse_parallelism(input)?;
    let run_id = require_run_id(input, "parallel_dispatch_tasks")?.to_string();
    let selected_tasks = load_selected_tasks(host, &run_id)?;
    validate_selected_group(&selected_tasks)?;

    // Move all selected tasks to in-progress before spawning workers. FIXME: decouple this
    for task in &selected_tasks {
        host.apply_task_automation_update(
            &task.task_id,
            TaskAutomationUpdate {
                status: Some(TaskStatus::InProgress),
                ..TaskAutomationUpdate::default()
            },
        )?;
    }

    // Set up the shared worktree before spawning workers.
    let repo_root_str = host.repo_root()?;
    let repo_root = Path::new(&repo_root_str);
    refresh_local_base_branch(repo_root, &base);
    let shared_worktree = resolve_shared_worktree_path(repo_root, &run_id)?;
    ensure_shared_worktree(repo_root, &shared_worktree, &base, &run_id)?;
    let shared_worktree_str = shared_worktree.to_string_lossy().to_string();
    set_worker_workspace_path(host, &selected_tasks, &shared_worktree_str)?;

    let mut pending = VecDeque::from(selected_tasks.clone());

    let mut launched = 0usize;
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    let mut failures = Vec::new();
    let mut completed_task_ids = HashSet::new();

    let worker_result = std::thread::scope(|scope| -> Result<(), OrbitError> {
        let (tx, rx) = mpsc::channel::<WorkerOutcome>();
        let mut active = Vec::<PendingTask>::new();

        while !pending.is_empty() || !active.is_empty() {
            while active.len() < parallelism {
                let Some(index) = find_launchable_index(&pending, &active) else {
                    break;
                };
                let task = pending.remove(index).ok_or_else(|| {
                    OrbitError::Execution(
                        "parallel dispatch: pending task index out of bounds".to_string(),
                    )
                })?;
                let tx = tx.clone();
                let task_id = task.task_id.clone();
                let worker_workspace = shared_worktree_str.clone();
                let worker_repo_root = repo_root_str.clone();
                active.push(task);
                launched += 1;

                scope.spawn(move || {
                    let result = host.run_job_now_with_input_debug(
                        PARALLEL_WORKER_JOB_ID,
                        json!({
                            "task_id": task_id.clone(),
                            "workspace_path": worker_workspace,
                            "repo_root": worker_repo_root,
                            "verification_mode": "deferred",
                        }),
                        debug,
                    );
                    let _ = tx.send(WorkerOutcome { task_id, result });
                });
            }

            if active.is_empty() {
                continue;
            }

            let outcome = match rx.recv_timeout(Duration::from_secs(7200)) {
                Ok(outcome) => outcome,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    eprintln!(
                        "orbit: parallel task pipeline timed out waiting for worker after 7200s; \
                         breaking out of receive loop"
                    );
                    let timeout_error = "worker timed out after 7200s";
                    for task in active.drain(..) {
                        failed += 1;
                        block_failed_parallel_task(
                            host,
                            &task.task_id,
                            &run_id,
                            "WORKER_TIMEOUT",
                            timeout_error,
                        );
                        failures.push(json!({
                            "task_id": task.task_id,
                            "error": timeout_error,
                        }));
                    }
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let disconnect_error =
                        "parallel task pipeline lost worker coordination channel";
                    for task in active.drain(..) {
                        block_failed_parallel_task(
                            host,
                            &task.task_id,
                            &run_id,
                            "WORKER_CHANNEL_DISCONNECTED",
                            disconnect_error,
                        );
                    }
                    return Err(OrbitError::Execution(disconnect_error.to_string()));
                }
            };
            if let Some(index) = active
                .iter()
                .position(|task| task.task_id == outcome.task_id)
            {
                active.swap_remove(index);
            }

            match outcome.result {
                Ok(result) if result.state == JobRunState::Success => {
                    completed_task_ids.insert(outcome.task_id);
                    succeeded += 1;
                }
                Ok(result) => {
                    failed += 1;
                    let error = format!(
                        "parallel worker completed in non-success state '{}'",
                        result.state
                    );
                    block_failed_parallel_task(
                        host,
                        &outcome.task_id,
                        &result.run_id,
                        "WORKER_NON_SUCCESS",
                        &error,
                    );
                    failures.push(json!({
                        "task_id": outcome.task_id,
                        "error": error,
                    }));
                }
                Err(error) => {
                    failed += 1;
                    let error = error.to_string();
                    block_failed_parallel_task(
                        host,
                        &outcome.task_id,
                        &run_id,
                        "WORKER_EXECUTION_ERROR",
                        &error,
                    );
                    failures.push(json!({
                        "task_id": outcome.task_id,
                        "error": error,
                    }));
                }
            }
        }

        Ok(())
    });
    // NOTE: Do not restore workspace paths here. Downstream pipeline steps
    // (finalize_tasks, commit_and_open_batch_pr, implement_batch_fix) expect
    // workspace_path to still point to the shared worktree.
    worker_result?;

    let completed_task_ids = selected_tasks
        .into_iter()
        .filter_map(|task| {
            completed_task_ids
                .contains(&task.task_id)
                .then_some(task.task_id)
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "launched": launched,
        "succeeded": succeeded,
        "failed": failed,
        "skipped": 0,
        "workspace_path": shared_worktree_str,
        "completed_task_ids": completed_task_ids,
        "failures": failures,
    }))
}

pub(super) fn resolve_shared_worktree_path(
    repo_root: &Path,
    run_id: &str,
) -> Result<PathBuf, OrbitError> {
    let dir_name = shared_worktree_dir_name(run_id)?;
    match std::env::var("ORBIT_WORKTREE_ROOT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(value) => {
            let repo_name = repo_root
                .file_name()
                .and_then(|value| value.to_str())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    OrbitError::Execution(format!(
                        "cannot derive repository name from '{}'",
                        repo_root.display()
                    ))
                })?;
            Ok(PathBuf::from(value).join(repo_name).join(dir_name))
        }
        None => Ok(repo_root.join(".orbit").join("worktrees").join(dir_name)),
    }
}

fn ensure_shared_worktree(
    repo_root: &Path,
    worktree_path: &Path,
    base_branch: &str,
    run_id: &str,
) -> Result<(), OrbitError> {
    let worktree_branch = shared_worktree_branch_name(run_id)?;
    let worktree_branch = worktree_branch.as_str();

    if worktree_path.exists() {
        // Worktree already exists — reset it to the base branch tip so it's fresh.
        let target = git_output(repo_root, &["rev-parse", base_branch])?;
        git_success(
            worktree_path,
            &["checkout", "-B", worktree_branch, target.trim()],
        )?;
        git_success(worktree_path, &["clean", "-fd"])?;
        return Ok(());
    }

    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            OrbitError::Execution(format!(
                "failed to create shared worktree directory '{}': {error}",
                parent.display()
            ))
        })?;
    }

    let start_point = resolve_worktree_start_point(repo_root, base_branch)?;

    // Create the worktree on its own branch, based off the base branch.
    // This avoids "branch already checked out" errors.
    git_success(
        repo_root,
        &[
            "worktree",
            "add",
            "-b",
            worktree_branch,
            &worktree_path.to_string_lossy(),
            &start_point,
        ],
    )
}

impl From<Task> for PendingTask {
    fn from(task: Task) -> Self {
        Self {
            task_id: task.id,
            context_files: task.context_files,
            original_workspace_path: task.workspace_path,
        }
    }
}

fn set_worker_workspace_path<H: TaskHost + ?Sized>(
    host: &H,
    tasks: &[PendingTask],
    workspace_path: &str,
) -> Result<(), OrbitError> {
    for task in tasks {
        host.apply_task_automation_update(
            &task.task_id,
            TaskAutomationUpdate {
                workspace_path: Some(Some(workspace_path.to_string())),
                ..TaskAutomationUpdate::default()
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
fn restore_task_workspace_paths<H: TaskHost + ?Sized>(
    host: &H,
    tasks: &[PendingTask],
) -> Result<(), OrbitError> {
    for task in tasks {
        host.apply_task_automation_update(
            &task.task_id,
            TaskAutomationUpdate {
                workspace_path: Some(task.original_workspace_path.clone()),
                ..TaskAutomationUpdate::default()
            },
        )?;
    }
    Ok(())
}

fn load_selected_tasks<H: TaskHost + ?Sized>(
    host: &H,
    batch_id: &str,
) -> Result<Vec<PendingTask>, OrbitError> {
    let tasks = host.list_tasks_filtered(Some(TaskStatus::Backlog), None, None, Some(batch_id))?;

    if tasks.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "no backlog tasks found for batch_id '{batch_id}'"
        )));
    }

    let mut seen = HashSet::new();
    let mut selected = Vec::with_capacity(tasks.len());
    for task in tasks {
        if !seen.insert(task.id.clone()) {
            continue;
        }
        selected.push(PendingTask::from(task));
    }

    Ok(selected)
}

fn parse_parallelism(input: &Value) -> Result<usize, OrbitError> {
    let Some(value) = input.get("parallelism") else {
        return Ok(DEFAULT_PARALLELISM);
    };
    let raw = value.as_u64().ok_or_else(|| {
        OrbitError::InvalidInput("parallelism must be a positive integer".to_string())
    })?;
    usize::try_from(raw)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| OrbitError::InvalidInput("parallelism must be at least 1".to_string()))
}

fn find_launchable_index(pending: &VecDeque<PendingTask>, active: &[PendingTask]) -> Option<usize> {
    pending.iter().position(|candidate| {
        !active
            .iter()
            .any(|running| tasks_conflict(&candidate.context_files, &running.context_files))
    })
}

fn validate_selected_group(selected: &[PendingTask]) -> Result<(), OrbitError> {
    for (index, left) in selected.iter().enumerate() {
        for right in &selected[index + 1..] {
            if tasks_conflict(&left.context_files, &right.context_files) {
                return Err(OrbitError::InvalidInput(format!(
                    "parallel task batch contains conflicting tasks '{}' and '{}'",
                    left.task_id, right.task_id
                )));
            }
        }
    }
    Ok(())
}

fn tasks_conflict(left: &[String], right: &[String]) -> bool {
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left.iter().any(|left_path| {
        right
            .iter()
            .any(|right_path| paths_conflict(left_path, right_path))
    })
}

fn paths_conflict(left: &str, right: &str) -> bool {
    let left = normalize_path(left);
    let right = normalize_path(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left == right
        || left.starts_with(&format!("{right}/"))
        || right.starts_with(&format!("{left}/"))
}

fn normalize_path(path: &str) -> String {
    path.trim().trim_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        PARALLEL_WORKER_JOB_ID, PendingTask, ensure_shared_worktree, find_launchable_index,
        paths_conflict, resolve_shared_worktree_path, restore_task_workspace_paths,
        run_parallel_task_pipeline, sanitize_run_id_token, set_worker_workspace_path,
        shared_worktree_branch_name, shared_worktree_dir_name, validate_selected_group,
    };
    use crate::context::{JobRunResult, RuntimeHost, TaskAutomationUpdate, TaskHost};
    use chrono::Utc;
    use orbit_tools::ToolContext;
    use orbit_types::{
        Activity, ActorIdentity, Job, JobRunState, JobTargetType, OrbitError, OrbitEvent, Role,
        Task, TaskPriority, TaskStatus, TaskType,
    };
    use serde_json::{Value, json};
    use std::collections::{HashMap, VecDeque};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    #[derive(Default)]
    struct WorkspaceUpdateHost {
        updates: Mutex<Vec<(String, Option<Option<String>>)>>,
    }

    impl WorkspaceUpdateHost {
        fn recorded_updates(&self) -> Vec<(String, Option<Option<String>>)> {
            self.updates.lock().expect("workspace updates lock").clone()
        }
    }

    impl TaskHost for WorkspaceUpdateHost {
        fn get_task(&self, _task_id: &str) -> Result<Task, OrbitError> {
            unimplemented!("not needed for workspace update tests")
        }

        fn get_task_artifacts(
            &self,
            _task_id: &str,
        ) -> Result<Vec<orbit_types::TaskArtifact>, OrbitError> {
            Ok(Vec::new())
        }

        fn list_tasks_filtered(
            &self,
            _status: Option<TaskStatus>,
            _priority: Option<TaskPriority>,
            _parent_id: Option<&str>,
            _batch_id: Option<&str>,
        ) -> Result<Vec<Task>, OrbitError> {
            unimplemented!("not needed for workspace update tests")
        }

        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not needed for workspace update tests")
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not needed for workspace update tests")
        }

        fn apply_task_automation_update(
            &self,
            task_id: &str,
            update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            self.updates
                .lock()
                .expect("workspace updates lock")
                .push((task_id.to_string(), update.workspace_path));
            Ok(())
        }
    }

    struct PipelineHost {
        tasks: Vec<Task>,
        results: Mutex<HashMap<String, Result<JobRunResult, OrbitError>>>,
        updates: Mutex<Vec<(String, TaskAutomationUpdate)>>,
        repo_root: PathBuf,
    }

    impl PipelineHost {
        fn new(
            tasks: Vec<Task>,
            results: HashMap<String, Result<JobRunResult, OrbitError>>,
            repo_root: PathBuf,
        ) -> Self {
            Self {
                tasks,
                results: Mutex::new(results),
                updates: Mutex::new(Vec::new()),
                repo_root,
            }
        }

        fn blocked_updates(&self) -> Vec<(String, TaskAutomationUpdate)> {
            self.updates
                .lock()
                .expect("pipeline updates lock")
                .iter()
                .filter(|(_, update)| update.status == Some(TaskStatus::Blocked))
                .cloned()
                .collect()
        }
    }

    impl TaskHost for PipelineHost {
        fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
            self.tasks
                .iter()
                .find(|task| task.id == task_id)
                .cloned()
                .ok_or_else(|| OrbitError::TaskNotFound(task_id.to_string()))
        }

        fn get_task_artifacts(
            &self,
            _task_id: &str,
        ) -> Result<Vec<orbit_types::TaskArtifact>, OrbitError> {
            Ok(Vec::new())
        }

        fn list_tasks_filtered(
            &self,
            status: Option<TaskStatus>,
            _priority: Option<TaskPriority>,
            _parent_id: Option<&str>,
            batch_id: Option<&str>,
        ) -> Result<Vec<Task>, OrbitError> {
            Ok(self
                .tasks
                .iter()
                .filter(|task| status.is_none_or(|expected| task.status == expected))
                .filter(|task| {
                    batch_id.is_none_or(|expected| task.batch_id.as_deref() == Some(expected))
                })
                .cloned()
                .collect())
        }

        fn start_task(
            &self,
            _task_id: &str,
            _note: Option<String>,
            _comment: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not needed for pipeline tests")
        }

        fn update_task_from_activity(
            &self,
            _task_id: &str,
            _status: TaskStatus,
            _execution_summary: Option<String>,
            _comment: Option<String>,
            _note: Option<String>,
        ) -> Result<Task, OrbitError> {
            unimplemented!("not needed for pipeline tests")
        }

        fn apply_task_automation_update(
            &self,
            task_id: &str,
            update: TaskAutomationUpdate,
        ) -> Result<(), OrbitError> {
            self.updates
                .lock()
                .expect("pipeline updates lock")
                .push((task_id.to_string(), update));
            Ok(())
        }
    }

    impl RuntimeHost for PipelineHost {
        fn record_event(&self, _event: OrbitEvent) -> Result<(), OrbitError> {
            Ok(())
        }

        fn repo_root(&self) -> Result<String, OrbitError> {
            Ok(self.repo_root.to_string_lossy().into_owned())
        }

        fn data_root(&self) -> &Path {
            Path::new(".")
        }

        fn run_job_now_with_input_debug(
            &self,
            _job_id: &str,
            input: Value,
            _debug: bool,
        ) -> Result<JobRunResult, OrbitError> {
            let task_id = input
                .get("task_id")
                .and_then(Value::as_str)
                .expect("worker input task_id");
            self.results
                .lock()
                .expect("pipeline results lock")
                .remove(task_id)
                .expect("task result registered")
        }

        fn validate_activity_target_exists(
            &self,
            _target_type: JobTargetType,
            _target_id: &str,
        ) -> Result<Activity, OrbitError> {
            unimplemented!("not needed for pipeline tests")
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
            unimplemented!("not needed for pipeline tests")
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
            Path::new(".")
        }
    }

    fn pipeline_task(task_id: &str, batch_id: &str, context_file: &str) -> Task {
        let now = Utc::now();
        Task {
            id: task_id.to_string(),
            parent_id: None,
            title: format!("Task {task_id}"),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            plan: String::new(),
            execution_summary: String::new(),
            context_files: vec![context_file.to_string()],
            workspace_path: None,
            repo_root: None,
            assigned_to: None,
            created_by: None,
            actor_identity: ActorIdentity::default(),
            status: TaskStatus::Backlog,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Task,
            pr_number: None,
            pr_status: None,
            proposed_by: None,
            source_task_id: None,
            batch_id: Some(batch_id.to_string()),
            comments: Vec::new(),
            history: Vec::new(),
            review_threads: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn detects_prefix_path_conflicts() {
        assert!(paths_conflict("src/lib.rs", "src/lib.rs"));
        assert!(paths_conflict("src", "src/lib.rs"));
        assert!(paths_conflict("src/lib.rs", "src"));
        assert!(!paths_conflict("src/lib.rs", "tests/lib.rs"));
    }

    #[test]
    fn skips_conflicting_pending_candidate() {
        let active = vec![PendingTask {
            task_id: "T-active".to_string(),
            context_files: vec!["src".to_string()],
            original_workspace_path: None,
        }];
        let pending = VecDeque::from(vec![
            PendingTask {
                task_id: "T-conflict".to_string(),
                context_files: vec!["src/lib.rs".to_string()],
                original_workspace_path: None,
            },
            PendingTask {
                task_id: "T-safe".to_string(),
                context_files: vec!["docs/readme.md".to_string()],
                original_workspace_path: None,
            },
        ]);

        assert_eq!(find_launchable_index(&pending, &active), Some(1));
    }

    #[test]
    fn rejects_conflicting_selected_group() {
        let selected = vec![
            PendingTask {
                task_id: "T-a".to_string(),
                context_files: vec!["src".to_string()],
                original_workspace_path: None,
            },
            PendingTask {
                task_id: "T-b".to_string(),
                context_files: vec!["src/lib.rs".to_string()],
                original_workspace_path: None,
            },
        ];

        assert!(validate_selected_group(&selected).is_err());
    }

    #[test]
    fn sets_task_workspace_to_shared_worktree_for_workers() {
        let host = WorkspaceUpdateHost::default();
        let tasks = vec![PendingTask {
            task_id: "T-a".to_string(),
            context_files: vec!["src/lib.rs".to_string()],
            original_workspace_path: Some("/repo".to_string()),
        }];

        set_worker_workspace_path(
            &host,
            &tasks,
            "/repo/.orbit/worktrees/parallel-batch-jrun-1",
        )
        .expect("workspace path update succeeds");

        assert_eq!(
            host.recorded_updates(),
            vec![(
                "T-a".to_string(),
                Some(Some(
                    "/repo/.orbit/worktrees/parallel-batch-jrun-1".to_string()
                )),
            )]
        );
    }

    #[test]
    fn blocks_only_failed_parallel_worker_tasks() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo_root = tempdir.path();

        run_git(repo_root, &["init", "--initial-branch=main"]);
        run_git(repo_root, &["config", "user.name", "Orbit Tests"]);
        run_git(
            repo_root,
            &["config", "user.email", "orbit-tests@example.com"],
        );
        std::fs::write(repo_root.join("README.md"), "hello\n").expect("write readme");
        run_git(repo_root, &["add", "README.md"]);
        run_git(repo_root, &["commit", "-m", "initial"]);

        let batch_id = "jrun-parallel-test";
        let host = PipelineHost::new(
            vec![
                pipeline_task("T-success", batch_id, "src/lib.rs"),
                pipeline_task("T-failed", batch_id, "docs/readme.md"),
            ],
            HashMap::from([
                (
                    "T-success".to_string(),
                    Ok(JobRunResult {
                        job_id: PARALLEL_WORKER_JOB_ID.to_string(),
                        run_id: "run-success".to_string(),
                        state: JobRunState::Success,
                        attempt: 1,
                        output: Some(json!({})),
                    }),
                ),
                (
                    "T-failed".to_string(),
                    Ok(JobRunResult {
                        job_id: PARALLEL_WORKER_JOB_ID.to_string(),
                        run_id: "run-failed".to_string(),
                        state: JobRunState::Failed,
                        attempt: 1,
                        output: Some(json!({})),
                    }),
                ),
            ]),
            repo_root.to_path_buf(),
        );

        let output = run_parallel_task_pipeline(
            &host,
            &json!({
                "run_id": batch_id,
                "parallelism": 2,
            }),
            false,
        )
        .expect("parallel pipeline succeeds");

        assert_eq!(output["succeeded"], json!(1));
        assert_eq!(output["failed"], json!(1));
        assert_eq!(output["completed_task_ids"], json!(["T-success"]));

        let blocked_updates = host.blocked_updates();
        assert_eq!(blocked_updates.len(), 1);
        assert_eq!(blocked_updates[0].0, "T-failed");
        assert_eq!(blocked_updates[0].1.status, Some(TaskStatus::Blocked));
        assert_eq!(
            blocked_updates[0].1.status_event.as_deref(),
            Some("workflow_run_failed")
        );
        let note = blocked_updates[0]
            .1
            .status_note
            .as_deref()
            .expect("blocked note");
        assert!(note.contains(&format!("job={PARALLEL_WORKER_JOB_ID}")));
        assert!(note.contains("run_id=run-failed"));
        assert!(note.contains("error_code=WORKER_NON_SUCCESS"));
        assert!(note.contains("parallel worker completed in non-success state 'failed'"));
        assert!(
            blocked_updates
                .iter()
                .all(|(task_id, _)| task_id != "T-success")
        );
    }

    #[test]
    fn sanitize_run_id_token_keeps_safe_characters_and_replaces_others() {
        assert_eq!(
            sanitize_run_id_token("jrun-20260408-0219-2").expect("safe id"),
            "jrun-20260408-0219-2"
        );
        assert_eq!(
            sanitize_run_id_token("jrun/2026 04 08").expect("sanitized id"),
            "jrun-2026-04-08"
        );
        assert!(sanitize_run_id_token("///").is_err());
    }

    #[test]
    fn distinct_run_ids_produce_distinct_worktree_paths_and_branches() {
        let repo_root = std::path::PathBuf::from("/repo");
        let path_a = resolve_shared_worktree_path(&repo_root, "jrun-1").expect("path a");
        let path_b = resolve_shared_worktree_path(&repo_root, "jrun-2").expect("path b");
        assert_ne!(path_a, path_b);
        assert!(
            path_a
                .to_string_lossy()
                .ends_with(".orbit/worktrees/parallel-batch-jrun-1")
        );
        assert!(
            path_b
                .to_string_lossy()
                .ends_with(".orbit/worktrees/parallel-batch-jrun-2")
        );

        let branch_a = shared_worktree_branch_name("jrun-1").expect("branch a");
        let branch_b = shared_worktree_branch_name("jrun-2").expect("branch b");
        assert_eq!(branch_a, "orbit/parallel-batch-jrun-1");
        assert_eq!(branch_b, "orbit/parallel-batch-jrun-2");
        assert_eq!(
            shared_worktree_dir_name("jrun-1").expect("dir name"),
            "parallel-batch-jrun-1"
        );
    }

    #[test]
    fn ensure_shared_worktree_succeeds_when_a_previous_static_branch_already_exists() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo_root = tempdir.path();

        run_git(repo_root, &["init", "--initial-branch=main"]);
        run_git(repo_root, &["config", "user.name", "Orbit Tests"]);
        run_git(
            repo_root,
            &["config", "user.email", "orbit-tests@example.com"],
        );
        std::fs::write(repo_root.join("README.md"), "hello\n").expect("write readme");
        run_git(repo_root, &["add", "README.md"]);
        run_git(repo_root, &["commit", "-m", "initial"]);

        // Simulate the legacy collision: a previous static branch is left behind.
        run_git(repo_root, &["branch", "orbit/parallel-batch"]);
        // Also simulate a stale branch that uses the new naming scheme for a
        // *different* run, to make sure we don't trip over unrelated state.
        run_git(repo_root, &["branch", "orbit/parallel-batch-jrun-old"]);

        let run_id = "jrun-20260408-0219-2";
        let worktree_path = resolve_shared_worktree_path(repo_root, run_id).expect("resolve path");
        ensure_shared_worktree(repo_root, &worktree_path, "main", run_id)
            .expect("create dynamic shared worktree despite stale static branch");

        assert!(worktree_path.exists());
        assert!(worktree_path.join(".git").exists());
    }

    fn run_git(dir: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("run git");
        assert!(status.success(), "git {:?} failed", args);
    }

    #[test]
    fn restores_original_task_workspace_after_parallel_run() {
        let host = WorkspaceUpdateHost::default();
        let tasks = vec![
            PendingTask {
                task_id: "T-a".to_string(),
                context_files: vec!["src/lib.rs".to_string()],
                original_workspace_path: Some("/repo".to_string()),
            },
            PendingTask {
                task_id: "T-b".to_string(),
                context_files: vec!["docs/readme.md".to_string()],
                original_workspace_path: None,
            },
        ];

        restore_task_workspace_paths(&host, &tasks).expect("workspace restore succeeds");

        assert_eq!(
            host.recorded_updates(),
            vec![
                ("T-a".to_string(), Some(Some("/repo".to_string()))),
                ("T-b".to_string(), Some(None)),
            ]
        );
    }
}
