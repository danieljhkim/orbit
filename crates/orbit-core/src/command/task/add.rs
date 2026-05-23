use orbit_common::types::{
    OrbitError, OrbitEvent, Task, TaskStatus, TaskType, normalize_task_dependencies,
    normalize_task_tags, prune_missing_context_files, validate_task_dependencies,
};
use orbit_store::TaskCreateParams as StoreTaskCreateParams;

use crate::OrbitRuntime;
use crate::runtime::TaskRecordUpdateParams;

use super::helpers::{authored_role_value, build_task_comments, effective_actor_label};
use super::params::TaskAddParams;
use super::paths::{
    context_files_pruned_history_entry, context_workspace_root,
    emit_graph_unavailable_warning_if_needed, normalize_context_files_for_write,
    normalize_workspace_path,
};

impl OrbitRuntime {
    pub fn add_task(&self, params: TaskAddParams) -> Result<Task, OrbitError> {
        self.add_task_with_identity(params, None, None)
    }

    pub fn add_task_with_identity(
        &self,
        params: TaskAddParams,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Task, OrbitError> {
        let (canonical_agent, canonical_model) =
            self.try_canonical_agent_model_identity(agent.as_deref(), model.as_deref())?;
        let actor = self.actor().clone();
        let effective_label = effective_actor_label(
            &actor.label,
            canonical_agent.as_deref(),
            canonical_model.as_deref(),
        );
        let (task_type, initial_status) = infer_task_create_type_and_status(
            params.task_type,
            params.status,
            TaskStatus::Proposed,
        )?;
        let uses_system_identity = params.system_created;
        let create_label = if uses_system_identity {
            "system".to_string()
        } else {
            effective_label.clone()
        };
        let planned_by = authored_role_value(params.plan.as_str(), &create_label);
        let comments = build_task_comments(params.comment.clone(), create_label.as_str())?;
        let workspace_path =
            normalize_workspace_path(&self.paths().repo_root, params.workspace_path.as_deref())?;
        let dependencies = normalize_task_dependencies(params.dependencies.clone())?;
        validate_task_dependencies(&self.list_tasks()?, None, &dependencies)?;
        self.validate_crew_name(params.crew.as_deref())?;

        let prune_root = context_workspace_root(&self.paths().repo_root, workspace_path.as_deref());
        let normalized_context_files =
            normalize_context_files_for_write(params.context_files.clone(), &prune_root)?;
        emit_graph_unavailable_warning_if_needed(&normalized_context_files, self.data_root_path());
        let (kept_context_files, dropped_context_files) =
            prune_missing_context_files(&prune_root, normalized_context_files);

        let task = self.with_mutation(|| {
            let task = self.stores().tasks().create(StoreTaskCreateParams {
                actor: create_label.clone(),
                parent_id: params.parent_id.clone(),
                title: params.title.clone(),
                description: params.description.clone(),
                acceptance_criteria: params.acceptance_criteria.clone(),
                dependencies: dependencies.clone(),
                relations: params.relations.clone(),
                tags: normalize_task_tags(params.tags.clone()),
                plan: params.plan.clone(),
                execution_summary: String::new(),
                context_files: kept_context_files.clone(),
                workspace_path: workspace_path.clone(),
                repo_root: None,
                created_by: Some(create_label.clone()),
                planned_by,
                implemented_by: None,
                status: initial_status,
                priority: params.priority,
                complexity: params.complexity,
                task_type,
                external_refs: params.external_refs.clone(),
                source_task_id: params.source_task_id.clone(),
                crew: params.crew.clone(),
                comments: comments.clone(),
            })?;
            Ok((
                task.clone(),
                OrbitEvent::TaskAdded {
                    id: task.id.clone(),
                },
            ))
        })?;

        let task = if dropped_context_files.is_empty() {
            task
        } else {
            self.stores().tasks().update(
                &task.id,
                TaskRecordUpdateParams {
                    actor: create_label.clone(),
                    append_history: vec![context_files_pruned_history_entry(
                        &create_label,
                        &dropped_context_files,
                    )],
                    ..Default::default()
                },
            )?
        };

        Ok(task)
    }
}

fn infer_task_create_type_and_status(
    requested_type: Option<TaskType>,
    requested_status: Option<TaskStatus>,
    default_status: TaskStatus,
) -> Result<(TaskType, TaskStatus), OrbitError> {
    if requested_status == Some(TaskStatus::Archived) {
        return Err(OrbitError::InvalidInput(
            "status 'archived' cannot be set at task creation; use the archive command".to_string(),
        ));
    }

    if requested_status == Some(TaskStatus::Friction) {
        return Err(OrbitError::InvalidInput(
            "friction reports are no longer tasks; use orbit.friction.add".to_string(),
        ));
    }

    Ok((
        requested_type.unwrap_or(TaskType::Chore),
        requested_status.unwrap_or(default_status),
    ))
}
