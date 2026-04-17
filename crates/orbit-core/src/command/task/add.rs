use orbit_store::{
    TaskCreateParams as StoreTaskCreateParams, TaskUpdateParams as StoreTaskUpdateParams,
    friction_bounty,
};
use orbit_types::{OrbitError, OrbitEvent, Task, prune_missing_context_files};

use crate::OrbitRuntime;
use crate::context::ActorKind;

use super::helpers::{authored_role_value, build_task_comments, effective_actor_label};
use super::params::TaskAddParams;
use super::paths::{
    context_files_pruned_history_entry, context_workspace_root, normalize_workspace_path,
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
            self.canonical_agent_model_identity(agent.as_deref(), model.as_deref());
        let actor = self.actor().clone();
        let effective_label = effective_actor_label(
            &actor.label,
            canonical_agent.as_deref(),
            canonical_model.as_deref(),
        );
        let initial_status =
            if actor.kind == ActorKind::Agent && self.task_approval_required_for_agent() {
                orbit_types::TaskStatus::Proposed
            } else {
                orbit_types::TaskStatus::Backlog
            };
        let uses_system_identity = params.system_created;
        let create_label = if uses_system_identity {
            "system".to_string()
        } else {
            effective_label.clone()
        };
        let planned_by = authored_role_value(params.plan.as_str(), &create_label);
        let comments = build_task_comments(params.comment.clone(), effective_label.as_str())?;
        let workspace_path =
            normalize_workspace_path(&self.paths().repo_root, params.workspace_path.as_deref())?;

        let prune_root = context_workspace_root(&self.paths().repo_root, workspace_path.as_deref());
        let (kept_context_files, dropped_context_files) =
            prune_missing_context_files(&prune_root, params.context_files.clone());

        let task = self.with_mutation(|| {
            let task = self.stores().tasks().create(StoreTaskCreateParams {
                actor: create_label.clone(),
                parent_id: params.parent_id.clone(),
                title: params.title.clone(),
                description: params.description.clone(),
                acceptance_criteria: params.acceptance_criteria.clone(),
                plan: params.plan.clone(),
                execution_summary: String::new(),
                context_files: kept_context_files.clone(),
                workspace_path: workspace_path.clone(),
                repo_root: None,
                created_by: Some(create_label.clone()),
                planned_by,
                implemented_by: None,
                agent: canonical_agent.clone(),
                model: canonical_model.clone(),
                status: initial_status,
                priority: params.priority,
                complexity: params.complexity,
                task_type: params.task_type,
                pr_number: None,
                source_task_id: params.source_task_id.clone(),
                comments: comments.clone(),
            })?;
            Ok((
                task.clone(),
                OrbitEvent::TaskAdded {
                    id: task.id.clone(),
                },
            ))
        })?;

        if self.scoring_enabled() && params.task_type.counts_toward_friction_bounty() {
            if let Some(model) = &canonical_model {
                let _ =
                    friction_bounty::record_friction_reported(&self.paths().scoreboard_dir, model);
            }
        }

        let task = if dropped_context_files.is_empty() {
            task
        } else {
            self.stores().tasks().update(
                &task.id,
                StoreTaskUpdateParams {
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
