use orbit_engine::{TaskAutomationUpdate, TaskReadHost, TaskWriteHost};
use orbit_types::{
    OrbitError, OrbitEvent, Task, TaskPriority, TaskStatus, normalize_optional_attribution_label,
};

use crate::OrbitRuntime;
use crate::runtime::TaskRecordUpdateParams as StoreTaskUpdateParams;

impl TaskReadHost for OrbitRuntime {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        OrbitRuntime::get_task(self, task_id)
    }

    fn get_task_artifacts(
        &self,
        task_id: &str,
    ) -> Result<Vec<orbit_types::TaskArtifact>, OrbitError> {
        OrbitRuntime::get_task_artifacts(self, task_id)
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        OrbitRuntime::list_tasks_filtered(self, status, priority, parent_id, batch_id)
    }
}

impl TaskWriteHost for OrbitRuntime {
    fn start_task(
        &self,
        task_id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        OrbitRuntime::start_task(self, task_id, note, comment)
    }

    fn update_task_from_activity(
        &self,
        task_id: &str,
        status: TaskStatus,
        execution_summary: Option<String>,
        comment: Option<String>,
        note: Option<String>,
    ) -> Result<Task, OrbitError> {
        OrbitRuntime::update_task_from_activity(
            self,
            task_id,
            status,
            execution_summary,
            comment,
            note,
        )
    }

    fn apply_task_automation_update(
        &self,
        task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        let existing_task = self.get_task(task_id)?;
        if update.status == Some(TaskStatus::InProgress) {
            if crate::command::task::in_progress_transition_requires_plan(existing_task.status) {
                crate::command::task::ensure_task_has_execution_plan(
                    task_id,
                    existing_task.plan.as_str(),
                )?;
            }
        }
        let _ = self.with_mutation(|| {
            let (agent, model) = self
                .canonical_agent_model_identity(update.agent.as_deref(), update.model.as_deref());
            let actor_label = normalize_optional_attribution_label(
                update
                    .model
                    .as_deref()
                    .or(update.agent.as_deref())
                    .or(Some("agent")),
                existing_task.model.as_deref().or(model.as_deref()),
            )
            .unwrap_or_else(|| "agent".to_string());
            let implemented_by = normalize_optional_attribution_label(
                existing_task
                    .model
                    .as_deref()
                    .or(model.as_deref())
                    .or(existing_task.implemented_by.as_deref())
                    .or(Some(actor_label.as_str())),
                existing_task.model.as_deref().or(model.as_deref()),
            );
            let task = self.stores().tasks().update(
                task_id,
                StoreTaskUpdateParams {
                    actor: actor_label.clone(),
                    execution_summary: update.execution_summary.clone(),
                    plan: update.plan.clone(),
                    planned_by: update.plan.as_ref().map(|_| Some(actor_label.clone())),
                    implemented_by: if matches!(
                        update.status,
                        Some(TaskStatus::Review | TaskStatus::Done)
                    ) {
                        implemented_by.clone().map(Some)
                    } else {
                        None
                    },
                    agent: agent.clone().map(Some),
                    model: model.clone().map(Some),
                    status: update.status,
                    workspace_path: update.workspace_path.clone(),
                    repo_root: update.repo_root.clone().map(Some),
                    pr_number: update.pr_number.clone().map(Some),
                    batch_id: update.batch_id.clone().map(Some),
                    status_event: update.status_event.clone(),
                    status_note: update.status_note.clone(),
                    append_comments: update.append_comments.clone(),
                    replace_review_threads: update.review_threads.clone(),
                    ..Default::default()
                },
            )?;
            Ok((
                task.clone(),
                OrbitEvent::TaskUpdated {
                    id: task_id.to_string(),
                },
            ))
        })?;
        Ok(())
    }
}
