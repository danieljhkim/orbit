use std::collections::{HashSet, VecDeque};
use std::sync::mpsc;

use orbit_types::{JobRunState, OrbitError, Task, TaskStatus};
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskHost};

const DEFAULT_PARALLEL_BASE: &str = "orbit-parallel-work-branch";
const DEFAULT_PARALLELISM: usize = 4;
const PARALLEL_WORKER_JOB_ID: &str = "job_parallel_task_worker";
const PARALLEL_FINALIZE_JOB_ID: &str = "job_parallel_task_finalize";

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
    let mut pending = VecDeque::from(selected_tasks.clone());

    let mut launched = 0usize;
    let mut succeeded = 0usize;
    let mut failed = 0usize;
    let skipped = 0usize;
    let mut failures = Vec::new();
    let mut ready_for_finalize = HashSet::new();

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
                let launch_base = base.clone();
                active.push(task);
                launched += 1;

                scope.spawn(move || {
                    let result = host.run_job_now_with_input_debug(
                        PARALLEL_WORKER_JOB_ID,
                        json!({
                            "task_id": task_id.clone(),
                            "base": launch_base,
                        }),
                        false,
                    );
                    let _ = tx.send(WorkerOutcome { task_id, result });
                });
            }

            if active.is_empty() {
                continue;
            }

            let outcome = rx.recv().map_err(|error| {
                OrbitError::Execution(format!(
                    "parallel task pipeline lost worker coordination channel: {error}"
                ))
            })?;
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
                            ready_for_finalize.insert(outcome.task_id);
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
                    let _ = host.release_file_locks(&outcome.task_id);
                }
                Err(error) => {
                    failed += 1;
                    failures.push(json!({
                        "task_id": outcome.task_id,
                        "error": error.to_string(),
                    }));
                    let _ = host.release_file_locks(&outcome.task_id);
                }
            }
        }

        Ok(())
    })?;

    for task in selected_tasks {
        if !ready_for_finalize.contains(&task.task_id) {
            continue;
        }
        let task_id = task.task_id;
        match host.run_job_now_with_input_debug(
            PARALLEL_FINALIZE_JOB_ID,
            json!({
                "task_id": task_id.clone(),
                "base": base.clone(),
            }),
            false,
        ) {
            Ok(result) if result.state == JobRunState::Success => {
                succeeded += 1;
            }
            Ok(result) => {
                failed += 1;
                failures.push(json!({
                    "task_id": task_id,
                    "error": format!(
                        "parallel finalization completed in non-success state '{}'",
                        result.state
                    ),
                }));
            }
            Err(error) => {
                failed += 1;
                failures.push(json!({
                    "task_id": task_id,
                    "error": format!("parallel finalization failed: {error}"),
                }));
            }
        }
    }

    Ok(json!({
        "launched": launched,
        "succeeded": succeeded,
        "failed": failed,
        "skipped": skipped,
        "failures": failures,
    }))
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
    let Some(task_ids) = input.get("task_ids").and_then(Value::as_array) else {
        return Err(OrbitError::InvalidInput(
            "parallel_dispatch_tasks requires input.task_ids".to_string(),
        ));
    };

    let mut seen = HashSet::new();
    let mut selected = Vec::with_capacity(task_ids.len());
    for task_id in task_ids {
        let task_id = task_id
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                OrbitError::InvalidInput(
                    "parallel_dispatch_tasks.task_ids must contain non-empty strings".to_string(),
                )
            })?;
        if !seen.insert(task_id.to_string()) {
            return Err(OrbitError::InvalidInput(format!(
                "parallel task batch contains duplicate task id '{task_id}'"
            )));
        }
        let task = host.get_task(task_id)?;
        if task.status != TaskStatus::Backlog {
            return Err(OrbitError::InvalidInput(format!(
                "parallel task batch requires backlog tasks; '{task_id}' is '{}'",
                task.status
            )));
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
    use super::{PendingTask, find_launchable_index, paths_conflict, validate_selected_group};
    use std::collections::VecDeque;

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
