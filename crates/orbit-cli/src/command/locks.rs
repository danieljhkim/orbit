//! `orbit locks ...` — task file-lock administration.
//!
//! `list` renders the file-lock projection over active (in-progress/review)
//! tasks. `release` clears a stale reservation by ID.
//!
//! Task lock reservations auto-release in workflow pipelines; `release` is the
//! operator escape hatch for a stale reservation that wedges a run. The
//! underlying `orbit.task.locks.release` tool is inactive on the agent MCP
//! surface, so `release` reaches it through the admin `runtime.run_tool`
//! bypass (mirrors `orbit adr list`, ORB-00289).

use std::collections::BTreeSet;

use clap::{Args, Subcommand};
use orbit_core::{OrbitError, OrbitRuntime, TaskStatus};
use serde_json::{Map, Value, json};

use crate::command::Execute;
use crate::command::task::output::{print_task_locks, task_lock_to_json};

#[derive(Args)]
#[command(about = "Inspect and release task file locks")]
pub struct LocksCommand {
    #[command(subcommand)]
    pub command: LocksSubcommand,
}

impl Execute for LocksCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum LocksSubcommand {
    /// Show files locked by active (in-progress/review) tasks
    List(LocksListArgs),
    /// Release a stale task lock reservation (operator/admin escape hatch)
    Release(LocksReleaseArgs),
}

impl Execute for LocksSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            LocksSubcommand::List(args) => args.execute(runtime),
            LocksSubcommand::Release(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
#[command(about = "Show files locked by active (in-progress/review) tasks")]
pub struct LocksListArgs {
    /// Output the lock projection as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for LocksListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let mut tasks: Vec<_> = runtime
            .list_tasks()?
            .into_iter()
            .filter(|task| matches!(task.status, TaskStatus::InProgress | TaskStatus::Review))
            .collect();
        tasks.sort_by_key(|task| {
            (
                lock_status_rank(task.status),
                task.created_at,
                task.id.clone(),
            )
        });

        let locked_files: BTreeSet<String> = tasks
            .iter()
            .flat_map(|task| task.context_files.iter().cloned())
            .collect();

        if self.json {
            let by_task: Vec<Value> = tasks.iter().map(task_lock_to_json).collect();
            crate::output::json::print_pretty(&json!({
                "locked_files": locked_files.iter().cloned().collect::<Vec<_>>(),
                "by_task": by_task,
                "total_locked": locked_files.len(),
                "total_tasks": tasks.len(),
            }))
        } else {
            print_task_locks(&tasks, &locked_files);
            Ok(())
        }
    }
}

fn lock_status_rank(status: TaskStatus) -> u8 {
    match status {
        TaskStatus::InProgress => 0,
        TaskStatus::Review => 1,
        _ => 2,
    }
}

#[derive(Args)]
#[command(about = "Release a stale task lock reservation (operator/admin escape hatch)")]
pub struct LocksReleaseArgs {
    /// Reservation ID to release (from the reservation store / debug tooling)
    pub reservation_id: String,
}

impl Execute for LocksReleaseArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let mut input = Map::new();
        input.insert(
            "reservation_id".to_string(),
            Value::String(self.reservation_id),
        );
        let value = runtime.run_tool("orbit.task.locks.release", Value::Object(input))?;
        crate::output::json::print_pretty(&value)
    }
}
