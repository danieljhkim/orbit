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
    /// Manage review threads on a task
    #[command(name = "review-thread")]
    ReviewThread(ReviewThreadCommand),
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
            TaskSubcommand::ReviewThread(cmd) => cmd.execute(runtime),
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
    /// Acceptance criteria. Repeat the flag for multiple criteria.
    #[arg(long = "acceptance-criteria")]
    pub acceptance_criteria: Vec<String>,
    /// Optional task plan payload. Leave blank for the executing agent or planning activity to author later.
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
    /// Workspace path for the task
    #[arg(long)]
    pub workspace: Option<String>,
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
                acceptance_criteria: self.acceptance_criteria,
                plan,
                comment: self.comment,
                context_files: parse_context_csv(&self.context),
                workspace_path: self.workspace,
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
    /// Filter by batch ID
    #[arg(long)]
    pub batch_id: Option<String>,
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
        let batch_id = self.batch_id;

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
            if !task.acceptance_criteria.is_empty() {
                println!("{}", bold("Acceptance Criteria:"));
                for criterion in &task.acceptance_criteria {
                    println!("  - {}", criterion);
                }
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
            if let Some(agent) = task.actor_identity.agent_name() {
                println!("{} {}", bold("Agent:"), agent);
            }
            if let Some(model) = task.actor_identity.agent_model() {
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
            if let Some(ref pr_status) = task.pr_status {
                println!("{} {}", bold("PR Status:"), pr_status);
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
    /// Acceptance criteria. Repeat the flag for multiple criteria.
    #[arg(long = "acceptance-criteria")]
    pub acceptance_criteria: Vec<String>,
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
    /// PR review status (approve, request-changes)
    #[arg(long)]
    pub pr_status: Option<String>,
    /// Batch ID to associate with the task (empty string clears)
    #[arg(long)]
    pub batch_id: Option<String>,
    /// Comma-separated context file paths (empty string clears)
    #[arg(long = "context", alias = "context-files")]
    pub context_files: Option<String>,
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
        let TaskUpdateArgs {
            id,
            title,
            description,
            acceptance_criteria,
            plan,
            execution_summary,
            comment,
            status,
            pr_number,
            pr_status,
            batch_id,
            context_files,
            agent,
            model,
            json,
        } = self;

        let pr_number = pr_number.map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        });
        let pr_status = pr_status.map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        });
        let batch_id = batch_id.map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        });
        let acceptance_criteria = (!acceptance_criteria.is_empty()).then_some(acceptance_criteria);

        let task = runtime.update_task_with_identity(
            &id,
            TaskUpdateParams {
                title,
                description,
                acceptance_criteria,
                plan,
                execution_summary,
                comment,
                status: status.map(Into::into),
                pr_number,
                pr_status,
                batch_id,
                context_files: context_files.map(|c| parse_context_csv(&c)),
                ..Default::default()
            },
            agent,
            model,
        )?;

        if json {
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
            let proposed =
                runtime.list_tasks_filtered(Some(TaskStatus::Proposed), None, None, None)?;
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
            let proposed =
                runtime.list_tasks_filtered(Some(TaskStatus::Proposed), None, None, None)?;
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

// --- Review Thread ---

#[derive(Args)]
pub struct ReviewThreadCommand {
    #[command(subcommand)]
    pub command: ReviewThreadSubcommand,
}

impl Execute for ReviewThreadCommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        self.command.execute(runtime)
    }
}

#[derive(Subcommand)]
pub enum ReviewThreadSubcommand {
    /// Create a new review thread on a task
    Add(ReviewThreadAddArgs),
    /// List review threads on a task
    List(ReviewThreadListArgs),
    /// Reply to an existing review thread
    Reply(ReviewThreadReplyArgs),
    /// Resolve a review thread
    Resolve(ReviewThreadResolveArgs),
}

impl Execute for ReviewThreadSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            ReviewThreadSubcommand::Add(args) => args.execute(runtime),
            ReviewThreadSubcommand::List(args) => args.execute(runtime),
            ReviewThreadSubcommand::Reply(args) => args.execute(runtime),
            ReviewThreadSubcommand::Resolve(args) => args.execute(runtime),
        }
    }
}

#[derive(Args)]
pub struct ReviewThreadAddArgs {
    /// Task ID
    pub id: String,
    /// Review comment body
    #[arg(long)]
    pub body: String,
    /// File path for inline comment
    #[arg(long)]
    pub path: Option<String>,
    /// Line number for inline comment
    #[arg(long)]
    pub line: Option<u64>,
    /// Explicit agent name
    #[arg(long)]
    pub agent: Option<String>,
    /// Explicit agent model
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for ReviewThreadAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let thread = runtime.add_review_thread(
            &self.id, self.body, self.path, self.line, self.agent, self.model,
        )?;
        if self.json {
            crate::output::json::print_pretty(&serde_json::to_value(&thread).unwrap_or_default())
        } else {
            println!("Created review thread '{}'", thread.thread_id);
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct ReviewThreadListArgs {
    /// Task ID
    pub id: String,
    /// Filter by thread status (open, resolved)
    #[arg(long)]
    pub status: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for ReviewThreadListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let status_filter = self
            .status
            .map(|s| {
                s.parse::<orbit_core::ReviewThreadStatus>()
                    .map_err(OrbitError::InvalidInput)
            })
            .transpose()?;
        let threads = runtime.list_review_threads(&self.id, status_filter)?;
        if self.json {
            crate::output::json::print_pretty(&serde_json::to_value(&threads).unwrap_or_default())
        } else {
            for t in &threads {
                println!(
                    "{}\t{}\t{} message(s)",
                    t.thread_id,
                    t.status,
                    t.messages.len()
                );
            }
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct ReviewThreadReplyArgs {
    /// Task ID
    pub id: String,
    /// Thread ID to reply to
    pub thread_id: String,
    /// Reply body
    #[arg(long)]
    pub body: String,
    /// Explicit agent name
    #[arg(long)]
    pub agent: Option<String>,
    /// Explicit agent model
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for ReviewThreadReplyArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let thread = runtime.reply_review_thread(
            &self.id,
            &self.thread_id,
            self.body,
            self.agent,
            self.model,
        )?;
        if self.json {
            crate::output::json::print_pretty(&serde_json::to_value(&thread).unwrap_or_default())
        } else {
            println!(
                "Replied to thread '{}' ({} messages)",
                thread.thread_id,
                thread.messages.len()
            );
            Ok(())
        }
    }
}

#[derive(Args)]
pub struct ReviewThreadResolveArgs {
    /// Task ID
    pub id: String,
    /// Thread ID to resolve
    pub thread_id: String,
    /// Explicit agent name
    #[arg(long)]
    pub agent: Option<String>,
    /// Explicit agent model
    #[arg(long)]
    pub model: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for ReviewThreadResolveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let thread =
            runtime.resolve_review_thread(&self.id, &self.thread_id, self.agent, self.model)?;
        if self.json {
            crate::output::json::print_pretty(&serde_json::to_value(&thread).unwrap_or_default())
        } else {
            println!("Resolved thread '{}'", thread.thread_id);
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
        "acceptance_criteria": task.acceptance_criteria,
        "plan": task.plan,
        "execution_summary": task.execution_summary,
        "context_files": task.context_files,
        "workspace_path": task.workspace_path,
        "repo_root": task.repo_root,
        "assigned_to": task.assigned_to,
        "created_by": task.created_by,
        "agent": task.actor_identity.agent_name(),
        "model": task.actor_identity.agent_model(),
        "status": task.status.to_string(),
        "priority": task.priority.to_string(),
        "complexity": task.complexity.map(|value| value.to_string()),
        "type": task.task_type.to_string(),
        "pr_number": task.pr_number,
        "pr_status": task.pr_status,
        "proposed_by": task.proposed_by,
        "source_task_id": task.source_task_id,
        "comments": task.comments,
        "history": task.history,
        "review_threads": task.review_threads,
        "created_at": task.created_at.to_rfc3339(),
        "updated_at": task.updated_at.to_rfc3339(),
    })
}

fn parse_context_csv(raw: &str) -> Vec<String> {
    crate::parse::csv_to_vec(raw)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::command::{Cli, Commands};

    use super::{TaskSubcommand, parse_context_csv};

    #[test]
    fn task_update_accepts_context_flag() {
        let cli = Cli::try_parse_from([
            "orbit",
            "task",
            "update",
            "T20260330-002312",
            "--context",
            "orbit-cli/src/command/task.rs,orbit-tools/src/builtin/orbit/task_update.rs",
        ])
        .expect("`--context` should parse");

        let Commands::Task(task_command) = cli.command else {
            panic!("expected task command");
        };
        let TaskSubcommand::Update(args) = task_command.command else {
            panic!("expected task update command");
        };
        assert_eq!(
            args.context_files.as_deref(),
            Some("orbit-cli/src/command/task.rs,orbit-tools/src/builtin/orbit/task_update.rs")
        );
    }

    #[test]
    fn task_update_accepts_context_files_alias() {
        let cli = Cli::try_parse_from([
            "orbit",
            "task",
            "update",
            "T20260330-002312",
            "--context-files",
            "orbit-cli/src/command/task.rs",
        ])
        .expect("`--context-files` should remain supported");

        let Commands::Task(task_command) = cli.command else {
            panic!("expected task command");
        };
        let TaskSubcommand::Update(args) = task_command.command else {
            panic!("expected task update command");
        };
        assert_eq!(
            args.context_files.as_deref(),
            Some("orbit-cli/src/command/task.rs")
        );
    }

    #[test]
    fn parse_context_csv_trims_update_context_values() {
        assert_eq!(
            parse_context_csv(
                " orbit-cli/src/command/task.rs, orbit-tools/src/builtin/orbit/task_update.rs "
            ),
            vec![
                "orbit-cli/src/command/task.rs".to_string(),
                "orbit-tools/src/builtin/orbit/task_update.rs".to_string(),
            ]
        );
    }
}
