use clap::Args;
use orbit_core::command::task::TaskAddParams;
use orbit_core::{OrbitError, OrbitRuntime, TaskComplexity, TaskPriority, TaskType};

use crate::command::Execute;

use super::output::task_to_json;

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
        if let Some(parent_id) = self.parent_id.as_deref() {
            if runtime.get_task(parent_id).is_err() {
                eprintln!(
                    "warning: parent task '{parent_id}' was not found; creating subtask anyway"
                );
            }
        }

        let (description, plan, priority, task_type) = if let Some(ref tpl_name) = self.template {
            let tpl = runtime.get_task_template(tpl_name)?;
            let description = if self.description.is_empty() {
                tpl.description_template
            } else {
                self.description
            };
            let plan = if self.plan.is_empty() {
                format!(
                    "{}\n\n---\n\n{}",
                    tpl.plan_template.trim_end(),
                    tpl.instructions_template.trim_end()
                )
            } else {
                self.plan
            };
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
                context_files: crate::parse::csv_to_vec(&self.context),
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
