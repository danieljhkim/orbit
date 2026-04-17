use std::fs;
use std::path::PathBuf;

use chrono::Utc;
use orbit_types::{
    ActorIdentity, OrbitError, Task, TaskArtifact, TaskHistoryEntry, TaskPriority, TaskStatus,
};

use super::bundle::{TaskBundle, bundle_to_task, merge_review_threads};
use super::constants::TASK_SCHEMA_VERSION;
use super::doc::TaskFileDocument;
use super::layout::TaskStateDir;
use crate::backend::{TaskCreateParams, TaskUpdateParams};

pub(crate) struct TaskFileStore {
    pub(super) root: PathBuf,
}

impl TaskFileStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn create_task(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        self.ensure_layout()?;
        if params.title.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task title must not be empty".to_string(),
            ));
        }
        if params.actor.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task actor must not be empty".to_string(),
            ));
        }

        let now = Utc::now();
        let id = self.next_task_id(now)?;
        let initial_state = TaskStateDir::from_status(params.status);
        let bundle = TaskBundle {
            doc: TaskFileDocument {
                schema_version: TASK_SCHEMA_VERSION,
                id,
                parent_id: params.parent_id,
                title: params.title,
                description: params.description,
                acceptance_criteria: params.acceptance_criteria,
                context_files: params.context_files,
                workspace_path: params.workspace_path,
                repo_root: params.repo_root,
                created_by: params.created_by,
                planned_by: params.planned_by,
                implemented_by: params.implemented_by,
                agent: params.agent,
                model: params.model,
                priority: params.priority,
                complexity: params.complexity,
                task_type: params.task_type,
                pr_number: params.pr_number,
                pr_status: None,
                actor_identity: ActorIdentity::default(),
                assigned_to: None,
                proposed_by: None,
                source_task_id: params.source_task_id,
                batch_id: None,
                created_at: now,
                updated_at: now,
                history: vec![TaskHistoryEntry {
                    at: now,
                    by: params.actor,
                    event: "created".to_string(),
                    note: None,
                    from_status: None,
                    to_status: Some(params.status),
                }],
                comments: params.comments,
                review_threads: Vec::new(),
            },
            plan: params.plan,
            execution_summary: params.execution_summary,
        };

        self.write_bundle_for_state(initial_state, &bundle)?;
        Ok(bundle_to_task(initial_state, bundle))
    }

    pub(crate) fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        let mut tasks = Vec::new();
        for state in TaskStateDir::all() {
            for task_dir in self.task_dirs_for_state(state)? {
                let bundle = self.read_bundle_at(&task_dir)?;
                tasks.push(bundle_to_task(state, bundle));
            }
        }

        tasks.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(tasks)
    }

    pub(crate) fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        let tasks = self.list_tasks()?;
        Ok(tasks
            .into_iter()
            .filter(|task| status.is_none_or(|value| task.status == value))
            .filter(|task| priority.is_none_or(|value| task.priority == value))
            .filter(|task| parent_id.is_none_or(|value| task.parent_id.as_deref() == Some(value)))
            .filter(|task| batch_id.is_none_or(|value| task.batch_id.as_deref() == Some(value)))
            .collect())
    }

    pub(crate) fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        let Some((state, task_dir)) = self.locate_task(id)? else {
            return Ok(None);
        };
        let bundle = self.read_bundle_at(&task_dir)?;
        Ok(Some(bundle_to_task(state, bundle)))
    }

    pub(crate) fn get_task_artifacts(
        &self,
        id: &str,
    ) -> Result<Option<Vec<TaskArtifact>>, OrbitError> {
        let Some((_, task_dir)) = self.locate_task(id)? else {
            return Ok(None);
        };
        Ok(Some(self.read_artifacts_at(&task_dir)?))
    }

    pub(crate) fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        let lowered = query.to_lowercase();
        let tasks = self.list_tasks()?;
        Ok(tasks
            .into_iter()
            .filter(|task| {
                task.title.to_lowercase().contains(&lowered)
                    || task.description.to_lowercase().contains(&lowered)
            })
            .collect())
    }

    pub(crate) fn update_task(
        &self,
        id: &str,
        fields: &TaskUpdateParams,
    ) -> Result<Task, OrbitError> {
        if fields.actor.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task actor must not be empty".to_string(),
            ));
        }
        let Some((current_state, current_dir)) = self.locate_task(id)? else {
            return Err(OrbitError::TaskNotFound(id.to_string()));
        };
        let mut bundle = self.read_bundle_at(&current_dir)?;

        let title_changed = if let Some(value) = &fields.title {
            let changed = *value != bundle.doc.title;
            bundle.doc.title = value.clone();
            changed
        } else {
            false
        };
        if let Some(value) = &fields.description {
            bundle.doc.description = value.clone();
        }
        if let Some(value) = &fields.acceptance_criteria {
            bundle.doc.acceptance_criteria = value.clone();
        }
        if let Some(value) = &fields.plan {
            bundle.plan = value.clone();
        }
        if let Some(value) = &fields.execution_summary {
            bundle.execution_summary = value.clone();
        }
        if let Some(value) = &fields.context_files {
            bundle.doc.context_files = value.clone();
        }
        if let Some(value) = &fields.workspace_path {
            bundle.doc.workspace_path = value.clone();
        }
        if let Some(value) = &fields.repo_root {
            bundle.doc.repo_root = value.clone();
        }
        if let Some(value) = &fields.created_by {
            bundle.doc.created_by = value.clone();
        }
        if let Some(value) = &fields.planned_by {
            bundle.doc.planned_by = value.clone();
        }
        if let Some(value) = &fields.implemented_by {
            bundle.doc.implemented_by = value.clone();
        }
        if let Some(value) = &fields.agent {
            bundle.doc.agent = value.clone();
        }
        if let Some(value) = &fields.model {
            bundle.doc.model = value.clone();
        }
        if let Some(value) = fields.priority {
            bundle.doc.priority = value;
        }
        if let Some(value) = fields.complexity {
            bundle.doc.complexity = Some(value);
        }
        if let Some(value) = fields.task_type {
            bundle.doc.task_type = value;
        }
        if let Some(value) = &fields.pr_number {
            bundle.doc.pr_number = value.clone();
        }
        if let Some(value) = &fields.pr_status {
            bundle.doc.pr_status = value.clone();
        }
        if let Some(value) = &fields.source_task_id {
            bundle.doc.source_task_id = value.clone();
        }
        if let Some(value) = &fields.batch_id {
            bundle.doc.batch_id = value.clone();
        }
        if !fields.append_history.is_empty() {
            bundle.doc.history.extend(fields.append_history.clone());
        }
        if !fields.append_comments.is_empty() {
            bundle.doc.comments.extend(fields.append_comments.clone());
        }
        if let Some(ref threads) = fields.replace_review_threads {
            bundle.doc.review_threads = threads.clone();
        } else if !fields.append_review_threads.is_empty() {
            merge_review_threads(
                &mut bundle.doc.review_threads,
                fields.append_review_threads.clone(),
            );
        }
        let upsert_artifacts = fields.upsert_artifacts.clone();

        let target_state = fields
            .status
            .map(TaskStateDir::from_status)
            .unwrap_or(current_state);
        let status_transition = (target_state != current_state)
            .then_some((current_state.to_status(), target_state.to_status()));

        let event = if let Some(event) = fields.status_event.clone() {
            Some(event)
        } else if target_state == current_state {
            None
        } else {
            Some("status_changed".to_string())
        };

        bundle.doc.updated_at = Utc::now();
        if let Some(event) = event {
            bundle.doc.history.push(TaskHistoryEntry {
                at: bundle.doc.updated_at,
                by: fields.actor.clone(),
                event,
                note: fields.status_note.clone(),
                from_status: status_transition.map(|(from, _)| from),
                to_status: status_transition.map(|(_, to)| to),
            });
        }
        if title_changed {
            bundle.doc.history.push(TaskHistoryEntry {
                at: bundle.doc.updated_at,
                by: fields.actor.clone(),
                event: "renamed".to_string(),
                note: None,
                from_status: None,
                to_status: None,
            });
        }

        let final_dir = if target_state == current_state {
            self.write_bundle_at(&current_dir, &bundle)?;
            current_dir.clone()
        } else {
            self.write_bundle_at(&current_dir, &bundle)?;
            let target_dir = self.task_dir(target_state, &bundle.doc.id);
            self.move_task_dir(&current_dir, &target_dir)?;
            target_dir
        };
        if !upsert_artifacts.is_empty() {
            self.write_artifacts_at(&final_dir, &upsert_artifacts)?;
        }

        Ok(bundle_to_task(target_state, bundle))
    }

    pub(crate) fn delete_task(&self, id: &str) -> Result<bool, OrbitError> {
        let Some((_, task_dir)) = self.locate_task(id)? else {
            return Ok(false);
        };
        fs::remove_dir_all(task_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        Ok(true)
    }
}
