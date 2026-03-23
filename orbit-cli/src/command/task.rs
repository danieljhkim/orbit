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
    /// Manage task templates
    Templates(TaskTemplatesCommand),
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
            TaskSubcommand::Templates(cmd) => cmd.execute(runtime),
        }
    }
}

// --- Add ---

#[derive(Args)]
pub struct TaskAddArgs {
    /// Parent task ID for hierarchical decomposition
    #[arg(long = "parent")]
    pub parent_id: Option<String>,
    /// Task title
    #[arg(long)]
    pub title: String,
    /// Task description (overrides template if --template is also given)
    #[arg(long, default_value = "")]
    pub description: String,
    /// Task plan payload (agent planning input; overrides template if --template is also given)
    #[arg(long, alias = "instructions", default_value = "")]
    pub plan: String,
    /// Pre-populate description, plan, and instructions from a named template
    #[arg(long)]
    pub template: Option<String>,
    /// Append an initial task comment
    #[arg(long)]
    pub comment: Option<String>,
    /// Comma-separated context file paths
    #[arg(long, default_value = "")]
    pub context: String,
    /// Priority level
    #[arg(long, value_enum, default_value_t = TaskPriority::Medium)]
    pub priority: TaskPriority,
    /// Task complexity
    #[arg(long, value_enum)]
    pub complexity: Option<TaskComplexity>,
    /// Task type
    #[arg(long = "type", value_enum, default_value_t = TaskType::Task)]
    pub task_type: TaskType,
    /// For bug tasks: the originating task whose implementation introduced the defect
    #[arg(long = "source-task")]
    pub source_task: Option<String>,
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

impl Execute for TaskAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        if let Some(parent_id) = self.parent_id.as_deref()
            && runtime.get_task(parent_id).is_err()
        {
            eprintln!("warning: parent task '{parent_id}' was not found; creating subtask anyway");
        }

        // If a template is requested, resolve it and use its fields as defaults.
        let (description, plan, priority, task_type) = if let Some(ref tpl_name) = self.template {
            let tpl = runtime.get_task_template(tpl_name)?;
            let description = if self.description.is_empty() {
                tpl.description_template
            } else {
                self.description
            };
            let plan = if self.plan.is_empty() {
                // Combine plan_template and instructions_template.
                let combined = format!(
                    "{}\n\n---\n\n{}",
                    tpl.plan_template.trim_end(),
                    tpl.instructions_template.trim_end()
                );
                combined
            } else {
                self.plan
            };
            // Template priority/type only apply when the caller didn't override them
            // (detect override by comparing against defaults).
            let priority = if self.priority == TaskPriority::Medium {
                tpl.priority
            } else {
                self.priority
            };
            let task_type = if self.task_type == TaskType::Task {
                tpl.task_type
            } else {
                self.task_type
            };
            (description, plan, priority, task_type)
        } else {
            (self.description, self.plan, self.priority, self.task_type)
        };

        let task = runtime.add_task_with_identity(
            TaskAddParams {
                parent_id: self.parent_id,
                title: self.title,
                description,
                plan,
                comment: self.comment,
                context_files: parse_context_csv(&self.context),
                workspace_path: None,
                priority,
                complexity: self.complexity,
                task_type,
                source_task_id: self.source_task.clone(),
            },
            self.agent,
            self.model,
        )?;

        if self.json {
            crate::output::json::print_pretty(&task_to_json(&task))
        } else {
            println!("{}", task.id);
            Ok(())
        }
    }
}

// --- Templates ---

#[derive(Args)]
pub struct TaskTemplatesCommand {
    #[command(subcommand)]
    pub command: TaskTemplatesSubcommand,
}

impl Execute for TaskTemplatesCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum TaskTemplatesSubcommand {
    /// List available task templates (built-in and user-defined)
    List(TaskTemplatesListArgs),
}

impl Execute for TaskTemplatesSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            TaskTemplatesSubcommand::List(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct TaskTemplatesListArgs {
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskTemplatesListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let templates = runtime.list_task_templates()?;

        if self.json {
            let json_templates: Vec<Value> = templates
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "task_type": t.task_type.to_string(),
                        "priority": t.priority.to_string(),
                        "description_template": t.description_template,
                        "plan_template": t.plan_template,
                        "instructions_template": t.instructions_template,
                        "builtin": t.builtin,
                    })
                })
                .collect();
            crate::output::json::print_pretty(&Value::Array(json_templates))
        } else {
            use comfy_table::Cell;
            let mut table = crate::output::table::build_table(&[
                "NAME",
                "TYPE",
                "PRIORITY",
                "SOURCE",
                "DESCRIPTION",
            ]);
            for t in &templates {
                let source = if t.builtin { "built-in" } else { "user" };
                table.add_row(vec![
                    Cell::new(&t.name),
                    Cell::new(t.task_type.to_string()),
                    Cell::new(t.priority.to_string()),
                    Cell::new(source),
                    Cell::new(&t.description),
                ]);
            }
            println!("{table}");
            Ok(())
        }
    }
}

// --- List ---

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
        let parent_id = self.parent_id;

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
            .filter(|t| {
                parent_id
                    .as_deref()
                    .is_none_or(|p| t.parent_id.as_deref() == Some(p))
            })
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
            if let Some(ref parent_id) = task.parent_id {
                println!("{} {}", bold("Parent Task:"), parent_id);
            }
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
            if let Some(ref assigned_to) = task.assigned_to {
                println!("{} {}", bold("Assigned To:"), assigned_to);
            }
            if let Some(ref created_by) = task.created_by {
                println!("{} {}", bold("Created By:"), created_by);
            }
            if let Some(ref agent) = task.agent {
                println!("{} {}", bold("Agent:"), agent);
            }
            if let Some(ref model) = task.model {
                println!("{} {}", bold("Model:"), model);
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
            if let Some(ref pr_number) = task.pr_number {
                println!("{} {}", bold("PR Number:"), pr_number);
            }
            if let Some(ref proposed_by) = task.proposed_by {
                println!("{} {}", bold("Proposed By:"), proposed_by);
            }
            if let Some(ref source_task_id) = task.source_task_id {
                println!("{} {}", bold("Source Task:"), source_task_id);
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
    /// Pull request number (empty string clears)
    #[arg(long)]
    pub pr_number: Option<String>,
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

impl Execute for TaskUpdateArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let pr_number = self.pr_number.map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        });

        let task = runtime.update_task_with_identity(
            &self.id,
            TaskUpdateParams {
                title: self.title,
                description: self.description,
                plan: self.plan,
                execution_summary: self.execution_summary,
                comment: self.comment,
                status: self.status.map(Into::into),
                pr_number,
            },
            self.agent,
            self.model,
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
    Someday,
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
            TaskUpdateStatusArg::Someday => TaskStatus::Someday,
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
            let proposed = runtime.list_tasks_filtered(Some(TaskStatus::Proposed), None, None)?;
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
                std::io::stdout()
                    .flush()
                    .map_err(|e| OrbitError::Io(e.to_string()))?;
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
                let task = runtime.approve_task_with_identity(
                    id,
                    self.note.clone(),
                    self.comment.clone(),
                    self.agent.clone(),
                    self.model.clone(),
                )?;
                results.push(task_to_json(&task));
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
            let proposed = runtime.list_tasks_filtered(Some(TaskStatus::Proposed), None, None)?;
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
                std::io::stdout()
                    .flush()
                    .map_err(|e| OrbitError::Io(e.to_string()))?;
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
                let task = runtime.reject_task_with_identity(
                    id,
                    self.note.clone(),
                    self.comment.clone(),
                    self.agent.clone(),
                    self.model.clone(),
                )?;
                results.push(task_to_json(&task));
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
        "parent_id": task.parent_id,
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
        "parent_id": task.parent_id,
        "title": task.title,
        "description": task.description,
        "plan": task.plan,
        "execution_summary": task.execution_summary,
        "context_files": task.context_files,
        "assigned_to": task.assigned_to,
        "created_by": task.created_by,
        "agent": task.agent,
        "model": task.model,
        "status": task.status.to_string(),
        "priority": task.priority.to_string(),
        "complexity": task.complexity.map(|value| value.to_string()),
        "type": task.task_type.to_string(),
        "pr_number": task.pr_number,
        "proposed_by": task.proposed_by,
        "source_task_id": task.source_task_id,
        "comments": task.comments,
        "history": task.history,
        "created_at": task.created_at.to_rfc3339(),
        "updated_at": task.updated_at.to_rfc3339(),
    })
}

fn parse_context_csv(raw: &str) -> Vec<String> {
    crate::parse::csv_to_vec(raw)
}
