use clap::{ArgAction, Args};
use orbit_core::{OrbitError, OrbitRuntime, TaskStatus};
use serde_json::{Value, json};

use crate::command::Execute;

use super::output::{print_task_table, task_to_json_for_runtime};

#[derive(Args)]
pub struct TaskStartArgs {
    /// Task ID
    pub id: String,
    /// Optional lifecycle note (records proposal approval when starting proposed work)
    #[arg(long)]
    pub note: Option<String>,
    /// Append a task comment
    #[arg(long)]
    pub comment: Option<String>,
    /// Explicit agent name to persist on the task artifact
    #[arg(long)]
    pub agent: Option<String>,
    /// Explicit agent model to persist on the task artifact
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskStartArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let task = runtime.start_task_with_identity(
            &self.id,
            self.note,
            self.comment,
            self.agent,
            self.model,
        )?;
        if self.json {
            crate::output::json::print_pretty(&task_to_json_for_runtime(runtime, &task)?)
        } else {
            println!("Started task '{}'", task.id);
            Ok(())
        }
    }
}

#[derive(Args)]
#[command(about = "Approve a task (proposed/friction -> backlog, or review -> done)")]
pub struct TaskApproveArgs {
    /// Task ID(s) to approve (one or more)
    #[arg(num_args = 1.., required_unless_present = "all_proposed", conflicts_with = "all_proposed")]
    pub ids: Vec<String>,
    /// Approve all tasks currently in proposed status
    #[arg(long)]
    pub all_proposed: bool,
    /// Skip confirmation prompt (for use with --all-proposed)
    #[arg(long)]
    pub yes: bool,
    /// Optional approval note
    #[arg(long)]
    pub note: Option<String>,
    /// Append a task comment
    #[arg(long)]
    pub comment: Option<String>,
    /// Explicit agent name to persist on the task artifact
    #[arg(long)]
    pub agent: Option<String>,
    /// Explicit agent model to persist on the task artifact
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskApproveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let ids = if self.all_proposed {
            select_proposed_task_ids(runtime, self.yes, "approved", self.json)?
        } else {
            self.ids
        };

        let bulk = self.all_proposed || ids.len() > 1;
        if self.json {
            let mut results = Vec::new();
            for id in &ids {
                let task = runtime.approve_task_with_identity(
                    id,
                    self.note.clone(),
                    self.comment.clone(),
                    self.agent.clone(),
                    self.model.clone(),
                )?;
                results.push(task_to_json_for_runtime(runtime, &task)?);
            }
            if bulk {
                crate::output::json::print_pretty(&Value::Array(results))
            } else {
                crate::output::json::print_pretty(results.first().unwrap_or(&Value::Null))
            }
        } else {
            for id in &ids {
                let task = runtime.approve_task_with_identity(
                    id,
                    self.note.clone(),
                    self.comment.clone(),
                    self.agent.clone(),
                    self.model.clone(),
                )?;
                println!("Approved task '{}'", task.id);
            }
            Ok(())
        }
    }
}

#[derive(Args)]
#[command(about = "Reject a task (proposed/friction/review/backlog/in-progress -> rejected)")]
pub struct TaskRejectArgs {
    /// Task ID(s) to reject (one or more)
    #[arg(num_args = 1.., required_unless_present = "all_proposed", conflicts_with = "all_proposed")]
    pub ids: Vec<String>,
    /// Reject all tasks currently in proposed status
    #[arg(long)]
    pub all_proposed: bool,
    /// Skip confirmation prompt (for use with --all-proposed)
    #[arg(long)]
    pub yes: bool,
    /// Rejection note
    #[arg(long)]
    pub note: String,
    /// Append a task comment
    #[arg(long)]
    pub comment: Option<String>,
    /// Explicit agent name to persist on the task artifact
    #[arg(long)]
    pub agent: Option<String>,
    /// Explicit agent model to persist on the task artifact
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskRejectArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let ids = if self.all_proposed {
            select_proposed_task_ids(runtime, self.yes, "rejected", self.json)?
        } else {
            self.ids
        };

        let bulk = self.all_proposed || ids.len() > 1;
        if self.json {
            let mut results = Vec::new();
            for id in &ids {
                let task = runtime.reject_task_with_identity(
                    id,
                    self.note.clone(),
                    self.comment.clone(),
                    self.agent.clone(),
                    self.model.clone(),
                )?;
                results.push(task_to_json_for_runtime(runtime, &task)?);
            }
            if bulk {
                crate::output::json::print_pretty(&Value::Array(results))
            } else {
                crate::output::json::print_pretty(results.first().unwrap_or(&Value::Null))
            }
        } else {
            for id in &ids {
                let task = runtime.reject_task_with_identity(
                    id,
                    self.note.clone(),
                    self.comment.clone(),
                    self.agent.clone(),
                    self.model.clone(),
                )?;
                println!("Rejected task '{}'", task.id);
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct TaskArchiveArgs {
    /// Task ID
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskArchiveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.archive_task(&self.id)?;
        if self.json {
            let task = runtime.get_task(&self.id)?;
            crate::output::json::print_pretty(&task_to_json_for_runtime(runtime, &task)?)
        } else {
            println!("Archived task '{}'", self.id);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct TaskUnarchiveArgs {
    /// Task ID
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskUnarchiveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.unarchive_task(&self.id)?;
        if self.json {
            let task = runtime.get_task(&self.id)?;
            crate::output::json::print_pretty(&task_to_json_for_runtime(runtime, &task)?)
        } else {
            println!("Unarchived task '{}'", self.id);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct TaskDeleteArgs {
    /// Task ID
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
    /// Force deletion without status guard (required for non-proposed/friction/rejected tasks)
    #[arg(long)]
    pub force: bool,
}

impl Execute for TaskDeleteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.delete_task_guarded(&self.id, self.force)?;
        if self.json {
            crate::output::json::print_pretty(&json!({
                "id": self.id,
                "deleted": true,
            }))
        } else {
            println!("Deleted task '{}'", self.id);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct TaskSearchArgs {
    /// Search query
    pub query: String,
    /// Filter by tag. Repeat for AND semantics.
    #[arg(long = "tag", action = ArgAction::Append, value_delimiter = ',')]
    pub tags: Vec<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskSearchArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let tasks = runtime.search_tasks_filtered(&self.query, &self.tags)?;

        if self.json {
            let json_tasks: Vec<Value> = tasks
                .iter()
                .map(|task| task_to_json_for_runtime(runtime, task))
                .collect::<Result<_, _>>()?;
            crate::output::json::print_pretty(&Value::Array(json_tasks))
        } else {
            print_task_table(&tasks, false);
            Ok(())
        }
    }
}

fn select_proposed_task_ids(
    runtime: &OrbitRuntime,
    yes: bool,
    action: &str,
    json: bool,
) -> Result<Vec<String>, OrbitError> {
    let proposed =
        runtime.list_tasks_filtered(Some(TaskStatus::Proposed), None, None, None, None, None)?;
    if proposed.is_empty() {
        if json {
            eprintln!("No proposed tasks found.");
        } else {
            println!("No proposed tasks found.");
        }
        return Ok(Vec::new());
    }
    if !yes {
        use std::io::Write;
        if json {
            eprintln!(
                "The following {} task(s) will be {}:",
                proposed.len(),
                action
            );
            for task in &proposed {
                eprintln!("  {} — {}", task.id, task.title);
            }
            eprint!("Proceed? [y/N] ");
            std::io::stderr()
                .flush()
                .map_err(|e| OrbitError::Io(e.to_string()))?;
        } else {
            println!(
                "The following {} task(s) will be {}:",
                proposed.len(),
                action
            );
            for task in &proposed {
                println!("  {} — {}", task.id, task.title);
            }
            print!("Proceed? [y/N] ");
            std::io::stdout()
                .flush()
                .map_err(|e| OrbitError::Io(e.to_string()))?;
        }
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .map_err(|e| OrbitError::Io(e.to_string()))?;
        if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
            if json {
                eprintln!("Aborted.");
            } else {
                println!("Aborted.");
            }
            return Ok(Vec::new());
        }
    }
    Ok(proposed.into_iter().map(|t| t.id).collect())
}
