use clap::{ArgAction, Args};
use orbit_common::types::TaskArtifact;
use orbit_core::command::task::TaskUpdateParams;
use orbit_core::{OrbitError, OrbitRuntime, TaskStatus, TaskType};

use crate::command::Execute;

use super::output::task_to_json_for_runtime;

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
    /// Comma-separated dependency task IDs (empty string clears)
    #[arg(long, alias = "dependency")]
    pub dependencies: Option<String>,
    /// Replacement task tags. Repeat or comma-separate for multiple tags.
    #[arg(long = "tag", action = ArgAction::Append, value_delimiter = ',')]
    pub tags: Vec<String>,
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
    /// New task type
    #[arg(long = "type", value_enum)]
    pub task_type: Option<TaskType>,
    /// Explicit planning attribution label (empty string clears)
    #[arg(long)]
    pub planned_by: Option<String>,
    /// Explicit implementation attribution label (empty string clears)
    #[arg(long)]
    pub implemented_by: Option<String>,
    /// PR review status (approve, request-changes)
    #[arg(long)]
    pub pr_status: Option<String>,
    /// Job run ID to associate with the task (empty string clears)
    #[arg(long)]
    pub job_run_id: Option<String>,
    /// Named crew to use when running this task (empty string clears)
    #[arg(long)]
    pub crew: Option<String>,
    /// Comma-separated task context selectors (empty string clears). Prefer
    /// `file:`, `dir:`, or `symbol:` forms; legacy raw paths are accepted and upgraded.
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
            dependencies,
            tags,
            plan,
            execution_summary,
            comment,
            status,
            task_type,
            planned_by,
            implemented_by,
            pr_status,
            job_run_id,
            crew,
            context_files,
            artifacts,
            agent,
            model,
            json,
        } = self;

        let pr_status = pr_status.map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        });
        let job_run_id = job_run_id.map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        });
        let crew = crew.map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        });
        let planned_by = planned_by.map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        });
        let implemented_by = implemented_by.map(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        });
        let acceptance_criteria = (!acceptance_criteria.is_empty()).then_some(acceptance_criteria);
        let dependencies = dependencies.map(|value| crate::parse::csv_to_vec(&value));
        let tags = (!tags.is_empty()).then_some(tags);
        let upsert_artifacts = parse_artifact_args(&artifacts)?;

        let task = runtime.update_task_with_identity(
            &id,
            TaskUpdateParams {
                title,
                description,
                acceptance_criteria,
                dependencies,
                tags,
                plan,
                execution_summary,
                comment,
                status: status.map(Into::into),
                task_type,
                planned_by,
                implemented_by,
                pr_status,
                job_run_id,
                crew,
                context_files: context_files.map(|c| crate::parse::csv_to_vec(&c)),
                upsert_artifacts,
                ..Default::default()
            },
            agent,
            model,
        )?;

        if json {
            crate::output::json::print_pretty(&task_to_json_for_runtime(runtime, &task)?)
        } else {
            println!("Updated task '{}'", task.id);
            Ok(())
        }
    }
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum TaskUpdateStatusArg {
    Proposed,
    Friction,
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
            TaskUpdateStatusArg::Friction => TaskStatus::Friction,
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
            Ok(TaskArtifact::from_text(path, content))
        })
        .collect()
}

#[cfg(test)]
mod tests;
