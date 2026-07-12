use clap::{ArgAction, Args};
use orbit_core::{
    AutoTaskAddParams, AutoTaskTemplate, DedupePolicy, OrbitError, OrbitRuntime, TaskPriority,
    TaskStatus, TaskType,
};

use crate::command::Execute;

use super::output::definition_to_json;
use super::schedule_args::require_schedule;

#[derive(Args)]
pub struct AutoTaskAddArgs {
    /// Unique definition name (lowercase alphanumeric, `-`/`_`, starts alphanumeric)
    #[arg(long)]
    pub name: String,
    /// Human description of the recurring chore
    #[arg(long, default_value = "")]
    pub description: String,
    /// 5-field cron expression (mutually exclusive with `--every-minutes`)
    #[arg(long)]
    pub cron: Option<String>,
    /// Interval in minutes (mutually exclusive with `--cron`)
    #[arg(long = "every-minutes")]
    pub every_minutes: Option<u64>,
    /// Title of each minted task
    #[arg(long)]
    pub title: String,
    /// Body / instruction of each minted task
    #[arg(long, default_value = "")]
    pub body: String,
    /// Acceptance criterion. Repeat for multiple.
    #[arg(long = "criterion", action = ArgAction::Append)]
    pub criteria: Vec<String>,
    /// Task type (defaults to chore)
    #[arg(long = "type", value_enum, default_value_t = TaskType::Chore)]
    pub task_type: TaskType,
    /// Tag applied to each minted task (in addition to the provenance tag). Repeat.
    #[arg(long = "tag", action = ArgAction::Append)]
    pub tags: Vec<String>,
    /// Priority (defaults to medium)
    #[arg(long, value_enum, default_value_t = TaskPriority::Medium)]
    pub priority: TaskPriority,
    /// Crew override for minted tasks
    #[arg(long)]
    pub crew: Option<String>,
    /// Status each minted task enters (defaults to backlog)
    #[arg(long, value_enum, default_value_t = TaskStatus::Backlog)]
    pub status: TaskStatus,
    /// Dedupe policy (defaults to skip-if-open)
    #[arg(long, value_enum, default_value_t = DedupePolicy::SkipIfOpen)]
    pub dedupe: DedupePolicy,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

impl Execute for AutoTaskAddArgs {
    fn execute(self, runtime: &OrbitRuntime) -> Result<(), OrbitError> {
        let schedule = require_schedule(self.cron, self.every_minutes)?;
        let template = AutoTaskTemplate {
            title: self.title,
            description: self.body,
            acceptance_criteria: self.criteria,
            task_type: self.task_type,
            tags: self.tags,
            priority: self.priority,
            crew: self.crew,
            status: self.status,
        };
        let definition = runtime.auto_task_add(AutoTaskAddParams {
            name: self.name,
            description: self.description,
            schedule,
            template,
            dedupe: self.dedupe,
        })?;

        if self.json {
            crate::output::json::print_pretty(&definition_to_json(&definition))
        } else {
            println!("{}", definition.name);
            Ok(())
        }
    }
}
