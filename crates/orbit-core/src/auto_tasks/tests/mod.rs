use orbit_common::types::{
    AutoTaskSchedule, AutoTaskTemplate, DedupePolicy, TaskPriority, TaskType,
};

use crate::auto_tasks::crud::AutoTaskAddParams;

mod crud;
mod generate;
mod schedule;
mod scheduler;
mod shipped;

/// A minimal, valid template for tests.
pub(super) fn template(title: &str) -> AutoTaskTemplate {
    AutoTaskTemplate {
        title: title.to_string(),
        description: "Recurring chore body.".to_string(),
        acceptance_criteria: vec!["Chore is observable.".to_string()],
        task_type: TaskType::Chore,
        tags: vec![],
        priority: TaskPriority::Medium,
        crew: None,
        status: orbit_common::types::TaskStatus::Backlog,
    }
}

/// Add-params for a definition on an N-minute interval schedule.
pub(super) fn interval_params(name: &str, every_minutes: u64) -> AutoTaskAddParams {
    AutoTaskAddParams {
        name: name.to_string(),
        description: format!("Auto-task {name}"),
        schedule: AutoTaskSchedule::Interval { every_minutes },
        template: template(&format!("Chore for {name}")),
        dedupe: DedupePolicy::SkipIfOpen,
    }
}
