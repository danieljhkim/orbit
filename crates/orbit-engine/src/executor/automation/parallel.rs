use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use orbit_common::types::{JobRunState, OrbitError, Task, TaskStatus};
use orbit_common::utility::selector::overlaps;
use serde_json::{Value, json};

use super::git::{
    base_sync_mode_from_input, git_output, git_success, resolve_worktree_start_point,
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
    original_status: TaskStatus,
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
    let base_sync_mode = base_sync_mode_from_input(input)?;
    let parallelism = parse_parallelism(input)?;
    let run_id = require_run_id(input, "parallel_dispatch_tasks")?.to_string();
    let Some(selected_tasks) = load_selected_tasks(host, &run_id)? else {
        // Planning can legitimately drain a batch by returning every selected
        // task to backlog and clearing its batch_id. Treat that as a clean no-op
        // so the rest of the local pipeline can short-circuit successfully.
        return Ok(json!({
            "launched": 0,
            "succeeded": 0,
            "failed": 0,
            "skipped": 0,
            "completed_task_ids": [],
            "failures": [],
        }));
    };
    validate_selected_group(&selected_tasks)?;

    // Set up the shared worktree before spawning workers.
    let repo_root_str = host.repo_root()?;
    let repo_root = Path::new(&repo_root_str);
    let start_point = resolve_worktree_start_point(repo_root, &base, base_sync_mode)?;
    let shared_worktree = resolve_shared_worktree_path(repo_root, &run_id)?;
    ensure_shared_worktree(repo_root, &shared_worktree, &start_point, &run_id)?;
    let shared_worktree_str = shared_worktree.to_string_lossy().to_string();
    prepare_tasks_for_worker_launch(host, &selected_tasks, &shared_worktree_str)?;

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
                    tracing::error!(
                        target: "orbit.engine.parallel",
                        timeout_secs = 7200,
                        "parallel task pipeline timed out waiting for worker; breaking out of receive loop",
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

    if failed > 0 {
        return Err(OrbitError::Execution(format!(
            "parallel task pipeline failed for {failed} task(s)"
        )));
    }

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
        None => Ok(repo_root
            .join(".orbit")
            .join("state")
            .join("worktrees")
            .join(dir_name)),
    }
}

fn ensure_shared_worktree(
    repo_root: &Path,
    worktree_path: &Path,
    start_point: &str,
    run_id: &str,
) -> Result<(), OrbitError> {
    let worktree_branch = shared_worktree_branch_name(run_id)?;
    let worktree_branch = worktree_branch.as_str();

    if worktree_path.exists() {
        // Worktree already exists — reset it to the base branch tip so it's fresh.
        let target = git_output(repo_root, &["rev-parse", start_point])?;
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
            start_point,
        ],
    )
}

impl From<Task> for PendingTask {
    fn from(task: Task) -> Self {
        Self {
            task_id: task.id,
            context_files: task.context_files,
            original_status: task.status,
            original_workspace_path: task.workspace_path,
        }
    }
}

fn prepare_tasks_for_worker_launch<H: TaskHost + ?Sized>(
    host: &H,
    tasks: &[PendingTask],
    workspace_path: &str,
) -> Result<(), OrbitError> {
    let mut updated = Vec::with_capacity(tasks.len());
    for task in tasks {
        let update_result = host.apply_task_automation_update(
            &task.task_id,
            TaskAutomationUpdate {
                status: Some(TaskStatus::InProgress),
                workspace_path: Some(Some(workspace_path.to_string())),
                ..TaskAutomationUpdate::default()
            },
        );
        if let Err(err) = update_result {
            return match rollback_prelaunch_task_updates(host, &updated) {
                Ok(()) => Err(err),
                Err(rollback_err) => Err(OrbitError::Execution(format!(
                    "failed to prepare task '{}' for parallel launch: {err}; rollback failed: {rollback_err}",
                    task.task_id
                ))),
            };
        }
        updated.push(task.clone());
    }
    Ok(())
}

fn rollback_prelaunch_task_updates<H: TaskHost + ?Sized>(
    host: &H,
    tasks: &[PendingTask],
) -> Result<(), OrbitError> {
    let mut failures = Vec::new();
    for task in tasks.iter().rev() {
        if let Err(err) = host.apply_task_automation_update(
            &task.task_id,
            TaskAutomationUpdate {
                status: Some(task.original_status),
                workspace_path: Some(task.original_workspace_path.clone()),
                ..TaskAutomationUpdate::default()
            },
        ) {
            failures.push(format!("{}: {err}", task.task_id));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(OrbitError::Execution(format!(
            "failed to roll back pre-launch task updates ({})",
            failures.join("; ")
        )))
    }
}

fn load_selected_tasks<H: TaskHost + ?Sized>(
    host: &H,
    batch_id: &str,
) -> Result<Option<Vec<PendingTask>>, OrbitError> {
    let tasks = host.list_tasks_filtered(None, None, None, Some(batch_id))?;
    if tasks.is_empty() {
        return Ok(None);
    }

    let mut seen = HashSet::new();
    let mut selected = Vec::with_capacity(tasks.len());
    for task in tasks {
        if !seen.insert(task.id.clone()) {
            continue;
        }
        if !matches!(task.status, TaskStatus::Backlog | TaskStatus::InProgress) {
            return Err(OrbitError::InvalidInput(format!(
                "parallel dispatch batch '{batch_id}' contains task '{}' in unsupported status '{}'",
                task.id, task.status
            )));
        }
        selected.push(PendingTask::from(task));
    }

    Ok(Some(selected))
}

pub(super) fn parse_parallelism(input: &Value) -> Result<usize, OrbitError> {
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

pub(super) fn tasks_conflict(left: &[String], right: &[String]) -> bool {
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
    overlaps(left, right)
}

#[cfg(test)]
mod tests {
    use super::tasks_conflict;

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
}
