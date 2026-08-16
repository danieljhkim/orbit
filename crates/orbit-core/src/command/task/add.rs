use orbit_common::types::{
    OrbitError, OrbitEvent, Task, TaskStatus, TaskType, normalize_task_dependencies,
    normalize_task_tags, prune_missing_context_files,
};
use orbit_common::utility::redaction::redact_all;
use orbit_store::TaskCreateParams as StoreTaskCreateParams;

use super::TaskRecordUpdateParams;
use crate::OrbitRuntime;

use super::helpers::{authored_role_value, build_task_comments, effective_actor_label};
use super::params::TaskAddParams;
use super::paths::{
    context_files_pruned_history_entry, context_workspace_root, normalize_context_files_for_write,
    normalize_workspace_path,
};

impl OrbitRuntime {
    pub fn add_task(&self, params: TaskAddParams) -> Result<Task, OrbitError> {
        self.add_task_with_identity(params, None, None)
    }

    pub fn add_task_with_identity(
        &self,
        mut params: TaskAddParams,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.ensure_coordination_task_write_permitted()?;
        // [ORB-00417] Redact secrets at the single task-creation choke point
        // (shared by the dashboard POST, CLI `task add`, and the MCP task tool)
        // so a pasted key never lands in the task registry or the audit trail.
        // `redact_all` is idempotent, so read-time redaction still composes.
        params.title = redact_all(&params.title);
        params.description = redact_all(&params.description);
        params.plan = redact_all(&params.plan);
        for criterion in params.acceptance_criteria.iter_mut() {
            *criterion = redact_all(criterion);
        }
        params.comment = params.comment.map(|comment| redact_all(&comment));

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
        self.validate_crew_name(params.crew.as_deref())?;
        params.orchestrator = self.canonical_crew_name(params.orchestrator.as_deref())?;
        if params.orchestrator.is_some()
            && !matches!(initial_status, TaskStatus::Proposed | TaskStatus::Backlog)
        {
            return Err(OrbitError::InvalidInput(format!(
                "initial status {initial_status} cannot carry an orchestrator; orchestrator can only be set while proposed or backlog"
            )));
        }

        let prune_root = context_workspace_root(&self.paths().repo_root, workspace_path.as_deref());
        let normalized_context_files =
            normalize_context_files_for_write(params.context_files.clone(), &prune_root)?;
        let (kept_context_files, dropped_context_files) =
            prune_missing_context_files(&prune_root, normalized_context_files);

        let task = self.with_mutation(|| {
            let task = self.stores().task_records().create(StoreTaskCreateParams {
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
                orchestrator: params.orchestrator.clone(),
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
            self.stores().task_records().update(
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

    Ok((
        requested_type.unwrap_or(TaskType::Chore),
        requested_status.unwrap_or(default_status),
    ))
}
