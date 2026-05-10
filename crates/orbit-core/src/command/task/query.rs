use orbit_common::types::{
    ExternalRef, OrbitError, Task, TaskArtifact, prune_missing_context_files,
};

use crate::OrbitRuntime;
use crate::runtime::TaskRecordUpdateParams;

use super::paths::{canonicalize_context_files_for_read, context_workspace_root};

impl OrbitRuntime {
    pub fn get_task(&self, id: &str) -> Result<Task, OrbitError> {
        self.stores()
            .tasks()
            .get(id)?
            .ok_or_else(|| OrbitError::TaskNotFound(id.to_string()))
    }

    pub fn get_task_artifacts(&self, id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
        self.stores()
            .tasks()
            .get_artifacts(id)?
            .ok_or_else(|| OrbitError::TaskNotFound(id.to_string()))
    }

    pub fn migrate_task_attribution_fields(&self, id: &str) -> Result<Task, OrbitError> {
        let task = self.get_task(id)?;
        self.stores().tasks().update(
            id,
            TaskRecordUpdateParams {
                actor: self.actor_label().to_string(),
                created_by: Some(task.created_by.clone()),
                planned_by: Some(task.planned_by.clone()),
                implemented_by: Some(task.implemented_by.clone()),
                agent: Some(task.agent.clone()),
                model: Some(task.model.clone()),
                ..Default::default()
            },
        )
    }

    pub fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        self.stores().tasks().list()
    }

    pub fn list_tasks_by_tags(&self, tags: &[String]) -> Result<Vec<Task>, OrbitError> {
        self.stores().tasks().list_by_tags(tags)
    }

    /// Returns the `context_files` entries that would be dropped if the task
    /// were re-saved through the normal write path. This does not mutate disk.
    pub fn dry_run_prune_context_files(&self, task: &Task) -> Vec<String> {
        let prune_root =
            context_workspace_root(&self.paths().repo_root, task.workspace_path.as_deref());
        let canonicalized = canonicalize_context_files_for_read(&task.context_files, &prune_root);
        let (_kept, dropped) = prune_missing_context_files(&prune_root, canonicalized);
        dropped
    }

    pub fn list_tasks_filtered(
        &self,
        status: Option<orbit_common::types::TaskStatus>,
        priority: Option<orbit_common::types::TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
        external_ref: Option<&ExternalRef>,
        has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.stores().tasks().list_filtered(
            status,
            priority,
            parent_id,
            batch_id,
            external_ref,
            has_external_ref_system,
        )
    }

    pub fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.stores().tasks().search(query)
    }

    pub fn search_tasks_filtered(
        &self,
        query: &str,
        tags: &[String],
    ) -> Result<Vec<Task>, OrbitError> {
        self.stores().tasks().search_filtered(query, tags)
    }
}
