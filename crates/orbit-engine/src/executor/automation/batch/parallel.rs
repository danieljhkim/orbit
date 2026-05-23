use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::time::{Duration, Instant};

use orbit_common::types::{OrbitError, Role, Task, TaskStatus};
use orbit_common::utility::selector::overlaps;
use orbit_tools::ToolContext;
use serde_json::{Value, json};

use super::super::vcs::{
    ensure_shared_worktree,
    git::{base_sync_mode_from_input, resolve_worktree_start_point},
    resolve_shared_worktree_path,
};
use crate::context::{
    RuntimeHost, TaskAutomationUpdate, TaskHost, blocked_workflow_failure_update,
};

const DEFAULT_PARALLEL_BASE: &str = "main";
const DEFAULT_PARALLELISM: usize = 4;
const DEFAULT_WORKER_TIMEOUT_SECS: u64 = 7200;
const WORKER_WAIT_POLL_SECS: u64 = 1;
// pub(crate) widened for tests/ layout migration (ORB-00240); test reaches via
// exposed surface per docs/design-patterns/test_layout.md. (Logged via
// orbit.task.update model=grok on ORB-00240 before this edit for the visibility
// change on internal const used by timeout tests.)
pub(crate) const PARALLEL_WORKER_JOB_ID: &str = "job_parallel_task_worker";

/// Extract the `run_id` from an activity input value, returning a trimmed
/// non-empty string. Used by downstream batch activities that need to resolve
/// the same shared worktree as the dispatch step.
pub(in crate::executor::automation) fn require_run_id<'a>(
    input: &'a Value,
    activity: &str,
) -> Result<&'a str, OrbitError> {
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
}

#[derive(Debug)]
struct ActiveWorker {
    task: PendingTask,
    run_id: String,
    launched_at: Instant,
}

#[derive(Debug)]
struct WorkerObservation {
    run_id: String,
    state: WorkerRunState,
}

#[derive(Debug)]
enum WorkerRunState {
    Succeeded,
    Failed { code: &'static str, message: String },
    Incomplete,
}

#[derive(Debug, Default)]
struct ParallelWorkerSummary {
    launched: usize,
    succeeded: usize,
    failed: usize,
    failures: Vec<Value>,
    completed_task_ids: HashSet<String>,
}

trait ParallelWorkerRunner {
    fn launch(&self, task_id: &str) -> Result<String, OrbitError>;
    fn wait(
        &self,
        run_ids: &[String],
        timeout: Duration,
    ) -> Result<Vec<WorkerObservation>, OrbitError>;
    fn cancel(&self, run_id: &str) -> Result<(), OrbitError>;
}

struct PipelineToolWorkerRunner<'a, H: RuntimeHost + ?Sized> {
    host: &'a H,
    shared_worktree: &'a str,
    repo_root: &'a str,
}

impl<H: RuntimeHost + ?Sized> ParallelWorkerRunner for PipelineToolWorkerRunner<'_, H> {
    fn launch(&self, task_id: &str) -> Result<String, OrbitError> {
        let output = self.host.run_tool_with_context_and_role(
            "orbit.pipeline.invoke",
            json!({
                "job_name": PARALLEL_WORKER_JOB_ID,
                "input": {
                    "task_id": task_id,
                    "workspace_path": self.shared_worktree,
                    "repo_root": self.repo_root,
                    "verification_mode": "deferred",
                },
            }),
            Role::Admin,
            ToolContext::default(),
        )?;
        output
            .get("run_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                OrbitError::Execution(
                    "orbit.pipeline.invoke returned no run_id for parallel worker".to_string(),
                )
            })
    }

    fn wait(
        &self,
        run_ids: &[String],
        timeout: Duration,
    ) -> Result<Vec<WorkerObservation>, OrbitError> {
        let timeout_seconds = timeout.as_secs().max(1);
        let output = self.host.run_tool_with_context_and_role(
            "orbit.pipeline.wait",
            json!({
                "run_ids": run_ids,
                "timeout_seconds": timeout_seconds,
                "poll_interval_seconds": WORKER_WAIT_POLL_SECS,
            }),
            Role::Admin,
            ToolContext::default(),
        )?;
        parse_worker_wait_observations(&output)
    }

    fn cancel(&self, run_id: &str) -> Result<(), OrbitError> {
        self.host.cancel_job_run(run_id)
    }
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

pub(in crate::executor::automation) fn run_parallel_task_pipeline<
    H: RuntimeHost + TaskHost + Sync + ?Sized,
>(
    host: &H,
    input: &Value,
    _debug: bool,
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
    let worker_timeout = parse_worker_timeout(input)?;
    let run_id = require_run_id(input, "parallel_dispatch_tasks")?.to_string();
    let Some(selected_tasks) = load_selected_tasks(host, &run_id)? else {
        // Planning can legitimately drain a batch by returning every selected
        // task to backlog and clearing its job run assignment. Treat that as a clean no-op
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
    prepare_tasks_for_worker_launch(host, &selected_tasks)?;

    let runner = PipelineToolWorkerRunner {
        host,
        shared_worktree: &shared_worktree_str,
        repo_root: &repo_root_str,
    };
    let worker_summary = dispatch_parallel_workers(
        host,
        &runner,
        VecDeque::from(selected_tasks.clone()),
        parallelism,
        worker_timeout,
        &run_id,
    )?;
    // NOTE: Do not restore workspace paths here. Downstream pipeline steps
    // (finalize_tasks, commit_and_open_batch_pr, implement_batch_fix) expect
    // workspace_path to still point to the shared worktree.

    let completed_task_ids = selected_tasks
        .into_iter()
        .filter_map(|task| {
            worker_summary
                .completed_task_ids
                .contains(&task.task_id)
                .then_some(task.task_id)
        })
        .collect::<Vec<_>>();

    if worker_summary.failed > 0 {
        return Err(OrbitError::Execution(format!(
            "parallel task pipeline failed for {} task(s)",
            worker_summary.failed
        )));
    }

    Ok(json!({
        "launched": worker_summary.launched,
        "succeeded": worker_summary.succeeded,
        "failed": worker_summary.failed,
        "skipped": 0,
        "workspace_path": shared_worktree_str,
        "completed_task_ids": completed_task_ids,
        "failures": worker_summary.failures,
    }))
}

fn dispatch_parallel_workers<H, R>(
    host: &H,
    runner: &R,
    mut pending: VecDeque<PendingTask>,
    parallelism: usize,
    worker_timeout: Duration,
    batch_run_id: &str,
) -> Result<ParallelWorkerSummary, OrbitError>
where
    H: TaskHost + ?Sized,
    R: ParallelWorkerRunner + ?Sized,
{
    let mut active = Vec::<ActiveWorker>::new();
    let mut summary = ParallelWorkerSummary::default();

    while !pending.is_empty() || !active.is_empty() {
        while active.len() < parallelism {
            let Some(index) = find_launchable_index(&pending, active_tasks(&active).as_slice())
            else {
                break;
            };
            let task = pending.remove(index).ok_or_else(|| {
                OrbitError::Execution(
                    "parallel dispatch: pending task index out of bounds".to_string(),
                )
            })?;
            let run_id = match runner.launch(&task.task_id) {
                Ok(run_id) => run_id,
                Err(error) => {
                    let error = error.to_string();
                    block_failed_parallel_task(
                        host,
                        &task.task_id,
                        batch_run_id,
                        "WORKER_LAUNCH_FAILED",
                        &error,
                    );
                    summary.failed += 1;
                    summary.failures.push(json!({
                        "task_id": task.task_id,
                        "error": error,
                    }));
                    break;
                }
            };
            active.push(ActiveWorker {
                task,
                run_id,
                launched_at: Instant::now(),
            });
            summary.launched += 1;
        }

        if active.is_empty() {
            continue;
        }

        if timeout_active_workers(
            host,
            runner,
            &mut active,
            &mut summary,
            worker_timeout,
            batch_run_id,
        ) {
            break;
        }

        let run_ids = active
            .iter()
            .map(|worker| worker.run_id.clone())
            .collect::<Vec<_>>();
        let observations =
            runner.wait(&run_ids, next_worker_wait_duration(&active, worker_timeout))?;
        apply_worker_observations(host, &mut active, &mut summary, observations)?;

        if timeout_active_workers(
            host,
            runner,
            &mut active,
            &mut summary,
            worker_timeout,
            batch_run_id,
        ) {
            break;
        }
    }

    Ok(summary)
}

fn active_tasks(active: &[ActiveWorker]) -> Vec<PendingTask> {
    active.iter().map(|worker| worker.task.clone()).collect()
}

fn next_worker_wait_duration(active: &[ActiveWorker], worker_timeout: Duration) -> Duration {
    active
        .iter()
        .map(|worker| worker_timeout.saturating_sub(worker.launched_at.elapsed()))
        .min()
        .unwrap_or(worker_timeout)
        .min(Duration::from_secs(WORKER_WAIT_POLL_SECS))
        .max(Duration::from_millis(1))
}

fn apply_worker_observations<H: TaskHost + ?Sized>(
    host: &H,
    active: &mut Vec<ActiveWorker>,
    summary: &mut ParallelWorkerSummary,
    observations: Vec<WorkerObservation>,
) -> Result<(), OrbitError> {
    for observation in observations {
        let Some(index) = active
            .iter()
            .position(|worker| worker.run_id == observation.run_id)
        else {
            continue;
        };
        match observation.state {
            WorkerRunState::Succeeded => {
                let worker = active.swap_remove(index);
                summary.completed_task_ids.insert(worker.task.task_id);
                summary.succeeded += 1;
            }
            WorkerRunState::Failed { code, message } => {
                let worker = active.swap_remove(index);
                block_failed_parallel_task(
                    host,
                    &worker.task.task_id,
                    &worker.run_id,
                    code,
                    &message,
                );
                summary.failed += 1;
                summary.failures.push(json!({
                    "task_id": worker.task.task_id,
                    "run_id": worker.run_id,
                    "error": message,
                }));
            }
            WorkerRunState::Incomplete => {}
        }
    }
    Ok(())
}

fn timeout_active_workers<H, R>(
    host: &H,
    runner: &R,
    active: &mut Vec<ActiveWorker>,
    summary: &mut ParallelWorkerSummary,
    worker_timeout: Duration,
    batch_run_id: &str,
) -> bool
where
    H: TaskHost + ?Sized,
    R: ParallelWorkerRunner + ?Sized,
{
    if !active
        .iter()
        .any(|worker| worker.launched_at.elapsed() >= worker_timeout)
    {
        return false;
    }

    tracing::error!(
        target: "orbit.engine.parallel",
        timeout_secs = worker_timeout.as_secs(),
        "parallel task pipeline timed out waiting for worker; cancelling active workers",
    );
    let timeout_error = format!(
        "worker timed out after {}s",
        worker_timeout.as_secs().max(1)
    );
    for worker in active.drain(..) {
        if let Err(error) = runner.cancel(&worker.run_id) {
            tracing::warn!(
                target: "orbit.engine.parallel",
                run_id = %worker.run_id,
                error = %error,
                "failed to cancel timed-out parallel worker run",
            );
        }
        summary.failed += 1;
        block_failed_parallel_task(
            host,
            &worker.task.task_id,
            batch_run_id,
            "WORKER_TIMEOUT",
            &timeout_error,
        );
        summary.failures.push(json!({
            "task_id": worker.task.task_id,
            "run_id": worker.run_id,
            "error": timeout_error,
        }));
    }

    true
}

impl From<Task> for PendingTask {
    fn from(task: Task) -> Self {
        Self {
            task_id: task.id,
            context_files: task.context_files,
            original_status: task.status,
        }
    }
}

fn prepare_tasks_for_worker_launch<H: TaskHost + ?Sized>(
    host: &H,
    tasks: &[PendingTask],
) -> Result<(), OrbitError> {
    let mut updated = Vec::with_capacity(tasks.len());
    for task in tasks {
        let update_result = host.apply_task_automation_update(
            &task.task_id,
            TaskAutomationUpdate {
                status: Some(TaskStatus::InProgress),
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
    let tasks = host.list_tasks_filtered(None, None, None, Some(batch_id), None, None)?;
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

fn parse_worker_timeout(input: &Value) -> Result<Duration, OrbitError> {
    let Some(value) = input.get("worker_timeout_seconds") else {
        return Ok(Duration::from_secs(DEFAULT_WORKER_TIMEOUT_SECS));
    };
    let seconds = value.as_u64().ok_or_else(|| {
        OrbitError::InvalidInput("worker_timeout_seconds must be a positive integer".to_string())
    })?;
    if seconds == 0 {
        return Err(OrbitError::InvalidInput(
            "worker_timeout_seconds must be at least 1".to_string(),
        ));
    }
    Ok(Duration::from_secs(seconds))
}

fn parse_worker_wait_observations(output: &Value) -> Result<Vec<WorkerObservation>, OrbitError> {
    let results = output
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            OrbitError::Execution("orbit.pipeline.wait returned no results array".to_string())
        })?;
    results
        .iter()
        .map(|entry| {
            let run_id = entry
                .get("run_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    OrbitError::Execution(
                        "orbit.pipeline.wait result entry has no run_id".to_string(),
                    )
                })?
                .to_string();
            let status = entry.get("status").and_then(Value::as_str).ok_or_else(|| {
                OrbitError::Execution(format!(
                    "orbit.pipeline.wait result for run '{run_id}' has no status"
                ))
            })?;
            let state = match status {
                "succeeded" => WorkerRunState::Succeeded,
                "failed" => WorkerRunState::Failed {
                    code: "WORKER_NON_SUCCESS",
                    message: worker_failure_message(entry, status),
                },
                "cancelled" => WorkerRunState::Failed {
                    code: "WORKER_CANCELLED",
                    message: worker_failure_message(entry, status),
                },
                "pending" | "running" | "timeout" => WorkerRunState::Incomplete,
                other => {
                    return Err(OrbitError::Execution(format!(
                        "orbit.pipeline.wait returned unknown status '{other}' for run '{run_id}'"
                    )));
                }
            };
            Ok(WorkerObservation { run_id, state })
        })
        .collect()
}

fn worker_failure_message(entry: &Value, status: &str) -> String {
    entry
        .get("error")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("parallel worker completed with status '{status}'"))
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
