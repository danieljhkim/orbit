use clap::{Args, Subcommand};
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
use orbit_core::{OrbitError, OrbitRuntime, TaskPriority, TaskStatus, TaskType};
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
    /// Optional assignee display name
    #[arg(long)]
    pub assigned_to: Option<String>,
    /// Optional creator display name
    #[arg(long)]
    pub created_by: Option<String>,
    /// Priority level
    #[arg(long, value_enum, default_value_t = TaskPriority::Medium)]
    pub priority: TaskPriority,
    /// Task type
    #[arg(long = "type", value_enum, default_value_t = TaskType::Task)]
    pub task_type: TaskType,
    /// Who proposed this task
    #[arg(long)]
    pub proposed_by: String,
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
            assigned_to: self.assigned_to,
            created_by: self.created_by,
            priority: self.priority,
            task_type: self.task_type,
            proposed_by: Some(self.proposed_by),
        })?;

        println!("{}", task.id);
        Ok(())
    }
}

// --- List ---

#[derive(Args)]
pub struct TaskListArgs {
    /// Filter by status
    #[arg(long, value_enum)]
    pub status: Option<TaskStatus>,
    /// Filter by priority
    #[arg(long, value_enum)]
    pub priority: Option<TaskPriority>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskListArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let tasks = if self.status.is_some() || self.priority.is_some() {
            runtime.list_tasks_filtered(self.status, self.priority)?
        } else {
            runtime.list_tasks()?
        };

        if self.json {
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
            println!("ID:          {}", task.id);
            println!("Title:       {}", task.title);
            println!("Status:      {}", task.status);
            println!("Priority:    {}", task.priority);
            println!("Type:        {}", task.task_type);
            if !task.description.is_empty() {
                println!("Description: {}", task.description);
            }
            if !task.plan.is_empty() {
                println!("Plan:        {}", task.plan);
            }
            if !task.execution_summary.is_empty() {
                println!("Execution Summary: {}", task.execution_summary);
            }
            if !task.comments.is_empty() {
                println!("Comments:");
                for comment in &task.comments {
                    println!(
                        "  [{}] {}: {}",
                        comment.at.to_rfc3339(),
                        comment.by,
                        comment.message
                    );
                }
            }
            if !task.context_files.is_empty() {
                println!("Context:     {}", task.context_files.join(", "));
            }
            if let Some(ref workspace_path) = task.workspace_path {
                println!("Workspace:   {}", workspace_path);
            }
            if let Some(ref assigned_to) = task.assigned_to {
                println!("Assigned To: {}", assigned_to);
            }
            if let Some(ref created_by) = task.created_by {
                println!("Created By:  {}", created_by);
            }
            if let Some(ref branch) = task.branch {
                println!("Branch:      {}", branch);
            }
            if let Some(ref pr_number) = task.pr_number {
                println!("PR Number:   {}", pr_number);
            }
            if let Some(ref proposed_by) = task.proposed_by {
                println!("Proposed By: {}", proposed_by);
            }
            if let Some(ref approved_by) = task.proposal_approved_by {
                println!("Proposal Approved By: {}", approved_by);
            }
            if let Some(ref rejected_by) = task.proposal_rejected_by {
                println!("Proposal Rejected By: {}", rejected_by);
            }
            if let Some(ref note) = task.proposal_decision_note {
                println!("Proposal Note: {}", note);
            }
            if let Some(ref approved_by) = task.review_approved_by {
                println!("Review Approved By: {}", approved_by);
            }
            if let Some(ref rejected_by) = task.review_rejected_by {
                println!("Review Rejected By: {}", rejected_by);
            }
            if let Some(ref note) = task.review_decision_note {
                println!("Review Note: {}", note);
            }
            println!("Created:     {}", task.created_at.to_rfc3339());
            println!("Updated:     {}", task.updated_at.to_rfc3339());
            Ok(())
        }
    }
}

// --- Update ---

#[derive(Args)]
pub struct TaskUpdateArgs {
    /// Task ID
    pub id: String,
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
    /// New assignee (empty string clears)
    #[arg(long)]
    pub assigned_to: Option<String>,
    /// New status
    #[arg(long, value_enum)]
    pub status: Option<TaskUpdateStatusArg>,
    /// Git branch name (empty string clears)
    #[arg(long)]
    pub branch: Option<String>,
    /// Pull request number (empty string clears)
    #[arg(long)]
    pub pr_number: Option<String>,
}

impl Execute for TaskUpdateArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let assigned_to = self.assigned_to.map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        });
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
                description: self.description,
                plan: self.plan,
                execution_summary: self.execution_summary,
                comment: self.comment,
                assigned_to,
                status: self.status.map(Into::into),
                branch,
                pr_number,
            },
        )?;

        println!("Updated task '{}'", task.id);
        Ok(())
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
        }
    }
}

// --- Approve ---

#[derive(Args)]
pub struct TaskApproveArgs {
    /// Task ID
    pub id: String,
    /// Approver identity
    #[arg(long, default_value = "human")]
    pub by: String,
    /// Optional approval note
    #[arg(long)]
    pub note: Option<String>,
    /// Append a task comment
    #[arg(long)]
    pub comment: Option<String>,
}

impl Execute for TaskApproveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let task = runtime.approve_task(&self.id, &self.by, self.note, self.comment)?;
        println!("Approved task '{}'", task.id);
        Ok(())
    }
}

// --- Reject ---

#[derive(Args)]
pub struct TaskRejectArgs {
    /// Task ID
    pub id: String,
    /// Rejector identity
    #[arg(long, default_value = "human")]
    pub by: String,
    /// Rejection note
    #[arg(long)]
    pub note: String,
    /// Append a task comment
    #[arg(long)]
    pub comment: Option<String>,
}

impl Execute for TaskRejectArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let task = runtime.reject_task(&self.id, &self.by, self.note, self.comment)?;
        println!("Rejected task '{}'", task.id);
        Ok(())
    }
}

// --- Archive ---

#[derive(Args)]
pub struct TaskArchiveArgs {
    /// Task ID
    pub id: String,
}

impl Execute for TaskArchiveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.archive_task(&self.id)?;
        println!("Archived task '{}'", self.id);
        Ok(())
    }
}

// --- Unarchive ---

#[derive(Args)]
pub struct TaskUnarchiveArgs {
    /// Task ID
    pub id: String,
}

impl Execute for TaskUnarchiveArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.unarchive_task(&self.id)?;
        println!("Unarchived task '{}'", self.id);
        Ok(())
    }
}

// --- Delete ---

#[derive(Args)]
pub struct TaskDeleteArgs {
    /// Task ID
    pub id: String,
}

impl Execute for TaskDeleteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        runtime.delete_task(&self.id)?;
        println!("Deleted task '{}'", self.id);
        Ok(())
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
    println!(
        "{:<28} {:<12} {:<8} {:<8} TITLE",
        "ID", "STATUS", "PRI", "TYPE"
    );
    for task in tasks {
        println!(
            "{:<28} {:<12} {:<8} {:<8} {}",
            task.id, task.status, task.priority, task.task_type, task.title
        );
    }
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
        "type": task.task_type.to_string(),
        "branch": task.branch,
        "pr_number": task.pr_number,
        "proposed_by": task.proposed_by,
        "proposal_approved_by": task.proposal_approved_by,
        "proposal_rejected_by": task.proposal_rejected_by,
        "proposal_decision_note": task.proposal_decision_note,
        "review_approved_by": task.review_approved_by,
        "review_rejected_by": task.review_rejected_by,
        "review_decision_note": task.review_decision_note,
        "comments": task.comments,
        "created_at": task.created_at.to_rfc3339(),
        "updated_at": task.updated_at.to_rfc3339(),
    })
}

fn parse_context_csv(raw: &str) -> Vec<String> {
    crate::parse::csv_to_vec(raw)
}
