use clap::{Args, Subcommand};
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
use orbit_core::{OrbitError, OrbitRuntime, TaskComplexity, TaskPriority, TaskStatus, TaskType};
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
pub struct TaskCommand {
    #[command(subcommand)]
    pub command: TaskSubcommand,
}

impl Execute for TaskCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum TaskSubcommand {
    /// Create a new task
    Add(TaskAddArgs),
    /// List tasks with optional filters
    List(TaskListArgs),
    /// Show detailed information about a task
    Show(TaskShowArgs),
    /// Update task fields
    Update(TaskUpdateArgs),
    /// Start work on a task, approving proposed work when needed
    Start(TaskStartArgs),
    /// Approve a task (proposed → backlog, or review → done)
    Approve(TaskApproveArgs),
    /// Reject a task (proposed → archived, or review → backlog)
    Reject(TaskRejectArgs),
    /// Archive a task
    Archive(TaskArchiveArgs),
    /// Unarchive a task (archived → backlog)
    Unarchive(TaskUnarchiveArgs),
    /// Delete a task permanently
    Delete(TaskDeleteArgs),
    /// Search tasks by title or description
    Search(TaskSearchArgs),
}

impl Execute for TaskSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            TaskSubcommand::Add(args) => args.execute(runtime),
            TaskSubcommand::List(args) => args.execute(runtime),
            TaskSubcommand::Show(args) => args.execute(runtime),
            TaskSubcommand::Update(args) => args.execute(runtime),
            TaskSubcommand::Start(args) => args.execute(runtime),
            TaskSubcommand::Approve(args) => args.execute(runtime),
            TaskSubcommand::Reject(args) => args.execute(runtime),
            TaskSubcommand::Archive(args) => args.execute(runtime),
            TaskSubcommand::Unarchive(args) => args.execute(runtime),
            TaskSubcommand::Delete(args) => args.execute(runtime),
            TaskSubcommand::Search(args) => args.execute(runtime),
        }
    }
}

// --- Add ---

#[derive(Args)]
pub struct TaskAddArgs {
    /// Task title
    #[arg(long)]
    pub title: String,
    /// Task description
    #[arg(long)]
    pub description: String,
    /// Task plan payload (agent planning input)
    #[arg(long, alias = "instructions")]
    pub plan: String,
    /// Append an initial task comment
    #[arg(long)]
    pub comment: Option<String>,
    /// Comma-separated context file paths
    #[arg(long, default_value = "")]
    pub context: String,
    /// Repository workspace path
    #[arg(long)]
    pub workspace: String,
    /// Priority level
    #[arg(long, value_enum, default_value_t = TaskPriority::Medium)]
    pub priority: TaskPriority,
    /// Task complexity
    #[arg(long, value_enum)]
    pub complexity: Option<TaskComplexity>,
    /// Task type
    #[arg(long = "type", value_enum, default_value_t = TaskType::Task)]
    pub task_type: TaskType,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let task = runtime.add_task(TaskAddParams {
            title: self.title,
            description: self.description,
            plan: self.plan,
            comment: self.comment,
            context_files: parse_context_csv(&self.context),
            workspace_path: Some(self.workspace),
            priority: self.priority,
            complexity: self.complexity,
            task_type: self.task_type,
        })?;

        if self.json {
            crate::output::json::print_pretty(&task_to_json(&task))
        } else {
            println!("{}", task.id);
            Ok(())
        }
    }
}

// --- List ---

#[derive(Args)]
#[command(
    after_help = "Examples:\n  orbit task list\n  orbit task list --all\n  orbit task list --status backlog\n  orbit task list --status in-progress,review\n  orbit task list --priority high\n  orbit task list --json"
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
    /// Output full task objects as JSON
    #[arg(long)]
    pub json: bool,
    /// Output signal-tier JSON (id, title, type, status, priority only)
    #[arg(long)]
    pub ops: bool,
}

impl Execute for TaskListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let all = self.all;
        let status = self.status;
        let priority = self.priority;

        let all_tasks = runtime.list_tasks()?;
        let active_statuses = [TaskStatus::Backlog, TaskStatus::InProgress];
        let status_filter: &[TaskStatus] = if all {
            &[]
        } else if !status.is_empty() {
            &status
        } else {
            &active_statuses
        };

        let tasks: Vec<_> = all_tasks
            .into_iter()
            .filter(|t| status_filter.is_empty() || status_filter.contains(&t.status))
            .filter(|t| priority.is_none_or(|p| t.priority == p))
            .collect();

        if self.ops {
            let json_tasks: Vec<Value> = tasks.iter().map(task_to_signal_json).collect();
            crate::output::json::print_pretty(&Value::Array(json_tasks))
        } else if self.json {
            let json_tasks: Vec<Value> = tasks.iter().map(task_to_json).collect();
            crate::output::json::print_pretty(&Value::Array(json_tasks))
        } else {
            print_task_table(&tasks);
            Ok(())
        }
    }
}

// --- Show ---

#[derive(Args)]
pub struct TaskShowArgs {
    /// Task ID
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let task = runtime.get_task(&self.id)?;

        if self.json {
            crate::output::json::print_pretty(&task_to_json(&task))
        } else {
            use crate::output::color::{bold, dimmed, priority_color, status_color};
            println!("{} {}", bold("ID:"), task.id);
            println!("{} {}", bold("Title:"), task.title);
            println!(
                "{} {}",
                bold("Status:"),
                status_color(&task.status.to_string())
            );
            println!(
                "{} {}",
                bold("Priority:"),
                priority_color(&task.priority.to_string())
            );
            if let Some(complexity) = task.complexity {
                println!("{} {}", bold("Complexity:"), complexity);
            }
            println!("{} {}", bold("Type:"), task.task_type);
            if !task.description.is_empty() {
                println!("{} {}", bold("Description:"), task.description);
            }
            if !task.plan.is_empty() {
                println!("{} {}", bold("Plan:"), task.plan);
            }
            if !task.execution_summary.is_empty() {
                println!("{} {}", bold("Execution Summary:"), task.execution_summary);
            }
            if !task.comments.is_empty() {
                println!("{}", bold("Comments:"));
                for comment in &task.comments {
                    println!(
                        "  {} {}: {}",
                        dimmed(&format!("[{}]", comment.at.to_rfc3339())),
                        comment.by,
                        comment.message
                    );
                }
            }
            if !task.context_files.is_empty() {
                println!("{} {}", bold("Context:"), task.context_files.join(", "));
            }
            if let Some(ref workspace_path) = task.workspace_path {
                println!("{} {}", bold("Workspace:"), workspace_path);
            }
            if let Some(ref assigned_to) = task.assigned_to {
                println!("{} {}", bold("Assigned To:"), assigned_to);
            }
            if let Some(ref created_by) = task.created_by {
                println!("{} {}", bold("Created By:"), created_by);
            }
            if !task.history.is_empty() {
                println!("{}", bold("History:"));
                for entry in &task.history {
                    if let Some(note) = &entry.note {
                        println!(
                            "  {} {}: {} ({})",
                            dimmed(&format!("[{}]", entry.at.to_rfc3339())),
                            entry.by,
                            entry.event,
                            note
                        );
                    } else {
                        println!(
                            "  {} {}: {}",
                            dimmed(&format!("[{}]", entry.at.to_rfc3339())),
                            entry.by,
                            entry.event
                        );
                    }
                }
            }
            if let Some(ref branch) = task.branch {
                println!("{} {}", bold("Branch:"), branch);
            }
            if let Some(ref pr_number) = task.pr_number {
                println!("{} {}", bold("PR Number:"), pr_number);
            }
            if let Some(ref proposed_by) = task.proposed_by {
                println!("{} {}", bold("Proposed By:"), proposed_by);
            }
            println!(
                "{} {}",
                bold("Created:"),
                dimmed(&task.created_at.to_rfc3339())
            );
            println!(
                "{} {}",
                bold("Updated:"),
                dimmed(&task.updated_at.to_rfc3339())
            );
            Ok(())
        }
    }
}

// --- Update ---

#[derive(Args)]
pub struct TaskUpdateArgs {
    /// Task ID
    pub id: String,
    /// New title
    #[arg(long)]
    pub title: Option<String>,
    /// New description (empty string clears)
    #[arg(long)]
    pub description: Option<String>,
    /// New task plan (empty string clears)
    #[arg(long, alias = "instructions")]
    pub plan: Option<String>,
    /// New execution summary (empty string clears)
    #[arg(long)]
    pub execution_summary: Option<String>,
    /// Append a task comment
    #[arg(long)]
    pub comment: Option<String>,
    /// New status
    #[arg(long, value_enum)]
    pub status: Option<TaskUpdateStatusArg>,
    /// Git branch name (empty string clears)
    #[arg(long)]
    pub branch: Option<String>,
    /// Pull request number (empty string clears)
    #[arg(long)]
    pub pr_number: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskUpdateArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let branch = self.branch.map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        });
        let pr_number = self.pr_number.map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        });

        let task = runtime.update_task(
            &self.id,
            TaskUpdateParams {
                title: self.title,
                description: self.description,
                plan: self.plan,
                execution_summary: self.execution_summary,
                comment: self.comment,
                status: self.status.map(Into::into),
                branch,
                pr_number,
            },
        )?;

        if self.json {
            crate::output::json::print_pretty(&task_to_json(&task))
        } else {
            println!("Updated task '{}'", task.id);
            Ok(())
        }
    }
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum TaskUpdateStatusArg {
    Proposed,
    Backlog,
    #[value(name = "in-progress", alias = "in_progress")]
    InProgress,
    Review,
    Done,
    Blocked,
    Rejected,
}

impl From<TaskUpdateStatusArg> for TaskStatus {
    fn from(value: TaskUpdateStatusArg) -> Self {
        match value {
            TaskUpdateStatusArg::Proposed => TaskStatus::Proposed,
            TaskUpdateStatusArg::Backlog => TaskStatus::Backlog,
            TaskUpdateStatusArg::InProgress => TaskStatus::InProgress,
            TaskUpdateStatusArg::Review => TaskStatus::Review,
            TaskUpdateStatusArg::Done => TaskStatus::Done,
            TaskUpdateStatusArg::Blocked => TaskStatus::Blocked,
            TaskUpdateStatusArg::Rejected => TaskStatus::Rejected,
        }
    }
}

// --- Start ---

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
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskStartArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let task = runtime.start_task(&self.id, self.note, self.comment)?;
        if self.json {
            crate::output::json::print_pretty(&task_to_json(&task))
        } else {
            println!("Started task '{}'", task.id);
            Ok(())
        }
    }
}

// --- Approve ---

#[derive(Args)]
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
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskApproveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let ids = if self.all_proposed {
            let proposed = runtime.list_tasks_filtered(Some(TaskStatus::Proposed), None)?;
            if proposed.is_empty() {
                println!("No proposed tasks found.");
                return Ok(());
            }
            if !self.yes {
                println!("The following {} task(s) will be approved:", proposed.len());
                for task in &proposed {
                    println!("  {} — {}", task.id, task.title);
                }
                print!("Proceed? [y/N] ");
                use std::io::Write;
                std::io::stdout().flush().map_err(|e| OrbitError::Io(e.to_string()))?;
                let mut input = String::new();
                std::io::stdin()
                    .read_line(&mut input)
                    .map_err(|e| OrbitError::Io(e.to_string()))?;
                if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
                    println!("Aborted.");
                    return Ok(());
                }
            }
            proposed.into_iter().map(|t| t.id).collect::<Vec<_>>()
        } else {
            self.ids
        };

        let bulk = self.all_proposed || ids.len() > 1;
        if self.json {
            let mut results = Vec::new();
            for id in &ids {
                let task = runtime.approve_task(id, self.note.clone(), self.comment.clone())?;
                results.push(task_to_json(&task));
            }
            if bulk {
                crate::output::json::print_pretty(&Value::Array(results))
            } else {
                crate::output::json::print_pretty(results.first().unwrap_or(&Value::Null))
            }
        } else {
            for id in &ids {
                let task = runtime.approve_task(id, self.note.clone(), self.comment.clone())?;
                println!("Approved task '{}'", task.id);
            }
            Ok(())
        }
    }
}

// --- Reject ---

#[derive(Args)]
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
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskRejectArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let ids = if self.all_proposed {
            let proposed = runtime.list_tasks_filtered(Some(TaskStatus::Proposed), None)?;
            if proposed.is_empty() {
                println!("No proposed tasks found.");
                return Ok(());
            }
            if !self.yes {
                println!("The following {} task(s) will be rejected:", proposed.len());
                for task in &proposed {
                    println!("  {} — {}", task.id, task.title);
                }
                print!("Proceed? [y/N] ");
                use std::io::Write;
                std::io::stdout().flush().map_err(|e| OrbitError::Io(e.to_string()))?;
                let mut input = String::new();
                std::io::stdin()
                    .read_line(&mut input)
                    .map_err(|e| OrbitError::Io(e.to_string()))?;
                if !matches!(input.trim().to_lowercase().as_str(), "y" | "yes") {
                    println!("Aborted.");
                    return Ok(());
                }
            }
            proposed.into_iter().map(|t| t.id).collect::<Vec<_>>()
        } else {
            self.ids
        };

        let bulk = self.all_proposed || ids.len() > 1;
        if self.json {
            let mut results = Vec::new();
            for id in &ids {
                let task = runtime.reject_task(id, self.note.clone(), self.comment.clone())?;
                results.push(task_to_json(&task));
            }
            if bulk {
                crate::output::json::print_pretty(&Value::Array(results))
            } else {
                crate::output::json::print_pretty(results.first().unwrap_or(&Value::Null))
            }
        } else {
            for id in &ids {
                let task = runtime.reject_task(id, self.note.clone(), self.comment.clone())?;
                println!("Rejected task '{}'", task.id);
            }
            Ok(())
        }
    }
}

// --- Archive ---

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
            crate::output::json::print_pretty(&task_to_json(&task))
        } else {
            println!("Archived task '{}'", self.id);
            Ok(())
        }
    }
}

// --- Unarchive ---

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
            crate::output::json::print_pretty(&task_to_json(&task))
        } else {
            println!("Unarchived task '{}'", self.id);
            Ok(())
        }
    }
}

// --- Delete ---

#[derive(Args)]
pub struct TaskDeleteArgs {
    /// Task ID
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskDeleteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.delete_task(&self.id)?;
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

// --- Search ---

#[derive(Args)]
pub struct TaskSearchArgs {
    /// Search query
    pub query: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskSearchArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let tasks = runtime.search_tasks(&self.query)?;

        if self.json {
            let json_tasks: Vec<Value> = tasks.iter().map(task_to_json).collect();
            crate::output::json::print_pretty(&Value::Array(json_tasks))
        } else {
            print_task_table(&tasks);
            Ok(())
        }
    }
}

// --- Helpers ---

fn print_task_table(tasks: &[orbit_core::Task]) {
    use comfy_table::Cell;
    let mut table = crate::output::table::build_table(&["ID", "STATUS", "PRI", "TYPE", "TITLE"]);
    for task in tasks {
        table.add_row(vec![
            Cell::new(&task.id),
            crate::output::color::status_color_cell(&task.status.to_string()),
            crate::output::color::priority_color_cell(&task.priority.to_string()),
            Cell::new(task.task_type.to_string()),
            Cell::new(&task.title),
        ]);
    }
    println!("{table}");
}

fn task_to_signal_json(task: &orbit_core::Task) -> Value {
    json!({
        "id": task.id,
        "title": task.title,
        "type": task.task_type.to_string(),
        "status": task.status.to_string(),
        "priority": task.priority.to_string(),
        "complexity": task.complexity.map(|value| value.to_string()),
    })
}

fn task_to_json(task: &orbit_core::Task) -> Value {
    json!({
        "id": task.id,
        "title": task.title,
        "description": task.description,
        "plan": task.plan,
        "instructions": task.plan,
        "execution_summary": task.execution_summary,
        "context_files": task.context_files,
        "workspace_path": task.workspace_path,
        "assigned_to": task.assigned_to,
        "created_by": task.created_by,
        "status": task.status.to_string(),
        "priority": task.priority.to_string(),
        "complexity": task.complexity.map(|value| value.to_string()),
        "type": task.task_type.to_string(),
        "branch": task.branch,
        "pr_number": task.pr_number,
        "proposed_by": task.proposed_by,
        "comments": task.comments,
        "history": task.history,
        "created_at": task.created_at.to_rfc3339(),
        "updated_at": task.updated_at.to_rfc3339(),
    })
}

fn parse_context_csv(raw: &str) -> Vec<String> {
    crate::parse::csv_to_vec(raw)
}
