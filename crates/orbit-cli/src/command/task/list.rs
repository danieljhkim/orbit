use std::collections::BTreeSet;

use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime, TaskPriority, TaskStatus};
use serde_json::{Value, json};

use crate::command::Execute;

use super::output::{
    print_task_locks, print_task_table, task_lock_to_json, task_to_json, task_to_signal_json,
};

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit task list\n  orbit task list --all\n  orbit task list --status backlog\n  orbit task list --status in-progress,review\n  orbit task list --priority high\n  orbit task list --parent T12345678-123456\n  orbit task list --json"
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
    /// Filter to subtasks belonging to a parent task
    #[arg(long = "parent")]
    pub parent_id: Option<String>,
    /// Filter by batch ID
    #[arg(long)]
    pub batch_id: Option<String>,
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
        let parent_id = self.parent_id;
        let batch_id = self.batch_id;

        let all_tasks = runtime.list_tasks()?;
        let active_statuses = [TaskStatus::Backlog, TaskStatus::InProgress];
        let status_filter =
            default_task_list_status_filter(all, &status, batch_id.as_deref(), &active_statuses);

        let tasks: Vec<_> = all_tasks
            .into_iter()
            .filter(|t| status_filter.is_empty() || status_filter.contains(&t.status))
            .filter(|t| priority.is_none_or(|p| t.priority == p))
            .filter(|t| {
                parent_id
                    .as_deref()
                    .is_none_or(|p| t.parent_id.as_deref() == Some(p))
            })
            .filter(|t| {
                batch_id
                    .as_deref()
                    .is_none_or(|b| t.batch_id.as_deref() == Some(b))
            })
            .collect();

        if self.ops {
            let json_tasks: Vec<Value> = tasks.iter().map(task_to_signal_json).collect();
            crate::output::json::print_pretty(&Value::Array(json_tasks))
        } else if self.json {
            let json_tasks: Vec<Value> = tasks.iter().map(task_to_json).collect();
            crate::output::json::print_pretty(&Value::Array(json_tasks))
        } else {
            print_task_table(&tasks, self.full);
            Ok(())
        }
    }
}

fn default_task_list_status_filter<'a>(
    all: bool,
    status: &'a [TaskStatus],
    batch_id: Option<&str>,
    active_statuses: &'a [TaskStatus],
) -> &'a [TaskStatus] {
    if all {
        &[]
    } else if !status.is_empty() {
        status
    } else if batch_id.is_some() {
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

        if self.json {
            let json_by_task: Vec<Value> = tasks.iter().map(task_lock_to_json).collect();
            crate::output::json::print_pretty(&json!({
                "locked_files": locked_files.iter().cloned().collect::<Vec<_>>(),
                "by_task": json_by_task,
                "total_locked": locked_files.len(),
                "total_tasks": tasks.len(),
            }))
        } else {
            print_task_locks(&tasks, &locked_files);
            Ok(())
        }
    }
}

fn task_lock_status_rank(status: TaskStatus) -> u8 {
    match status {
        TaskStatus::InProgress => 0,
        TaskStatus::Review => 1,
        _ => 2,
    }
}
