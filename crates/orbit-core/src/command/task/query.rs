use std::collections::BTreeMap;

use orbit_common::types::{
    ArtifactManifestFileV2, ExternalRef, NotFoundKind, OrbitError, Task, TaskArtifact, TaskComment,
    TaskHistoryEntry, prune_missing_context_files,
};

use crate::OrbitRuntime;

use super::paths::{canonicalize_context_files_for_read, context_workspace_root};

impl OrbitRuntime {
    pub fn get_task(&self, id: &str) -> Result<Task, OrbitError> {
        self.stores()
            .tasks()
            .get(id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, id.to_string()))
    }

    pub fn get_task_artifacts(&self, id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
        self.stores()
            .tasks()
            .get_artifacts(id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, id.to_string()))
    }

    pub fn get_task_artifact_manifest(
        &self,
        id: &str,
    ) -> Result<Vec<ArtifactManifestFileV2>, OrbitError> {
        self.stores()
            .tasks()
            .get_artifact_manifest(id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, id.to_string()))
    }

    pub fn get_task_artifact(
        &self,
        id: &str,
        path: &str,
    ) -> Result<Option<TaskArtifact>, OrbitError> {
        self.stores().tasks().get_artifact(id, path)
    }

    pub fn get_task_comments(&self, id: &str) -> Result<Vec<TaskComment>, OrbitError> {
        self.stores()
            .tasks()
            .get_comments(id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, id.to_string()))
    }

    pub fn get_task_history(&self, id: &str) -> Result<Vec<TaskHistoryEntry>, OrbitError> {
        self.stores()
            .tasks()
            .get_history(id)?
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, id.to_string()))
    }

    pub fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        self.stores().tasks().list()
    }

    /// Returns the coordination registry's global status projection for
    /// dependency readiness while leaving task listing workspace-scoped.
    pub fn task_status_index(
        &self,
    ) -> Result<BTreeMap<String, orbit_common::types::TaskStatus>, OrbitError> {
        self.stores().tasks().status_index()
    }

    pub fn list_tasks_by_tags(&self, tags: &[String]) -> Result<Vec<Task>, OrbitError> {
        self.stores().tasks().list_by_tags(tags)
    }

    /// Returns the `context_files` entries that would be dropped if the task
    /// were re-saved through the normal write path. This does not mutate disk.
    pub fn dry_run_prune_context_files(&self, task: &Task) -> Vec<String> {
        let prune_root = context_workspace_root(&self.paths().repo_root, None);
        let canonicalized = canonicalize_context_files_for_read(&task.context_files, &prune_root);
        let (_kept, dropped) = prune_missing_context_files(&prune_root, canonicalized);
        dropped
    }

    /// Drops `context_files` entries whose anchors no longer exist in the
    /// workspace (the `orbit task lint --fix` path, formerly the standalone
    /// `prune-context` backfill). Returns the updated task and the dropped
    /// entries; no write happens when nothing is stale.
    pub fn prune_task_context_files(&self, id: &str) -> Result<(Task, Vec<String>), OrbitError> {
        let task = self.get_task(id)?;
        let prune_root = context_workspace_root(&self.paths().repo_root, None);
        let canonicalized = canonicalize_context_files_for_read(&task.context_files, &prune_root);
        let (kept, dropped) = prune_missing_context_files(&prune_root, canonicalized);
        if dropped.is_empty() {
            return Ok((task, dropped));
        }
        let updated = self.update_task(
            id,
            super::TaskUpdateParams {
                context_files: Some(kept),
                ..Default::default()
            },
        )?;
        Ok((updated, dropped))
    }

    pub fn list_tasks_filtered(
        &self,
        status: Option<orbit_common::types::TaskStatus>,
        priority: Option<orbit_common::types::TaskPriority>,
        parent_id: Option<&str>,
        job_run_id: Option<&str>,
        external_ref: Option<&ExternalRef>,
        has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.stores().tasks().list_filtered(
            status,
            priority,
            parent_id,
            job_run_id,
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
