use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use orbit_types::{JobRunState, OrbitError, Task, TaskStatus};
use serde_json::{Value, json};

use super::git::{git_output, git_success, resolve_worktree_start_point};
use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

const DEFAULT_PARALLEL_BASE: &str = "agent-main";
const DEFAULT_PARALLELISM: usize = 4;
const PARALLEL_WORKER_JOB_ID: &str = "job_parallel_task_worker";
const SHARED_WORKTREE_NAME: &str = "parallel-batch";
const SHARED_WORKTREE_BRANCH: &str = "orbit/parallel-batch";

#[derive(Debug, Clone)]
struct PendingTask {
    task_id: String,
    context_files: Vec<String>,
}

#[derive(Debug)]
struct WorkerOutcome {
    task_id: String,
    result: Result<crate::context::JobRunResult, OrbitError>,
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
    let selected_tasks = load_selected_tasks(host, input)?;
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
    let shared_worktree = resolve_shared_worktree_path(repo_root)?;
    ensure_shared_worktree(repo_root, &shared_worktree, &base)?;
    let shared_worktree_str = shared_worktree.to_string_lossy().to_string();

    let mut pending = VecDeque::from(selected_tasks.clone());

    let mut launched = 0usize;
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    let mut failures = Vec::new();
    let mut completed_task_ids = HashSet::new();

    std::thread::scope(|scope| -> Result<(), OrbitError> {
        let (tx, rx) = mpsc::channel::<WorkerOutcome>();
        let mut active = Vec::<PendingTask>::new();

        while !pending.is_empty() || !active.is_empty() {
            while active.len() < parallelism {
                let Some(index) = find_launchable_index(&pending, &active) else {
                    break;
                };
                let task = pending
                    .remove(index)
                    .expect("launchable pending task index must exist");
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
                    for task in active.drain(..) {
                        failed += 1;
                        failures.push(json!({
                            "task_id": task.task_id,
                            "error": "worker timed out after 7200s",
                        }));
                    }
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(OrbitError::Execution(
                        "parallel task pipeline lost worker coordination channel".to_string(),
                    ));
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
                    match host.release_file_locks(&outcome.task_id) {
                        Ok(_) => {
                            completed_task_ids.insert(outcome.task_id);
                            succeeded += 1;
                        }
                        Err(error) => {
                            failed += 1;
                            failures.push(json!({
                                "task_id": outcome.task_id,
                                "error": format!(
                                    "parallel worker succeeded but lock release failed: {error}"
                                ),
                            }));
                        }
                    }
                }
                Ok(result) => {
                    failed += 1;
                    failures.push(json!({
                        "task_id": outcome.task_id,
                        "error": format!(
                            "parallel worker completed in non-success state '{}'",
                            result.state
                        ),
                    }));
                    if let Err(lock_err) = host.release_file_locks(&outcome.task_id) {
                        eprintln!(
                            "orbit: failed to release file locks for task '{}' \
                             after non-success worker: {lock_err}",
                            outcome.task_id
                        );
                    }
                }
                Err(error) => {
                    failed += 1;
                    failures.push(json!({
                        "task_id": outcome.task_id,
                        "error": error.to_string(),
                    }));
                    if let Err(lock_err) = host.release_file_locks(&outcome.task_id) {
                        eprintln!(
                            "orbit: failed to release file locks for task '{}' \
                             after worker error: {lock_err}",
                            outcome.task_id
                        );
                    }
                }
            }
        }

        Ok(())
    })?;

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
        "completed_task_ids": completed_task_ids,
        "failures": failures,
    }))
}

pub(super) fn resolve_shared_worktree_path(repo_root: &Path) -> Result<PathBuf, OrbitError> {
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
            Ok(PathBuf::from(value)
                .join(repo_name)
                .join(SHARED_WORKTREE_NAME))
        }
        None => Ok(repo_root
            .join(".orbit")
            .join("worktrees")
            .join(SHARED_WORKTREE_NAME)),
    }
}

fn ensure_shared_worktree(
    repo_root: &Path,
    worktree_path: &Path,
    base_branch: &str,
) -> Result<(), OrbitError> {
    let worktree_branch = SHARED_WORKTREE_BRANCH;

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
        }
    }
}

fn load_selected_tasks<H: TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Vec<PendingTask>, OrbitError> {
    let batch_id = input
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            OrbitError::InvalidInput("parallel_dispatch_tasks requires input.run_id".to_string())
        })?;

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
        PendingTask, ensure_shared_worktree, find_launchable_index, git_output, git_success,
        paths_conflict, validate_selected_group,
    };
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    fn init_test_repo() -> TempDir {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let repo_root = tempdir.path();

        git_success(repo_root, &["init", "--initial-branch=controller"]).expect("init repo");
        git_success(repo_root, &["config", "user.name", "Orbit Tests"])
            .expect("configure user name");
        git_success(
            repo_root,
            &["config", "user.email", "orbit-tests@example.com"],
        )
        .expect("configure user email");

        std::fs::write(repo_root.join("README.md"), "controller\n").expect("write initial file");
        git_success(repo_root, &["add", "README.md"]).expect("stage initial file");
        git_success(repo_root, &["commit", "-m", "initial"]).expect("commit initial file");
        git_success(repo_root, &["branch", "agent-main"]).expect("create agent-main");

        tempdir
    }

    fn create_branch_commit(repo_root: &Path, branch: &str, contents: &str) {
        git_success(repo_root, &["checkout", "-b", branch]).expect("create branch");
        std::fs::write(repo_root.join("README.md"), contents).expect("write branch contents");
        git_success(repo_root, &["add", "README.md"]).expect("stage branch contents");
        git_success(repo_root, &["commit", "-m", &format!("update {branch}")])
            .expect("commit branch contents");
        git_success(repo_root, &["checkout", "controller"]).expect("checkout controller");
    }

    fn shared_worktree_path(repo_root: &Path) -> PathBuf {
        repo_root
            .join(".orbit")
            .join("worktrees")
            .join("parallel-batch")
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
        }];
        let pending = VecDeque::from(vec![
            PendingTask {
                task_id: "T-conflict".to_string(),
                context_files: vec!["src/lib.rs".to_string()],
            },
            PendingTask {
                task_id: "T-safe".to_string(),
                context_files: vec!["docs/readme.md".to_string()],
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
            },
            PendingTask {
                task_id: "T-b".to_string(),
                context_files: vec!["src/lib.rs".to_string()],
            },
        ];

        assert!(validate_selected_group(&selected).is_err());
    }
}
