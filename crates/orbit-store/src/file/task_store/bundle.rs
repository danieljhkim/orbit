use std::fs;
use std::path::Path;

use orbit_common::types::{
    ActorIdentity, ExternalRef, OrbitError, ReviewThread, Task, TaskStatus,
    normalize_optional_attribution_label, normalize_task_tags,
};

use crate::file::yaml_doc::{read_yaml_with, write_yaml_atomic_with};
use orbit_common::utility::fs::atomic_write_text_volatile as write_atomic;

use super::{
    TaskFileStore,
    constants::TASK_SCHEMA_VERSION,
    doc::{TaskFileDocument, serialize_task_doc_yaml},
    layout::{TaskStateDir, validate_task_id},
};

#[derive(Debug, Clone)]
pub(super) struct TaskBundle {
    pub(super) doc: TaskFileDocument,
    pub(super) plan: String,
    pub(super) execution_summary: String,
}

impl TaskFileStore {
    pub(super) fn write_bundle_for_state(
        &self,
        state: TaskStateDir,
        bundle: &TaskBundle,
    ) -> Result<(), OrbitError> {
        validate_task_id(&bundle.doc.id)?;
        self.write_bundle_at(&self.task_dir(state, &bundle.doc.id), bundle)
    }

    pub(super) fn write_bundle_at(
        &self,
        task_dir: &Path,
        bundle: &TaskBundle,
    ) -> Result<(), OrbitError> {
        let mut bundle = bundle.clone();
        bundle.doc.schema_version = TASK_SCHEMA_VERSION;
        bundle.doc.tags = normalize_task_tags(bundle.doc.tags);
        self.validate_bundle(&bundle, Some(task_dir))?;
        fs::create_dir_all(task_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        fs::create_dir_all(self.artifacts_dir(task_dir))
            .map_err(|e| OrbitError::Io(e.to_string()))?;

        write_yaml_atomic_with(
            &self.task_doc_path(task_dir),
            &bundle.doc,
            serialize_task_doc_yaml,
        )?;
        write_atomic(&self.plan_path(task_dir), &bundle.plan)?;
        write_atomic(
            &self.execution_summary_path(task_dir),
            &bundle.execution_summary,
        )?;
        Ok(())
    }

    pub(super) fn read_bundle_at(&self, task_dir: &Path) -> Result<TaskBundle, OrbitError> {
        let doc_path = self.task_doc_path(task_dir);
        if !doc_path.exists() {
            return Err(bundle_read_error(
                &doc_path,
                "task metadata",
                std::io::Error::new(std::io::ErrorKind::NotFound, "task metadata not found"),
            ));
        }
        let doc = read_yaml_with(&doc_path, |path, err| {
            OrbitError::Store(format!("invalid task file {}: {err}", path.display()))
        })?;
        let bundle = TaskBundle {
            doc,
            plan: read_companion_text(&self.plan_path(task_dir), "task plan")?,
            execution_summary: read_companion_text(
                &self.execution_summary_path(task_dir),
                "task execution summary",
            )?,
        };
        self.validate_bundle(&bundle, Some(task_dir))?;
        Ok(bundle)
    }

    pub(super) fn validate_bundle(
        &self,
        bundle: &TaskBundle,
        task_dir: Option<&Path>,
    ) -> Result<(), OrbitError> {
        if bundle.doc.id.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task id must not be empty".to_string(),
            ));
        }
        validate_task_id(&bundle.doc.id)?;
        if bundle.doc.title.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task title must not be empty".to_string(),
            ));
        }
        if let Some(task_dir) = task_dir {
            let Some(dir_name) = task_dir.file_name().and_then(|name| name.to_str()) else {
                return Err(OrbitError::Store(format!(
                    "invalid task directory path {}",
                    task_dir.display()
                )));
            };
            if dir_name != bundle.doc.id {
                return Err(OrbitError::Store(format!(
                    "task directory {} does not match task id {}",
                    task_dir.display(),
                    bundle.doc.id
                )));
            }
        }
        for external_ref in &bundle.doc.external_refs {
            validate_external_ref(external_ref)?;
        }
        Ok(())
    }
}

fn validate_external_ref(external_ref: &ExternalRef) -> Result<(), OrbitError> {
    ExternalRef::try_new(
        external_ref.system.clone(),
        external_ref.id.clone(),
        external_ref.url.clone(),
    )
    .map(|_| ())
}

pub(super) fn read_companion_text(path: &Path, label: &str) -> Result<String, OrbitError> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(value),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(bundle_read_error(path, label, err)),
    }
}

pub(super) fn bundle_read_error(path: &Path, label: &str, err: std::io::Error) -> OrbitError {
    if err.kind() == std::io::ErrorKind::NotFound {
        OrbitError::Store(format!("missing {label} at {}", path.display()))
    } else {
        OrbitError::Io(err.to_string())
    }
}

pub(super) fn merge_review_threads(existing: &mut Vec<ReviewThread>, incoming: Vec<ReviewThread>) {
    for thread in incoming {
        if let Some(existing_thread) = existing
            .iter_mut()
            .find(|t| t.thread_id == thread.thread_id)
        {
            existing_thread.messages.extend(thread.messages);
            existing_thread.status = thread.status;
            if thread.github_thread_id.is_some() {
                existing_thread.github_thread_id = thread.github_thread_id;
            }
        } else {
            existing.push(thread);
        }
    }
}

pub(super) fn bundle_to_task(state: TaskStateDir, bundle: TaskBundle) -> Task {
    let legacy_identity = if bundle.doc.actor_identity.is_system() && bundle.doc.agent.is_some() {
        ActorIdentity::from_legacy(bundle.doc.agent.as_deref(), bundle.doc.model.as_deref())
    } else {
        bundle.doc.actor_identity
    };
    let (legacy_agent, legacy_model) = legacy_identity.to_legacy();
    let model_hint = bundle.doc.model.as_deref().or(legacy_model.as_deref());
    let created_by = normalize_optional_attribution_label(
        bundle
            .doc
            .created_by
            .as_deref()
            .or(bundle.doc.proposed_by.as_deref())
            .or(legacy_model.as_deref()),
        model_hint,
    );
    let planned_by =
        normalize_optional_attribution_label(bundle.doc.planned_by.as_deref(), model_hint);
    let legacy_implemented_by = if matches!(
        state.to_status(),
        TaskStatus::Review | TaskStatus::Done | TaskStatus::Archived
    ) {
        bundle
            .doc
            .assigned_to
            .clone()
            .or_else(|| legacy_model.clone())
    } else {
        None
    };
    let implemented_by = normalize_optional_attribution_label(
        bundle
            .doc
            .implemented_by
            .as_deref()
            .or(legacy_implemented_by.as_deref()),
        model_hint,
    );

    Task {
        id: bundle.doc.id,
        parent_id: bundle.doc.parent_id,
        title: bundle.doc.title,
        description: bundle.doc.description,
        acceptance_criteria: bundle.doc.acceptance_criteria,
        dependencies: bundle.doc.dependencies,
        tags: normalize_task_tags(bundle.doc.tags),
        plan: bundle.plan,
        execution_summary: bundle.execution_summary,
        context_files: bundle.doc.context_files,
        workspace_path: bundle.doc.workspace_path,
        repo_root: bundle.doc.repo_root,
        created_by,
        planned_by,
        implemented_by,
        agent: bundle.doc.agent.or(legacy_agent),
        model: bundle.doc.model.or(legacy_model),
        status: state.to_status(),
        priority: bundle.doc.priority,
        complexity: bundle.doc.complexity,
        task_type: bundle.doc.task_type,
        pr_status: bundle.doc.pr_status,
        external_refs: bundle.doc.external_refs,
        source_task_id: bundle.doc.source_task_id,
        batch_id: bundle.doc.batch_id,
        comments: bundle.doc.comments,
        history: bundle.doc.history,
        review_threads: bundle.doc.review_threads,
        created_at: bundle.doc.created_at,
        updated_at: bundle.doc.updated_at,
    }
}
