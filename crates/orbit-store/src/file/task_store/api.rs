use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{
    ActorIdentity, ExternalRef, OrbitError, Task, TaskArtifact, TaskHistoryEntry, TaskPriority,
    TaskStatus,
};

use super::bundle::{TaskBundle, bundle_to_task, merge_review_threads};
use super::constants::TASK_SCHEMA_VERSION;
use super::doc::TaskFileDocument;
use super::layout::{TaskStateDir, validate_task_id};
use crate::backend::{
    TaskArtifactUpdateParams, TaskCreateParams, TaskDocumentUpdateParams, TaskHistoryUpdateParams,
    TaskReviewUpdateParams,
};
use crate::file::sort::sort_by_created_desc_id_asc;

pub(crate) struct TaskFileStore {
    pub(super) root: PathBuf,
}

impl TaskFileStore {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn create_task(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        let _allocation_lock = self.acquire_task_allocation_lock()?;
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
                dependencies: params.dependencies,
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
                pr_status: None,
                external_refs: params.external_refs,
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

        let task_dir = self.task_dir(initial_state, &bundle.doc.id);
        if let Err(err) = self.write_bundle_for_state(initial_state, &bundle) {
            if let Err(cleanup_err) = cleanup_partial_task_dir(&task_dir) {
                return Err(OrbitError::Store(format!(
                    "failed to create task bundle: {err}; cleanup failed: {cleanup_err}"
                )));
            }
            return Err(err);
        }
        Ok(bundle_to_task(initial_state, bundle))
    }

    pub(crate) fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        self.migrate_legacy_proposed_friction_tasks()?;
        let mut tasks = Vec::new();
        for state in TaskStateDir::all() {
            for task_dir in self.task_dirs_for_state(state)? {
                let bundle = self.read_bundle_at(&task_dir)?;
                tasks.push(bundle_to_task(state, bundle));
            }
        }

        sort_by_created_desc_id_asc(&mut tasks, |task| &task.created_at, |task| &task.id);
        Ok(tasks)
    }

    pub(crate) fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
        external_ref: Option<&ExternalRef>,
        has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.migrate_legacy_proposed_friction_tasks()?;
        let mut tasks = if let Some(status) = status {
            let state = TaskStateDir::from_status(status);
            let mut tasks = Vec::new();
            for task_dir in self.task_dirs_for_state(state)? {
                let bundle = self.read_bundle_at(&task_dir)?;
                tasks.push(bundle_to_task(state, bundle));
            }
            sort_by_created_desc_id_asc(&mut tasks, |task| &task.created_at, |task| &task.id);
            tasks
        } else {
            self.list_tasks()?
        };

        tasks.retain(|task| {
            priority.is_none_or(|value| task.priority == value)
                && parent_id.is_none_or(|value| task.parent_id.as_deref() == Some(value))
                && batch_id.is_none_or(|value| task.batch_id.as_deref() == Some(value))
                && external_ref.is_none_or(|value| {
                    task.external_refs.iter().any(|candidate| {
                        candidate.system == value.system && candidate.id == value.id
                    })
                })
                && has_external_ref_system.is_none_or(|value| {
                    task.external_refs
                        .iter()
                        .any(|candidate| candidate.system == value)
                })
        });
        Ok(tasks)
    }

    pub(crate) fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        validate_task_id(id)?;
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
        validate_task_id(id)?;
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
                    || task
                        .external_refs
                        .iter()
                        .any(|external_ref| external_ref.id.to_lowercase().contains(&lowered))
            })
            .collect())
    }

    pub(crate) fn update_task_document(
        &self,
        id: &str,
        fields: &TaskDocumentUpdateParams,
    ) -> Result<(), OrbitError> {
        validate_task_id(id)?;
        if fields.actor.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task actor must not be empty".to_string(),
            ));
        }
        let _task_lock = self.acquire_task_lock(id)?;
        let Some((current_state, current_dir)) = self.locate_task(id)? else {
            return Err(OrbitError::TaskNotFound(id.to_string()));
        };
        let mut bundle = self.read_bundle_at(&current_dir)?;
        let previous_bundle = bundle.clone();

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
        if let Some(value) = &fields.dependencies {
            bundle.doc.dependencies = value.clone();
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
        if let Some(value) = &fields.external_refs {
            bundle.doc.external_refs = value.clone();
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

        bundle.doc.updated_at = Utc::now();
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

        self.persist_bundle_update(
            current_state,
            &current_dir,
            current_state,
            &previous_bundle,
            &bundle,
        )?;
        Ok(())
    }

    pub(crate) fn update_task_history(
        &self,
        id: &str,
        fields: &TaskHistoryUpdateParams,
    ) -> Result<(), OrbitError> {
        validate_task_id(id)?;
        if fields.actor.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task actor must not be empty".to_string(),
            ));
        }
        let _task_lock = self.acquire_task_lock(id)?;
        let Some((current_state, current_dir)) = self.locate_task(id)? else {
            return Err(OrbitError::TaskNotFound(id.to_string()));
        };
        let mut bundle = self.read_bundle_at(&current_dir)?;
        let previous_bundle = bundle.clone();

        if !fields.append_history.is_empty() {
            bundle.doc.history.extend(fields.append_history.clone());
        }
        if !fields.append_comments.is_empty() {
            bundle.doc.comments.extend(fields.append_comments.clone());
        }

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

        self.persist_bundle_update(
            current_state,
            &current_dir,
            target_state,
            &previous_bundle,
            &bundle,
        )?;
        Ok(())
    }

    pub(crate) fn update_task_reviews(
        &self,
        id: &str,
        fields: &TaskReviewUpdateParams,
    ) -> Result<(), OrbitError> {
        validate_task_id(id)?;
        let _task_lock = self.acquire_task_lock(id)?;
        let Some((current_state, current_dir)) = self.locate_task(id)? else {
            return Err(OrbitError::TaskNotFound(id.to_string()));
        };
        let mut bundle = self.read_bundle_at(&current_dir)?;
        let previous_bundle = bundle.clone();

        if let Some(ref threads) = fields.replace_review_threads {
            bundle.doc.review_threads = threads.clone();
        } else if !fields.append_review_threads.is_empty() {
            merge_review_threads(
                &mut bundle.doc.review_threads,
                fields.append_review_threads.clone(),
            );
        }

        bundle.doc.updated_at = Utc::now();
        self.persist_bundle_update(
            current_state,
            &current_dir,
            current_state,
            &previous_bundle,
            &bundle,
        )?;
        Ok(())
    }

    pub(crate) fn upsert_task_artifacts(
        &self,
        id: &str,
        fields: &TaskArtifactUpdateParams,
    ) -> Result<(), OrbitError> {
        validate_task_id(id)?;
        let _task_lock = self.acquire_task_lock(id)?;
        let Some((_, task_dir)) = self.locate_task(id)? else {
            return Err(OrbitError::TaskNotFound(id.to_string()));
        };
        if !fields.upsert_artifacts.is_empty() {
            self.write_artifacts_at(&task_dir, &fields.upsert_artifacts)?;
        }
        Ok(())
    }

    fn persist_bundle_update(
        &self,
        current_state: TaskStateDir,
        current_dir: &Path,
        target_state: TaskStateDir,
        previous_bundle: &TaskBundle,
        bundle: &TaskBundle,
    ) -> Result<(), OrbitError> {
        if let Err(err) = self.write_bundle_at(current_dir, bundle) {
            return self.rollback_bundle_update(current_dir, previous_bundle, err);
        }

        if target_state != current_state {
            let target_dir = self.task_dir(target_state, &bundle.doc.id);
            if let Err(err) = self.move_task_dir(current_dir, &target_dir) {
                return self.rollback_bundle_update(current_dir, previous_bundle, err);
            }
        }

        Ok(())
    }

    pub(crate) fn delete_task(&self, id: &str) -> Result<bool, OrbitError> {
        validate_task_id(id)?;
        let _task_lock = self.acquire_task_lock(id)?;
        let Some((_, task_dir)) = self.locate_task(id)? else {
            return Ok(false);
        };
        fs::remove_dir_all(task_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        Ok(true)
    }

    fn rollback_bundle_update(
        &self,
        task_dir: &Path,
        previous_bundle: &TaskBundle,
        original_error: OrbitError,
    ) -> Result<(), OrbitError> {
        match self.write_bundle_at(task_dir, previous_bundle) {
            Ok(()) => Err(original_error),
            Err(rollback_error) => Err(OrbitError::Store(format!(
                "failed to persist task bundle update: {original_error}; rollback failed: {rollback_error}"
            ))),
        }
    }
}

fn cleanup_partial_task_dir(task_dir: &Path) -> Result<(), OrbitError> {
    match fs::remove_dir_all(task_dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use orbit_common::types::{TaskPriority, TaskType};
    use tempfile::tempdir;

    fn create_params(title: &str, external_refs: Vec<ExternalRef>) -> TaskCreateParams {
        TaskCreateParams {
            actor: "test".to_string(),
            parent_id: None,
            title: title.to_string(),
            description: "Fixture task.".to_string(),
            acceptance_criteria: Vec::new(),
            dependencies: Vec::new(),
            plan: String::new(),
            execution_summary: String::new(),
            context_files: Vec::new(),
            workspace_path: Some(".".to_string()),
            repo_root: None,
            created_by: Some("test".to_string()),
            planned_by: None,
            implemented_by: None,
            agent: None,
            model: None,
            status: TaskStatus::Backlog,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Task,
            external_refs,
            source_task_id: None,
            comments: Vec::new(),
        }
    }

    fn assert_invalid_task_id<T: std::fmt::Debug>(result: Result<T, OrbitError>) {
        let err = result.expect_err("invalid task id should be rejected");
        assert!(
            matches!(err, OrbitError::InvalidInput(_)),
            "expected invalid input error, got {err:?}"
        );
    }

    #[test]
    fn public_task_id_entrypoints_reject_invalid_ids_before_locking() {
        let root = tempdir().expect("tempdir");
        let store = TaskFileStore::new(root.path().to_path_buf());
        let invalid_ids = [
            "",
            "   ",
            "../T20260426-1",
            "../../outside",
            "/tmp/T20260426-1",
            r"C:\tmp\T20260426-1",
            r"T20260426-1\outside",
            "T20260426-1/outside",
            "T20260426-1..2",
        ];

        for id in invalid_ids {
            assert_invalid_task_id(store.get_task(id));
            assert_invalid_task_id(store.get_task_artifacts(id));
            assert_invalid_task_id(store.update_task_document(
                id,
                &TaskDocumentUpdateParams {
                    actor: "test".to_string(),
                    ..Default::default()
                },
            ));
            assert_invalid_task_id(store.update_task_history(
                id,
                &TaskHistoryUpdateParams {
                    actor: "test".to_string(),
                    ..Default::default()
                },
            ));
            assert_invalid_task_id(
                store.update_task_reviews(id, &TaskReviewUpdateParams::default()),
            );
            assert_invalid_task_id(
                store.upsert_task_artifacts(id, &TaskArtifactUpdateParams::default()),
            );
            assert_invalid_task_id(store.delete_task(id));
        }

        assert!(
            !store.root.join(".locks").exists(),
            "invalid ids must not create task lock files"
        );
    }

    #[test]
    fn delete_task_rejects_traversal_without_removing_outside_dir_or_creating_lock() {
        let outer = tempdir().expect("tempdir");
        let store_root = outer.path().join("tasks");
        let store = TaskFileStore::new(store_root.clone());
        let outside_dir = outer.path().join("outside");
        let sentinel = outside_dir.join("sentinel.txt");
        fs::create_dir_all(&outside_dir).expect("create outside dir");
        fs::write(&sentinel, "keep").expect("write sentinel");

        assert_invalid_task_id(store.delete_task("../../outside"));

        assert!(outside_dir.is_dir(), "outside dir must not be removed");
        assert!(sentinel.is_file(), "outside sentinel must not be removed");
        assert!(
            !store_root.join(".locks").exists(),
            "invalid delete must not create an outside lock file"
        );
    }

    #[test]
    fn list_and_search_match_external_refs() {
        let root = tempdir().expect("tempdir");
        let store = TaskFileStore::new(root.path().to_path_buf());
        let jira = ExternalRef::parse_key("jira:ENG-1234").expect("jira ref");
        let linear = ExternalRef::parse_key("linear:LIN-567").expect("linear ref");

        let linked = store
            .create_task(create_params("Linked task", vec![jira.clone(), linear]))
            .expect("create linked task");
        let jira_only = store
            .create_task(create_params("Jira only", vec![jira.clone()]))
            .expect("create jira-only task");

        let exact = store
            .list_tasks_filtered(None, None, None, None, Some(&jira), None)
            .expect("filter by exact ref");
        let exact_ids = exact
            .iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(exact_ids, vec![jira_only.id.as_str(), linked.id.as_str()]);

        let linear_and_jira = store
            .list_tasks_filtered(None, None, None, None, Some(&jira), Some("linear"))
            .expect("filter by exact ref and system");
        assert_eq!(linear_and_jira.len(), 1);
        assert_eq!(linear_and_jira[0].id, linked.id);

        let matches = store.search_tasks("eng-1234").expect("search by ref id");
        let match_ids = matches
            .iter()
            .map(|task| task.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(match_ids, vec![jira_only.id.as_str(), linked.id.as_str()]);
    }

    #[test]
    fn schema_two_task_loads_with_empty_external_refs_and_roundtrips_without_field() {
        let root = tempdir().expect("tempdir");
        let store = TaskFileStore::new(root.path().to_path_buf());
        let id = "T20260101-1";
        let task_dir = store.task_dir(TaskStateDir::Backlog, id);
        fs::create_dir_all(&task_dir).expect("create legacy task dir");
        fs::write(
            store.task_doc_path(&task_dir),
            r#"schema_version: 2
id: T20260101-1
priority: medium
title: Legacy task
created_at: 2026-01-01T00:00:00Z
updated_at: 2026-01-01T00:00:00Z
"#,
        )
        .expect("write legacy task yaml");

        let task = store
            .get_task(id)
            .expect("load legacy task")
            .expect("legacy task exists");
        assert!(task.external_refs.is_empty());

        store
            .update_task_document(
                id,
                &TaskDocumentUpdateParams {
                    actor: "test".to_string(),
                    description: Some("Updated.".to_string()),
                    ..Default::default()
                },
            )
            .expect("roundtrip legacy task");

        let yaml = fs::read_to_string(store.task_doc_path(&task_dir)).expect("read task yaml");
        assert!(yaml.contains("schema_version: 4"));
        assert!(!yaml.contains("external_refs"));
    }

    #[test]
    fn legacy_pr_number_loads_as_github_pr_external_ref_and_roundtrips_without_pr_number() {
        let root = tempdir().expect("tempdir");
        let store = TaskFileStore::new(root.path().to_path_buf());
        let id = "T20260101-2";
        let task_dir = store.task_dir(TaskStateDir::Backlog, id);
        fs::create_dir_all(&task_dir).expect("create legacy task dir");
        fs::write(
            store.task_doc_path(&task_dir),
            r#"schema_version: 3
id: T20260101-2
priority: medium
title: Legacy PR task
pr_number: "42"
created_at: 2026-01-01T00:00:00Z
updated_at: 2026-01-01T00:00:00Z
"#,
        )
        .expect("write legacy task yaml");

        let task = store
            .get_task(id)
            .expect("load legacy task")
            .expect("legacy task exists");
        assert_eq!(task.external_refs.len(), 1);
        assert_eq!(task.external_refs[0].system, "github-pr");
        assert_eq!(task.external_refs[0].id, "42");

        store
            .update_task_document(
                id,
                &TaskDocumentUpdateParams {
                    actor: "test".to_string(),
                    description: Some("Updated.".to_string()),
                    ..Default::default()
                },
            )
            .expect("roundtrip legacy task");

        let yaml = fs::read_to_string(store.task_doc_path(&task_dir)).expect("read task yaml");
        assert!(yaml.contains("schema_version: 4"));
        assert!(!yaml.contains("pr_number"));
        assert!(yaml.contains("system: github-pr"));
        assert!(yaml.contains("id: '42'"));
    }

    #[test]
    fn legacy_pr_number_does_not_duplicate_existing_github_pr_ref() {
        let root = tempdir().expect("tempdir");
        let store = TaskFileStore::new(root.path().to_path_buf());
        let id = "T20260101-3";
        let task_dir = store.task_dir(TaskStateDir::Backlog, id);
        fs::create_dir_all(&task_dir).expect("create legacy task dir");
        fs::write(
            store.task_doc_path(&task_dir),
            r#"schema_version: 3
id: T20260101-3
priority: medium
title: Legacy PR task
pr_number: "42"
external_refs:
- system: github-pr
  id: "42"
  url: https://example.com/pull/42
created_at: 2026-01-01T00:00:00Z
updated_at: 2026-01-01T00:00:00Z
"#,
        )
        .expect("write legacy task yaml");

        let task = store
            .get_task(id)
            .expect("load legacy task")
            .expect("legacy task exists");

        assert_eq!(task.external_refs.len(), 1);
        assert_eq!(task.external_refs[0].system, "github-pr");
        assert_eq!(task.external_refs[0].id, "42");
        assert_eq!(
            task.external_refs[0].url.as_deref(),
            Some("https://example.com/pull/42")
        );
    }
}
