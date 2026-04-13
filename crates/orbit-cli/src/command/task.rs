use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
use orbit_core::{OrbitError, OrbitRuntime, TaskComplexity, TaskPriority, TaskStatus, TaskType};
use orbit_types::TaskArtifact;
use serde_json::{Value, json};

use crate::command::Execute;

#[derive(Args)]
#[command(about = "Create, update, and manage tasks")]
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
    /// Lint a task for stale paths and vague acceptance criteria
    Lint(TaskLintArgs),
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
    /// Backfill: drop non-existent `context_files` entries from active tasks.
    /// Defaults to a dry-run report; pass `--write` to apply.
    #[command(name = "prune-context")]
    PruneContext(TaskPruneContextArgs),
}

impl Execute for TaskSubcommand {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        match self {
            TaskSubcommand::Add(args) => args.execute(runtime),
            TaskSubcommand::List(args) => args.execute(runtime),
            TaskSubcommand::Show(args) => args.execute(runtime),
            TaskSubcommand::Lint(args) => args.execute(runtime),
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
            TaskSubcommand::PruneContext(args) => args.execute(runtime),
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
                system_created: false,
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

// --- Show ---

#[derive(Args)]
pub struct TaskShowArgs {
    /// Task ID
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
    /// Print only the specified field projection(s). Valid values: comments, plan,
    /// execution_summary, description, acceptance_criteria, history, context_files, artifacts.
    /// Repeat the flag or use a comma-separated value list. Combined with --json,
    /// a single field returns that field as JSON and multiple fields return a JSON object.
    #[arg(long = "fields", alias = "field", value_delimiter = ',', num_args = 1..)]
    pub fields: Vec<String>,
}

impl Execute for TaskShowArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let task = runtime.get_task(&self.id)?;
        let fields = normalize_task_show_fields(&self.fields)?;

        if !fields.is_empty() {
            if self.json {
                return crate::output::json::print_pretty(&task_fields_to_json(
                    runtime, &task, &fields,
                )?);
            }
            return print_task_fields(runtime, &task, &fields);
        }

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

// --- Lint ---

#[derive(Args)]
pub struct TaskLintArgs {
    /// Task ID
    pub id: String,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for TaskLintArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let report = runtime.lint_task(&self.id)?;

        if self.json {
            let value = serde_json::to_value(&report).map_err(|e| OrbitError::Io(e.to_string()))?;
            return crate::output::json::print_pretty(&value);
        }

        if report.findings.is_empty() {
            println!(
                "No lint findings for '{}' ({} ms).",
                report.task_id, report.duration_ms
            );
            return Ok(());
        }

        println!(
            "{} finding(s) for '{}' ({} ms):",
            report.finding_count, report.task_id, report.duration_ms
        );
        for finding in report.findings {
            let severity = match finding.severity {
                orbit_core::command::task::TaskLintSeverity::Error => "error",
                orbit_core::command::task::TaskLintSeverity::Warning => "warning",
            };
            println!("[{severity}] {}: {}", finding.check, finding.message);
            println!("  fix: {}", finding.fix_it);
        }
        Ok(())
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
    /// Task artifact write in `path=content` form. Repeat for multiple artifacts.
    #[arg(long = "artifact")]
    pub artifacts: Vec<String>,
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
            artifacts,
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
        let upsert_artifacts = parse_artifact_args(&artifacts)?;

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
                upsert_artifacts,
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
    /// Force deletion without status guard (required for non-proposed/rejected tasks)
    #[arg(long)]
    pub force: bool,
}

impl Execute for TaskDeleteArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let task = runtime.get_task(&self.id)?;
        if !self.force && !matches!(task.status, TaskStatus::Proposed | TaskStatus::Rejected) {
            return Err(OrbitError::InvalidInput(format!(
                "task '{}' is in status '{}'; use --force to delete tasks not in proposed or rejected status",
                self.id, task.status
            )));
        }
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
            print_task_table(&tasks, false);
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

fn print_task_table(tasks: &[orbit_core::Task], full: bool) {
    use comfy_table::Cell;
    let headers = if full {
        vec![
            "ID",
            "TITLE",
            "STATUS",
            "PRIORITY",
            "TYPE",
            "ASSIGNED_TO",
            "CREATED_AT",
            "UPDATED_AT",
        ]
    } else {
        vec!["ID", "TITLE", "STATUS", "PRIORITY", "TYPE"]
    };
    let mut table = crate::output::table::build_table(&headers);
    for task in tasks {
        let mut row = vec![
            Cell::new(&task.id),
            Cell::new(&task.title),
            crate::output::color::status_color_cell(&task.status.to_string()),
            crate::output::color::priority_color_cell(&task.priority.to_string()),
            Cell::new(task.task_type.to_string()),
        ];
        if full {
            row.extend([
                Cell::new(task.assigned_to.as_deref().unwrap_or("-")),
                Cell::new(format_task_table_timestamp(task.created_at)),
                Cell::new(format_task_table_timestamp(task.updated_at)),
            ]);
        }
        crate::output::table::add_single_line_row(&mut table, row);
    }
    println!("{table}");
}

fn format_task_table_timestamp(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%d %H:%M").to_string()
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

fn normalize_task_show_fields(fields: &[String]) -> Result<Vec<String>, OrbitError> {
    let mut normalized = Vec::new();
    for field in fields {
        let trimmed = field.trim();
        if trimmed.is_empty() {
            return Err(OrbitError::InvalidInput(
                "task show field selectors must not be empty".to_string(),
            ));
        }
        if !matches!(
            trimmed,
            "comments"
                | "plan"
                | "execution_summary"
                | "description"
                | "acceptance_criteria"
                | "history"
                | "context_files"
                | "artifacts"
        ) {
            return Err(OrbitError::InvalidInput(format!(
                "unknown field selector `{trimmed}`. Valid values: comments, plan, execution_summary, description, acceptance_criteria, history, context_files, artifacts"
            )));
        }
        normalized.push(trimmed.to_string());
    }
    Ok(normalized)
}

fn task_field_to_json(
    runtime: &OrbitRuntime,
    task: &orbit_core::Task,
    field: &str,
) -> Result<Value, OrbitError> {
    match field {
        "comments" => {
            serde_json::to_value(&task.comments).map_err(|e| OrbitError::Io(e.to_string()))
        }
        "plan" => Ok(Value::String(task.plan.clone())),
        "execution_summary" => Ok(Value::String(task.execution_summary.clone())),
        "description" => Ok(Value::String(task.description.clone())),
        "acceptance_criteria" => serde_json::to_value(&task.acceptance_criteria)
            .map_err(|e| OrbitError::Io(e.to_string())),
        "history" => serde_json::to_value(&task.history).map_err(|e| OrbitError::Io(e.to_string())),
        "context_files" => {
            serde_json::to_value(&task.context_files).map_err(|e| OrbitError::Io(e.to_string()))
        }
        "artifacts" => serde_json::to_value(runtime.get_task_artifacts(&task.id)?)
            .map_err(|e| OrbitError::Io(e.to_string())),
        other => Err(OrbitError::InvalidInput(format!(
            "unknown field selector `{other}`. Valid values: comments, plan, execution_summary, description, acceptance_criteria, history, context_files, artifacts"
        ))),
    }
}

fn task_fields_to_json(
    runtime: &OrbitRuntime,
    task: &orbit_core::Task,
    fields: &[String],
) -> Result<Value, OrbitError> {
    if fields.len() == 1 {
        return task_field_to_json(runtime, task, &fields[0]);
    }

    let mut object = serde_json::Map::new();
    for field in fields {
        object.insert(field.clone(), task_field_to_json(runtime, task, field)?);
    }
    Ok(Value::Object(object))
}

fn print_task_fields(
    runtime: &OrbitRuntime,
    task: &orbit_core::Task,
    fields: &[String],
) -> Result<(), OrbitError> {
    if fields.len() == 1 {
        return print_single_task_field(runtime, task, &fields[0]);
    }

    use crate::output::color::bold;
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("{} {}", bold("Field:"), field);
        print_single_task_field(runtime, task, field)?;
    }
    Ok(())
}

fn print_single_task_field(
    runtime: &OrbitRuntime,
    task: &orbit_core::Task,
    field: &str,
) -> Result<(), OrbitError> {
    match field {
        "comments" => {
            use crate::output::color::dimmed;
            for comment in &task.comments {
                println!(
                    "{} {}: {}",
                    dimmed(&format!("[{}]", comment.at.to_rfc3339())),
                    comment.by,
                    comment.message
                );
            }
            Ok(())
        }
        "plan" => {
            print!("{}", task.plan);
            Ok(())
        }
        "execution_summary" => {
            print!("{}", task.execution_summary);
            Ok(())
        }
        "description" => {
            print!("{}", task.description);
            Ok(())
        }
        "acceptance_criteria" => {
            for criterion in &task.acceptance_criteria {
                println!("- {}", criterion);
            }
            Ok(())
        }
        "history" => {
            use crate::output::color::dimmed;
            for entry in &task.history {
                if let Some(note) = &entry.note {
                    println!(
                        "{} {}: {} ({})",
                        dimmed(&format!("[{}]", entry.at.to_rfc3339())),
                        entry.by,
                        entry.event,
                        note
                    );
                } else {
                    println!(
                        "{} {}: {}",
                        dimmed(&format!("[{}]", entry.at.to_rfc3339())),
                        entry.by,
                        entry.event
                    );
                }
            }
            Ok(())
        }
        "context_files" => {
            for path in &task.context_files {
                println!("{}", path);
            }
            Ok(())
        }
        "artifacts" => {
            use crate::output::color::bold;
            let artifacts = runtime.get_task_artifacts(&task.id)?;
            for (index, artifact) in artifacts.iter().enumerate() {
                if index > 0 {
                    println!();
                }
                println!("{} {}", bold("Artifact:"), artifact.path);
                print!("{}", artifact.content);
            }
            Ok(())
        }
        other => Err(OrbitError::InvalidInput(format!(
            "unknown field selector `{other}`. Valid values: comments, plan, execution_summary, description, acceptance_criteria, history, context_files, artifacts"
        ))),
    }
}

fn parse_artifact_args(raw_values: &[String]) -> Result<Vec<TaskArtifact>, OrbitError> {
    raw_values
        .iter()
        .map(|raw| {
            let Some((path, content)) = raw.split_once('=') else {
                return Err(OrbitError::InvalidInput(format!(
                    "task artifact must use `path=content` form, got `{raw}`"
                )));
            };
            let path = path.trim();
            if path.is_empty() {
                return Err(OrbitError::InvalidInput(
                    "task artifact path must not be empty".to_string(),
                ));
            }
            Ok(TaskArtifact {
                path: path.to_string(),
                content: content.to_string(),
            })
        })
        .collect()
}

fn parse_context_csv(raw: &str) -> Vec<String> {
    crate::parse::csv_to_vec(raw)
}

// --- PruneContext (backfill) ---

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
                // Forward the *current* (stale) list to update_task. The write-path
                // pruner will drop the missing entries and append a
                // `context_files_pruned` history event automatically.
                runtime.update_task(
                    &task.id,
                    TaskUpdateParams {
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

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::command::{Cli, Commands};

    use super::{
        TaskSubcommand, default_task_list_status_filter, parse_artifact_args, parse_context_csv,
        task_fields_to_json, task_to_json,
    };
    use orbit_core::command::task::{TaskAddParams, TaskUpdateParams};
    use orbit_core::{OrbitRuntime, TaskPriority, TaskStatus, TaskType};
    use orbit_types::TaskArtifact;

    #[test]
    fn task_update_accepts_context_flag() {
        let cli = Cli::try_parse_from([
            "orbit",
            "task",
            "update",
            "T20260330-002312",
            "--context",
            "crates/orbit-cli/src/command/task.rs,crates/orbit-tools/src/builtin/orbit/task_update.rs",
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
            Some(
                "crates/orbit-cli/src/command/task.rs,crates/orbit-tools/src/builtin/orbit/task_update.rs"
            )
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
            "crates/orbit-cli/src/command/task.rs",
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
            Some("crates/orbit-cli/src/command/task.rs")
        );
    }

    #[test]
    fn task_update_accepts_repeatable_artifact_flag() {
        let cli = Cli::try_parse_from([
            "orbit",
            "task",
            "update",
            "T20260330-002312",
            "--artifact",
            "planning-duel/codex-gpt-5.4.md=*authored by: codex / gpt-5.4*",
            "--artifact",
            "planning-duel/claude-opus.md=*authored by: claude / opus*",
        ])
        .expect("`--artifact` should parse");

        let Commands::Task(task_command) = cli.command else {
            panic!("expected task command");
        };
        let TaskSubcommand::Update(args) = task_command.command else {
            panic!("expected task update command");
        };
        assert_eq!(args.artifacts.len(), 2);
    }

    #[test]
    fn parse_artifact_args_splits_on_first_equals() {
        let artifacts = parse_artifact_args(&[
            "planning-duel/codex-gpt-5.4.md=*authored by: codex / gpt-5.4*\n## Plan\n- A = still part of content"
                .to_string(),
        ])
        .expect("artifact should parse");

        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].path, "planning-duel/codex-gpt-5.4.md");
        assert!(artifacts[0].content.contains("A = still part of content"));
    }

    #[test]
    fn task_show_accepts_fields_flag() {
        let cli = Cli::try_parse_from([
            "orbit",
            "task",
            "show",
            "T20260408-0503",
            "--fields",
            "artifacts,history",
            "--json",
        ])
        .expect("task show parses");

        let Commands::Task(task_command) = cli.command else {
            panic!("expected task command");
        };
        let TaskSubcommand::Show(args) = task_command.command else {
            panic!("expected task show command");
        };
        assert_eq!(
            args.fields,
            vec!["artifacts".to_string(), "history".to_string()]
        );
        assert!(args.json);
    }

    #[test]
    fn task_show_accepts_field_alias() {
        let cli = Cli::try_parse_from([
            "orbit",
            "task",
            "show",
            "T20260408-0503",
            "--field",
            "artifacts",
        ])
        .expect("task show parses");

        let Commands::Task(task_command) = cli.command else {
            panic!("expected task command");
        };
        let TaskSubcommand::Show(args) = task_command.command else {
            panic!("expected task show command");
        };
        assert_eq!(args.fields, vec!["artifacts".to_string()]);
    }

    #[test]
    fn task_show_full_json_omits_artifacts_by_default() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "artifact projection".to_string(),
                description: String::new(),
                acceptance_criteria: Vec::new(),
                plan: String::new(),
                comment: None,
                context_files: Vec::new(),
                workspace_path: None,
                priority: TaskPriority::Medium,
                complexity: None,
                task_type: TaskType::Task,
                source_task_id: None,
                ..Default::default()
            })
            .expect("task");
        let task = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    upsert_artifacts: vec![TaskArtifact {
                        path: "planning-duel/codex-gpt-5.4.md".to_string(),
                        content: "*authored by: codex / gpt-5.4*".to_string(),
                    }],
                    ..Default::default()
                },
            )
            .expect("artifact update");

        let json = task_to_json(&task);
        assert!(json.get("artifacts").is_none());
    }

    #[test]
    fn task_show_artifacts_projection_is_explicit() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "artifact projection".to_string(),
                description: String::new(),
                acceptance_criteria: Vec::new(),
                plan: String::new(),
                comment: None,
                context_files: Vec::new(),
                workspace_path: None,
                priority: TaskPriority::Medium,
                complexity: None,
                task_type: TaskType::Task,
                source_task_id: None,
                ..Default::default()
            })
            .expect("task");
        let task = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    upsert_artifacts: vec![TaskArtifact {
                        path: "planning-duel/codex-gpt-5.4.md".to_string(),
                        content: "*authored by: codex / gpt-5.4*".to_string(),
                    }],
                    ..Default::default()
                },
            )
            .expect("artifact update");

        let json = task_fields_to_json(&runtime, &task, &["artifacts".to_string()])
            .expect("artifacts projection");
        let artifacts = json.as_array().expect("artifacts array");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(
            artifacts[0].get("path").and_then(serde_json::Value::as_str),
            Some("planning-duel/codex-gpt-5.4.md")
        );
    }

    #[test]
    fn parse_context_csv_trims_update_context_values() {
        assert_eq!(
            parse_context_csv(
                " crates/orbit-cli/src/command/task.rs, crates/orbit-tools/src/builtin/orbit/task_update.rs "
            ),
            vec![
                "crates/orbit-cli/src/command/task.rs".to_string(),
                "crates/orbit-tools/src/builtin/orbit/task_update.rs".to_string(),
            ]
        );
    }

    #[test]
    fn task_lint_accepts_id_and_json_flag() {
        let cli = Cli::try_parse_from(["orbit", "task", "lint", "T20260408-0503", "--json"])
            .expect("task lint parses");

        let Commands::Task(task_command) = cli.command else {
            panic!("expected task command");
        };
        let TaskSubcommand::Lint(args) = task_command.command else {
            panic!("expected task lint command");
        };
        assert_eq!(args.id, "T20260408-0503");
        assert!(args.json);
    }

    #[test]
    fn task_list_batch_queries_bypass_default_active_status_filter() {
        let active_statuses = [TaskStatus::Backlog, TaskStatus::InProgress];

        let status_filter = default_task_list_status_filter(
            false,
            &[],
            Some("jrun-20260402-0145-2"),
            &active_statuses,
        );

        assert!(status_filter.is_empty());
    }

    #[test]
    fn task_list_keeps_explicit_status_filter_for_non_batch_queries() {
        let active_statuses = [TaskStatus::Backlog, TaskStatus::InProgress];
        let requested_statuses = [TaskStatus::Review];

        let status_filter =
            default_task_list_status_filter(false, &requested_statuses, None, &active_statuses);

        assert_eq!(status_filter, requested_statuses);
    }

    #[test]
    fn task_list_keeps_explicit_status_filter_for_batch_queries() {
        let active_statuses = [TaskStatus::Backlog, TaskStatus::InProgress];
        let requested_statuses = [TaskStatus::Review];

        let status_filter = default_task_list_status_filter(
            false,
            &requested_statuses,
            Some("jrun-20260402-0145-2"),
            &active_statuses,
        );

        assert_eq!(status_filter, requested_statuses);
    }

    #[test]
    fn task_list_accepts_full_flag() {
        let cli =
            Cli::try_parse_from(["orbit", "task", "list", "--full"]).expect("task list parses");

        let Commands::Task(task_command) = cli.command else {
            panic!("expected task command");
        };
        let TaskSubcommand::List(args) = task_command.command else {
            panic!("expected task list command");
        };
        assert!(args.full);
    }
}
