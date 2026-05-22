use orbit_common::types::{
    ExternalRef, OrbitError, OrbitEvent, ReviewThread, Task, TaskComment, TaskHistoryEntry,
    TaskPriority, TaskStatus, normalize_optional_attribution_label, push_external_ref_if_missing,
};
use orbit_engine::{
    RuntimeHost, TaskActivityUpdate, TaskAutomationUpdate, TaskReadHost, TaskWriteHost,
};

use crate::OrbitRuntime;
use crate::command::task::SYSTEM_ACTOR_LABEL;
use crate::runtime::TaskRecordUpdateParams as StoreTaskUpdateParams;

impl TaskReadHost for OrbitRuntime {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        OrbitRuntime::get_task(self, task_id)
    }

    fn get_task_artifacts(
        &self,
        task_id: &str,
    ) -> Result<Vec<orbit_common::types::TaskArtifact>, OrbitError> {
        OrbitRuntime::get_task_artifacts(self, task_id)
    }

    fn get_task_comments(&self, task_id: &str) -> Result<Vec<TaskComment>, OrbitError> {
        OrbitRuntime::get_task_comments(self, task_id)
    }

    fn get_task_history(&self, task_id: &str) -> Result<Vec<TaskHistoryEntry>, OrbitError> {
        OrbitRuntime::get_task_history(self, task_id)
    }

    fn get_task_review_threads(&self, task_id: &str) -> Result<Vec<ReviewThread>, OrbitError> {
        OrbitRuntime::get_task_review_threads(self, task_id)
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        job_run_id: Option<&str>,
        external_ref: Option<&ExternalRef>,
        has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        OrbitRuntime::list_tasks_filtered(
            self,
            status,
            priority,
            parent_id,
            job_run_id,
            external_ref,
            has_external_ref_system,
        )
    }
}

impl TaskWriteHost for OrbitRuntime {
    fn start_task(
        &self,
        task_id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        OrbitRuntime::start_task_as_system(self, task_id, note, comment)
    }

    fn admit_task_for_workflow(&self, task_id: &str, workflow: &str) -> Result<Task, OrbitError> {
        OrbitRuntime::admit_task_for_workflow_as_system(self, task_id, workflow)
    }

    fn update_task_from_activity(
        &self,
        task_id: &str,
        update: TaskActivityUpdate,
    ) -> Result<Task, OrbitError> {
        OrbitRuntime::update_task_from_activity(self, task_id, update)
    }

    fn apply_task_automation_update(
        &self,
        task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        let existing_task = self.get_task(task_id)?;
        if update.status == Some(TaskStatus::Friction)
            && existing_task.status != TaskStatus::Friction
        {
            return Err(OrbitError::InvalidInput(format!(
                "status 'friction' can only be set at creation; task '{task_id}' is currently '{}'",
                existing_task.status
            )));
        }
        if update.status == Some(TaskStatus::InProgress)
            && crate::command::task::in_progress_transition_requires_plan(existing_task.status)
        {
            crate::command::task::ensure_task_has_execution_plan(
                task_id,
                existing_task.plan.as_str(),
            )?;
        }
        let (agent, model) = self
            .try_canonical_agent_model_identity(update.agent.as_deref(), update.model.as_deref())?;
        let runtime_model_identity = <Self as RuntimeHost>::actor_model_identity(self);
        let task = self.with_mutation(|| {
            let actor_label = SYSTEM_ACTOR_LABEL.to_string();
            let explicit_attribution_label = normalize_optional_attribution_label(
                update
                    .model
                    .as_deref()
                    .or(model.as_deref())
                    .or(update.agent.as_deref())
                    .or(agent.as_deref()),
                model.as_deref(),
            );
            let planned_by = update.plan.as_ref().map(|_| {
                Some(
                    explicit_attribution_label
                        .clone()
                        .or_else(|| runtime_model_identity.clone())
                        .unwrap_or_else(|| actor_label.clone()),
                )
            });
            let implemented_by = if let Some(existing) = existing_task.implemented_by.as_deref() {
                normalize_optional_attribution_label(Some(existing), None)
            } else {
                normalize_optional_attribution_label(
                    model
                        .as_deref()
                        .or(explicit_attribution_label.as_deref())
                        .or(runtime_model_identity.as_deref())
                        .or(Some(actor_label.as_str())),
                    model.as_deref(),
                )
            };
            let external_refs = if update.external_refs.is_empty() {
                None
            } else {
                let mut refs = existing_task.external_refs.clone();
                for external_ref in update.external_refs.clone() {
                    push_external_ref_if_missing(&mut refs, external_ref);
                }
                Some(refs)
            };
            let task = self.stores().tasks().update(
                task_id,
                StoreTaskUpdateParams {
                    actor: actor_label.clone(),
                    execution_summary: update.execution_summary.clone(),
                    plan: update.plan.clone(),
                    context_files: update.context_files.clone(),
                    planned_by,
                    implemented_by: if matches!(
                        update.status,
                        Some(TaskStatus::Review | TaskStatus::Done)
                    ) {
                        implemented_by.clone().map(Some)
                    } else {
                        None
                    },
                    status: update.status,
                    external_refs,
                    job_run_id: update.job_run_id.clone().map(Some),
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
        if task.status == TaskStatus::Done {
            self.record_resolves_side_effects(&task)?;
        }
        Ok(())
    }
}

