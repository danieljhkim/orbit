use orbit_types::{OrbitError, Task, TaskArtifact, prune_missing_context_files};

use crate::OrbitRuntime;
use crate::runtime::TaskRecordUpdateParams;

use super::paths::context_workspace_root;

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

    /// Returns the `context_files` entries that would be dropped if the task
    /// were re-saved through the normal write path. This does not mutate disk.
    pub fn dry_run_prune_context_files(&self, task: &Task) -> Vec<String> {
        let prune_root =
            context_workspace_root(&self.paths().repo_root, task.workspace_path.as_deref());
        let (_kept, dropped) = prune_missing_context_files(&prune_root, task.context_files.clone());
        dropped
    }

    pub fn list_tasks_filtered(
        &self,
        status: Option<orbit_types::TaskStatus>,
        priority: Option<orbit_types::TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.stores()
            .tasks()
            .list_filtered(status, priority, parent_id, batch_id)
    }

    pub fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.stores().tasks().search(query)
    }
}
