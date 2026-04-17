use clap::Args;
use orbit_core::{OrbitError, OrbitRuntime, TaskStatus};
use serde_json::Value;
use serde_json::json;

use crate::command::Execute;

/// Statuses considered "active" for the prune-context backfill.
///
/// Done / Archived / Rejected tasks are intentionally skipped — they are
/// historical records and re-saving them would mutate audit trails for tasks
/// nobody is going to execute again.
const PRUNE_CONTEXT_ACTIVE_STATUSES: &[TaskStatus] = &[
    TaskStatus::Proposed,
    TaskStatus::Backlog,
    TaskStatus::Someday,
    TaskStatus::InProgress,
    TaskStatus::Blocked,
    TaskStatus::Review,
];

#[derive(Args)]
pub struct TaskPruneContextArgs {
    /// Apply pruning to disk. Without this flag, only a dry-run report is printed.
    #[arg(long)]
    pub write: bool,
    /// Restrict pruning to a specific status (repeatable). Defaults to all
    /// active statuses (proposed, backlog, someday, in-progress, blocked, review).
    #[arg(long = "status", value_enum)]
    pub statuses: Vec<TaskStatus>,
    /// Emit a JSON report instead of human-readable output.
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskPruneContextArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let allowed_statuses: Vec<TaskStatus> = if self.statuses.is_empty() {
            PRUNE_CONTEXT_ACTIVE_STATUSES.to_vec()
        } else {
            self.statuses.clone()
        };

        let tasks = runtime.list_tasks()?;
        let mut report = Vec::<Value>::new();
        let mut total_dropped = 0usize;
        let mut tasks_with_drops = 0usize;
        let mut tasks_written = 0usize;

        for task in tasks {
            if !allowed_statuses.contains(&task.status) {
                continue;
            }
            if task.context_files.is_empty() {
                continue;
            }
            let dropped = runtime.dry_run_prune_context_files(&task);
            if dropped.is_empty() {
                continue;
            }

            tasks_with_drops += 1;
            total_dropped += dropped.len();

            let written = if self.write {
                runtime.update_task(
                    &task.id,
                    orbit_core::command::task::TaskUpdateParams {
                        context_files: Some(task.context_files.clone()),
                        ..Default::default()
                    },
                )?;
                tasks_written += 1;
                true
            } else {
                false
            };

            report.push(json!({
                "id": task.id,
                "status": task.status,
                "dropped": dropped,
                "written": written,
            }));
        }

        if self.json {
            let payload = json!({
                "tasks_inspected": report.len(),
                "tasks_with_drops": tasks_with_drops,
                "total_dropped": total_dropped,
                "tasks_written": tasks_written,
                "dry_run": !self.write,
                "tasks": report,
            });
            return crate::output::json::print_pretty(&payload);
        }

        if report.is_empty() {
            println!("No active tasks have stale context_files entries.");
            return Ok(());
        }

        for entry in &report {
            let id = entry.get("id").and_then(Value::as_str).unwrap_or("");
            let dropped = entry
                .get("dropped")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!("{id}: {dropped}");
        }
        let action = if self.write { "pruned" } else { "would prune" };
        println!("\n{action} {total_dropped} entries across {tasks_with_drops} task(s).");
        if !self.write {
            println!("Re-run with --write to apply.");
        }
        Ok(())
    }
}
