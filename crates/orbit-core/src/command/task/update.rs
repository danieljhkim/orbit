use orbit_common::types::{
    OrbitError, OrbitEvent, Task, TaskHistoryEntry, TaskStatus, normalize_task_dependencies,
    normalize_task_tags, prune_missing_context_files, validate_task_dependencies,
};
use orbit_engine::TaskActivityUpdate;

use crate::OrbitRuntime;
use crate::runtime::TaskRecordUpdateParams;

use super::helpers::{
    SYSTEM_ACTOR_LABEL, build_task_comments, effective_actor_label, implementation_label,
    task_comment_history_entries,
};
use super::params::TaskUpdateParams;
use super::paths::{
    canonicalize_context_files_for_read, context_files_pruned_history_entry,
    context_workspace_root, emit_graph_unavailable_warning_if_needed,
    normalize_context_files_for_write,
};
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
        update: TaskActivityUpdate,
    ) -> Result<Task, OrbitError> {
        let TaskActivityUpdate {
            status,
            execution_summary,
            comment,
            note,
            agent,
            model,
        } = update;
        self.update_task_with_status_note_and_identity(
            id,
            TaskUpdateParams {
                execution_summary,
                comment,
                status: Some(status),
                ..Default::default()
            },
            note,
            agent.or_else(|| model.is_none().then(|| SYSTEM_ACTOR_LABEL.to_string())),
            model,
        )
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
            self.try_canonical_agent_model_identity(agent.as_deref(), model.as_deref())?;
        let task = self.get_task(id)?;
        let prune_root = context_workspace_root(&self.paths().repo_root, None);

        let dropped_context_files: Vec<String> = if let Some(candidates) =
            params.context_files.take()
        {
            let normalized = normalize_context_files_for_write(candidates, &prune_root)?;
            emit_graph_unavailable_warning_if_needed(&normalized, self.data_root_path());
            let (kept, dropped) = prune_missing_context_files(&prune_root, normalized);
            params.context_files = Some(kept);
            dropped
        } else {
            let normalized = canonicalize_context_files_for_read(&task.context_files, &prune_root);
            if normalized != task.context_files {
                emit_graph_unavailable_warning_if_needed(&normalized, self.data_root_path());
                let (kept, dropped) = prune_missing_context_files(&prune_root, normalized);
                params.context_files = Some(kept);
                dropped
            } else {
                Vec::new()
            }
        };
        if let Some(dependencies) = params.dependencies.take() {
            let normalized_dependencies = normalize_task_dependencies(dependencies)?;
            validate_task_dependencies(&self.list_tasks()?, Some(id), &normalized_dependencies)?;
            params.dependencies = Some(normalized_dependencies);
        }
        if let Some(tags) = params.tags.take() {
            params.tags = Some(normalize_task_tags(tags));
        }
        if let Some(crew) = &params.crew {
            self.validate_crew_name(crew.as_deref())?;
        }
        if params.has_any_mutation() && task.status == TaskStatus::Archived {
            return Err(OrbitError::InvalidInput(format!(
                "task {id} is {} and cannot be modified; unarchive or reopen it first",
                task.status
            )));
        }
        if params.has_non_comment_mutation() && task.status == TaskStatus::Done {
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
            if target_status == TaskStatus::Friction && task.status != TaskStatus::Friction {
                let history = self.get_task_history(id)?;
                return Err(OrbitError::InvalidInput(friction_reentry_error(
                    id,
                    task.status,
                    &history,
                )));
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
        let planned_by = params
            .planned_by
            .clone()
            .or_else(|| params.plan.as_ref().map(|_| Some(effective_label.clone())));
        let implementation_label =
            implementation_label(&task, effective_label.as_str(), canonical_model.as_deref());
        let implemented_by = params.implemented_by.clone().or_else(|| {
            params.status.and_then(|status| {
                if matches!(status, TaskStatus::Review | TaskStatus::Done) {
                    implementation_label.clone().map(Some)
                } else {
                    None
                }
            })
        });
        let source_task_id_changed = params
            .source_task_id
            .as_ref()
            .is_some_and(|source_task_id| task.source_task_id() != source_task_id.as_deref());

        let mut append_history: Vec<TaskHistoryEntry> = if dropped_context_files.is_empty() {
            Vec::new()
        } else {
            vec![context_files_pruned_history_entry(
                effective_label.as_str(),
                &dropped_context_files,
            )]
        };
        append_history.extend(task_comment_history_entries(&append_comments));
        if source_task_id_changed {
            append_history.push(TaskHistoryEntry {
                at: chrono::Utc::now(),
                by: effective_label.clone(),
                event: "updated".to_string(),
                note: Some("source_task_id changed".to_string()),
                from_status: None,
                to_status: None,
            });
        }
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

        Ok(updated)
    }
}

fn friction_reentry_error(
    id: &str,
    current_status: TaskStatus,
    history: &[TaskHistoryEntry],
) -> String {
    if let Some(entry) = history
        .iter()
        .rev()
        .find(|entry| entry.from_status == Some(TaskStatus::Friction))
    {
        let to_status = entry
            .to_status
            .map(|status| status.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        return format!(
            "status 'friction' can only be set at creation; task '{id}' previously transitioned out of friction (friction -> {to_status})"
        );
    }

    format!(
        "status 'friction' can only be set at creation; task '{id}' is currently '{}'",
        current_status
    )
}
