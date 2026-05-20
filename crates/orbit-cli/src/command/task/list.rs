use std::collections::BTreeSet;

use clap::{ArgAction, Args};
use orbit_core::{
    ExternalRef, OrbitError, OrbitRuntime, TaskPriority, TaskStatus, TaskType,
    build_task_status_index, task_dependencies_ready, task_selectors_contain_path,
};
use serde_json::{Value, json};

use crate::command::Execute;

use super::output::{
    print_task_locks, print_task_table, task_lock_to_json, task_to_json, task_to_signal_json,
};

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit task list\n  orbit task list --all\n  orbit task list --status backlog\n  orbit task list --status friction\n  orbit task list --status in-progress,review\n  orbit task list --type feature\n  orbit task list --priority high\n  orbit task list --parent T12345678-123456\n  orbit task list --ref jira:ENG-1234\n  orbit task list --has-ref jira\n  orbit task list --tag perf --tag bench\n  orbit task list --path src/auth/login.rs\n  orbit task list --json"
)]
pub struct TaskListArgs {
    /// Filter by one or more statuses (comma-separated). Defaults to backlog,in-progress.
    #[arg(long, value_enum, value_delimiter = ',')]
    pub status: Vec<TaskStatus>,
    /// Show all tasks regardless of status
    #[arg(long, conflicts_with = "status")]
    pub all: bool,
    /// Filter by priority level (low, medium, high)
    #[arg(long, value_enum)]
    pub priority: Option<TaskPriority>,
    /// Filter by task type (feature, bug, refactor, chore)
    #[arg(long = "type", value_enum)]
    pub task_type: Option<TaskType>,
    /// Filter to subtasks belonging to a parent task
    #[arg(long = "parent")]
    pub parent_id: Option<String>,
    /// Filter by job run ID
    #[arg(long)]
    pub job_run_id: Option<String>,
    /// Filter by tag. Repeat for AND semantics.
    #[arg(long = "tag", action = ArgAction::Append, value_delimiter = ',')]
    pub tags: Vec<String>,
    /// Filter by exact external reference in `system:id` form
    #[arg(long = "ref")]
    pub external_ref: Option<String>,
    /// Filter by external reference system
    #[arg(long = "has-ref")]
    pub has_ref: Option<String>,
    /// Keep only tasks whose dependencies are already satisfied
    #[arg(long)]
    pub ready: bool,
    /// Filter to tasks whose `context_files` selectors apply to this path.
    /// Selector forms supported: `file:`, `dir:`, `symbol:`, and bare paths.
    /// Bidirectional containment — passing a directory matches every
    /// selector under it.
    #[arg(long)]
    pub path: Option<String>,
    /// Output full task objects as JSON
    #[arg(long)]
    pub json: bool,
    /// Output signal-tier JSON (id, title, type, status, priority only)
    #[arg(long)]
    pub ops: bool,
    /// Show all table columns in text output
    #[arg(long)]
    pub full: bool,
}

impl Execute for TaskListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let all = self.all;
        let status = self.status;
        let priority = self.priority;
        let task_type = self.task_type;
        let parent_id = self.parent_id;
        let job_run_id = self.job_run_id;
        let tags = self.tags;
        let path = self.path;
        let external_ref = self
            .external_ref
            .as_deref()
            .map(ExternalRef::parse_key)
            .transpose()?;
        let has_ref_system = self
            .has_ref
            .map(|system| validate_external_ref_system(&system))
            .transpose()?;
        let ready = self.ready;

        let tasks_matching_tags = runtime.list_tasks_by_tags(&tags)?;
        let status_by_id = build_task_status_index(&runtime.list_tasks()?);
        let active_statuses = [TaskStatus::Backlog, TaskStatus::InProgress];
        let status_filter =
            default_task_list_status_filter(all, &status, job_run_id.as_deref(), &active_statuses);

        let tasks: Vec<_> = tasks_matching_tags
            .into_iter()
            .filter(|t| status_filter.is_empty() || status_filter.contains(&t.status))
            .filter(|t| priority.is_none_or(|p| t.priority == p))
            .filter(|t| task_type.is_none_or(|kind| t.task_type == kind))
            .filter(|t| {
                parent_id
                    .as_deref()
                    .is_none_or(|p| t.parent_id() == Some(p))
            })
            .filter(|t| {
                job_run_id
                    .as_deref()
                    .is_none_or(|value| t.job_run_id.as_deref() == Some(value))
            })
            .filter(|t| {
                external_ref.as_ref().is_none_or(|external_ref| {
                    t.external_refs.iter().any(|candidate| {
                        candidate.system == external_ref.system && candidate.id == external_ref.id
                    })
                })
            })
            .filter(|t| {
                has_ref_system.as_deref().is_none_or(|system| {
                    t.external_refs
                        .iter()
                        .any(|candidate| candidate.system == system)
                })
            })
            .filter(|t| !ready || task_dependencies_ready(t, &status_by_id))
            .filter(|t| {
                path.as_deref()
                    .is_none_or(|p| task_selectors_contain_path(&t.context_files, p))
            })
            .collect();

        if self.ops {
            let json_tasks: Vec<Value> = tasks.iter().map(task_to_signal_json).collect();
            crate::output::json::print_pretty(&Value::Array(json_tasks))
        } else if self.json {
            let json_tasks: Vec<Value> = tasks
                .iter()
                .map(|task| task_to_json(task, &status_by_id))
                .collect();
            crate::output::json::print_pretty(&Value::Array(json_tasks))
        } else {
            print_task_table(&tasks, self.full);
            Ok(())
        }
    }
}

fn validate_external_ref_system(system: &str) -> Result<String, OrbitError> {
    ExternalRef::validate_system(system)
}

fn default_task_list_status_filter<'a>(
    all: bool,
    status: &'a [TaskStatus],
    job_run_id: Option<&str>,
    active_statuses: &'a [TaskStatus],
) -> &'a [TaskStatus] {
    if all {
        &[]
    } else if !status.is_empty() {
        status
    } else if job_run_id.is_some() {
        &[]
    } else {
        active_statuses
    }
}

#[derive(Args)]
pub struct TaskLocksArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskLocksArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        if self.json {
            crate::output::json::print_pretty(&task_locks_json(runtime)?)
        } else {
            let (tasks, locked_files) = task_locks(runtime)?;
            print_task_locks(&tasks, &locked_files);
            Ok(())
        }
    }
}

pub(crate) fn task_locks_json(runtime: &OrbitRuntime) -> Result<Value, OrbitError> {
    let (tasks, locked_files) = task_locks(runtime)?;
    let json_by_task: Vec<Value> = tasks.iter().map(task_lock_to_json).collect();
    Ok(json!({
        "locked_files": locked_files.iter().cloned().collect::<Vec<_>>(),
        "by_task": json_by_task,
        "total_locked": locked_files.len(),
        "total_tasks": tasks.len(),
    }))
}

fn task_locks(
    runtime: &OrbitRuntime,
) -> Result<(Vec<orbit_core::Task>, BTreeSet<String>), OrbitError> {
    let mut tasks: Vec<_> = runtime
        .list_tasks()?
        .into_iter()
        .filter(|task| matches!(task.status, TaskStatus::InProgress | TaskStatus::Review))
        .collect();

    tasks.sort_by_key(|task| {
        (
            task_lock_status_rank(task.status),
            task.created_at,
            task.id.clone(),
        )
    });

    let locked_files: BTreeSet<String> = tasks
        .iter()
        .flat_map(|task| task.context_files.iter().cloned())
        .collect();

    Ok((tasks, locked_files))
}

fn task_lock_status_rank(status: TaskStatus) -> u8 {
    match status {
        TaskStatus::InProgress => 0,
        TaskStatus::Review => 1,
        _ => 2,
    }
}
