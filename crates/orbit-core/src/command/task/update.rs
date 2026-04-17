use orbit_types::{
    OrbitError, OrbitEvent, Task, TaskHistoryEntry, TaskStatus, prune_missing_context_files,
};

use crate::OrbitRuntime;
use crate::runtime::TaskRecordUpdateParams;

use super::helpers::{build_task_comments, effective_actor_label};
use super::params::TaskUpdateParams;
use super::paths::{context_files_pruned_history_entry, context_workspace_root};
use super::transitions::{ensure_task_has_execution_plan, in_progress_transition_requires_plan};

impl OrbitRuntime {
    pub fn update_task(&self, id: &str, params: TaskUpdateParams) -> Result<Task, OrbitError> {
        self.update_task_with_identity(id, params, None, None)
    }

    pub fn update_task_with_identity(
        &self,
        id: &str,
        params: TaskUpdateParams,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.update_task_with_status_note_and_identity(id, params, None, agent, model)
    }

    pub fn update_task_from_activity(
        &self,
        id: &str,
        status: TaskStatus,
        execution_summary: Option<String>,
        comment: Option<String>,
        note: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.update_task_with_status_note(
            id,
            TaskUpdateParams {
                execution_summary,
                comment,
                status: Some(status),
                ..Default::default()
            },
            note,
        )
    }

    fn update_task_with_status_note(
        &self,
        id: &str,
        params: TaskUpdateParams,
        status_note: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.update_task_with_status_note_and_identity(id, params, status_note, None, None)
    }

    fn update_task_with_status_note_and_identity(
        &self,
        id: &str,
        mut params: TaskUpdateParams,
        status_note: Option<String>,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Task, OrbitError> {
        let (canonical_agent, canonical_model) =
            self.canonical_agent_model_identity(agent.as_deref(), model.as_deref());
        let task = self.get_task(id)?;

        let dropped_context_files: Vec<String> =
            if let Some(candidates) = params.context_files.take() {
                let prune_root =
                    context_workspace_root(&self.paths().repo_root, task.workspace_path.as_deref());
                let (kept, dropped) = prune_missing_context_files(&prune_root, candidates);
                params.context_files = Some(kept);
                dropped
            } else {
                Vec::new()
            };
        let locked_field_update = params.plan.is_some()
            || params.execution_summary.is_some()
            || params.comment.is_some()
            || params.pr_number.is_some()
            || params.pr_status.is_some()
            || params.batch_id.is_some()
            || !params.upsert_artifacts.is_empty();

        if locked_field_update && matches!(task.status, TaskStatus::Done | TaskStatus::Archived) {
            return Err(OrbitError::InvalidInput(format!(
                "task {id} is {} and cannot be modified; unarchive or reopen it first",
                task.status
            )));
        }

        if let Some(target_status) = params.status {
            if target_status == TaskStatus::Archived {
                return Err(OrbitError::InvalidInput(
                    "use `orbit task archive <id>` instead of setting status to archived"
                        .to_string(),
                ));
            }
            task.status
                .validate_transition(target_status)
                .map_err(OrbitError::TaskStatusTransition)?;
            if target_status == TaskStatus::InProgress
                && task.status != TaskStatus::InProgress
                && in_progress_transition_requires_plan(task.status)
            {
                let effective_plan = params.plan.as_deref().unwrap_or(task.plan.as_str());
                ensure_task_has_execution_plan(id, effective_plan)?;
            }
        }

        if task.status == TaskStatus::InProgress && params.status == Some(TaskStatus::Review) {
            let effective_execution_summary = params
                .execution_summary
                .as_deref()
                .unwrap_or(task.execution_summary.as_str());
            if effective_execution_summary.trim().is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "task '{id}' requires non-empty execution_summary before transitioning in-progress -> review"
                )));
            }
        }

        let actor = self.actor().clone();
        let effective_label = effective_actor_label(
            &actor.label,
            canonical_agent.as_deref(),
            canonical_model.as_deref(),
        );
        let status_note = status_note
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let append_comments =
            build_task_comments(params.comment.clone(), effective_label.as_str())?;
        let planned_by = params.plan.as_ref().map(|_| Some(effective_label.clone()));
        let implemented_by = params.status.and_then(|status| {
            if matches!(status, TaskStatus::Review | TaskStatus::Done) {
                Some(Some(effective_label.clone()))
            } else {
                None
            }
        });

        let old_status = task.status;
        let target_status = params.status;
        let append_history: Vec<TaskHistoryEntry> = if dropped_context_files.is_empty() {
            Vec::new()
        } else {
            vec![context_files_pruned_history_entry(
                effective_label.as_str(),
                &dropped_context_files,
            )]
        };
        let updated = self.with_mutation(|| {
            let task = self.stores().tasks().update(
                id,
                TaskRecordUpdateParams {
                    actor: effective_label.clone(),
                    planned_by,
                    implemented_by,
                    status_note,
                    append_comments: append_comments.clone(),
                    append_history: append_history.clone(),
                    ..TaskRecordUpdateParams::from(params)
                },
            )?;
            Ok((task.clone(), OrbitEvent::TaskUpdated { id: id.to_string() }))
        })?;

        if let Some(new_status) = target_status {
            if new_status != old_status {
                self.try_record_friction_transition(&task, old_status, new_status);
            }
        }

        Ok(updated)
    }
}
