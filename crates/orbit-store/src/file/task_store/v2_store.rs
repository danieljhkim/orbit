use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use orbit_common::types::{
    ArtifactManifestFileV2, ArtifactManifestV2, ExternalRef, NotFoundKind, OrbitError, OrbitId,
    ReviewMessage, ReviewThread, ReviewThreadMessageMetadataV2, ReviewThreadMetadataV2,
    TASK_ARTIFACT_FILES_DIR_NAME, TASK_ARTIFACT_SCHEMA_VERSION, TASK_ARTIFACTS_DIR_NAME, Task,
    TaskArtifact, TaskComment, TaskCommentRowV2, TaskEnvelopeV2, TaskEventRowV2, TaskHistoryEntry,
    TaskPriority, TaskRelation, TaskRelationType, TaskStatus, normalize_task_tags,
    validate_relative_artifact_path,
};
use orbit_common::utility::fs::{atomic_write_bytes, with_exclusive_file_lock};
use sha2::{Digest, Sha256};

use crate::backend::{
    TaskArtifactUpdateParams, TaskCreateParams, TaskDocumentUpdateParams, TaskHistoryUpdateParams,
    TaskReviewUpdateParams,
};
use crate::file::sort::sort_by_created_desc_id_asc;
use crate::file::task_store::v2_bundle::{
    TaskBundleStoreV2, TaskBundleV2, TaskDocumentV2, TaskReviewThreadV2,
};
use crate::sqlite::task_registry::{TaskIndexFilter, TaskRegistryStore};

pub(crate) struct TaskV2Store {
    registry: TaskRegistryStore,
    bundle_store: TaskBundleStoreV2,
    workspace_id: String,
}

impl TaskV2Store {
    pub(crate) fn new(
        registry: TaskRegistryStore,
        workspace_id: String,
        workspace_orbit_dir: PathBuf,
        _workspace_path: Option<String>,
        _repo_root: Option<String>,
    ) -> Self {
        Self {
            bundle_store: TaskBundleStoreV2::new(
                registry.clone(),
                workspace_id.clone(),
                workspace_orbit_dir,
            ),
            registry,
            workspace_id,
        }
    }

    pub(crate) fn create_task(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
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
        let relations = relations_from_create_params(&params)?;

        let now = Utc::now();
        let id = self.registry.allocate_task_id(&self.workspace_id)?;
        let comments = params
            .comments
            .iter()
            .enumerate()
            .map(|(index, comment)| orbit_common::types::TaskCommentRowV2 {
                schema_version: orbit_common::types::TASK_ARTIFACT_SCHEMA_VERSION,
                comment_id: format!("C-{number:04}", number = index + 1),
                at: comment.at,
                by: comment.by.clone(),
                body: comment.message.clone(),
            })
            .collect();
        let bundle = TaskBundleV2 {
            envelope: orbit_common::types::TaskEnvelopeV2 {
                schema_version: orbit_common::types::TASK_ARTIFACT_SCHEMA_VERSION,
                id: id.clone(),
                title: params.title,
                status: params.status,
                task_type: params.task_type,
                priority: params.priority,
                complexity: params.complexity,
                job_run_id: None,
                relations,
                tags: normalize_task_tags(params.tags),
                context_files: params.context_files,
                external_refs: params.external_refs,
                created_by: params.created_by,
                planned_by: params.planned_by,
                implemented_by: params.implemented_by,
                created_at: now,
                updated_at: now,
            },
            description: params.description,
            acceptance: render_acceptance(&params.acceptance_criteria),
            plan: params.plan,
            execution_summary: params.execution_summary,
            events: vec![orbit_common::types::TaskEventRowV2 {
                schema_version: orbit_common::types::TASK_ARTIFACT_SCHEMA_VERSION,
                event_id: "EV-0001".to_string(),
                at: now,
                by: params.actor,
                event_type: "created".to_string(),
                note: None,
                from_status: None,
                to_status: Some(params.status),
            }],
            comments,
            review_threads: Vec::new(),
            artifact_manifest: None,
        };

        self.bundle_store.create_bundle(&bundle)?;
        self.replace_index_best_effort(&bundle.envelope, "task creation");
        self.task_from_bundle(bundle)
    }

    pub(crate) fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        if let Some(tasks) = self.indexed_tasks(TaskIndexFilter {
            status: None,
            priority: None,
            job_run_id: None,
            tags: Vec::new(),
        })? {
            return Ok(tasks);
        }

        let mut tasks = self
            .bundle_store
            .list_bundles()?
            .into_iter()
            .map(|bundle| self.task_from_bundle(bundle))
            .collect::<Result<Vec<_>, _>>()?;
        sort_by_created_desc_id_asc(&mut tasks, |task| &task.created_at, |task| &task.id);
        Ok(tasks)
    }

    pub(crate) fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        job_run_id: Option<&str>,
        external_ref: Option<&ExternalRef>,
        has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        let mut tasks = match self.indexed_tasks(TaskIndexFilter {
            status,
            priority,
            job_run_id: job_run_id.map(ToOwned::to_owned),
            tags: Vec::new(),
        })? {
            Some(tasks) => tasks,
            None => self.list_tasks()?,
        };
        tasks.retain(|task| {
            status.is_none_or(|value| task.status == value)
                && priority.is_none_or(|value| task.priority == value)
                && parent_id.is_none_or(|value| task.parent_id() == Some(value))
                && job_run_id.is_none_or(|value| task.job_run_id.as_deref() == Some(value))
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

    pub(crate) fn list_tasks_by_tags(&self, tags: &[String]) -> Result<Vec<Task>, OrbitError> {
        let required_tags = normalize_task_tags(tags.to_vec());
        if required_tags.is_empty() {
            return self.list_tasks();
        }
        if let Some(tasks) = self.indexed_tasks(TaskIndexFilter {
            status: None,
            priority: None,
            job_run_id: None,
            tags: required_tags.clone(),
        })? {
            return Ok(tasks);
        }
        let mut tasks = self.list_tasks()?;
        tasks.retain(|task| {
            required_tags
                .iter()
                .all(|required| task.tags.iter().any(|tag| tag == required))
        });
        Ok(tasks)
    }

    pub(crate) fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        orbit_common::types::validate_orb_task_id(id)?;
        match self.bundle_store.read_bundle(id) {
            Ok(bundle) => self.task_from_bundle(bundle).map(Some),
            Err(OrbitError::NotFound {
                kind: NotFoundKind::Task,
                ..
            }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub(crate) fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.search_tasks_filtered(query, &[])
    }

    pub(crate) fn search_tasks_filtered(
        &self,
        query: &str,
        tags: &[String],
    ) -> Result<Vec<Task>, OrbitError> {
        let lowered = query.to_lowercase();
        let mut tasks = self.list_tasks_by_tags(tags)?;
        let mut matches = Vec::new();
        for task in tasks.drain(..) {
            if self.task_matches_query(&task, &lowered)? {
                matches.push(task);
            }
        }
        Ok(matches)
    }

    pub(crate) fn delete_task(&self, id: &str) -> Result<bool, OrbitError> {
        orbit_common::types::validate_orb_task_id(id)?;
        let lock_target = self.bundle_store.bundle_path(id)?;
        with_exclusive_file_lock(&lock_target, "task artifact v2 delete", || {
            self.bundle_store.delete_bundle(id)
        })
    }

    pub(crate) fn update_task_document(
        &self,
        id: &str,
        fields: &TaskDocumentUpdateParams,
    ) -> Result<(), OrbitError> {
        orbit_common::types::validate_orb_task_id(id)?;
        if fields.actor.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task actor must not be empty".to_string(),
            ));
        }
        reject_unsupported_document_fields(fields)?;

        self.with_task_lock(id, || {
            let mut bundle = self.read_existing_bundle(id)?;
            let mut envelope_changed = false;
            let mut title_changed = false;

            if let Some(value) = &fields.title {
                if value.trim().is_empty() {
                    return Err(OrbitError::InvalidInput(
                        "task title must not be empty".to_string(),
                    ));
                }
                title_changed = *value != bundle.envelope.title;
                bundle.envelope.title = value.clone();
                envelope_changed = true;
            }
            if let Some(value) = &fields.tags {
                bundle.envelope.tags = normalize_task_tags(value.clone());
                envelope_changed = true;
            }
            if let Some(value) = &fields.context_files {
                bundle.envelope.context_files = value.clone();
                envelope_changed = true;
            }
            if let Some(value) = &fields.created_by {
                bundle.envelope.created_by = value.clone();
                envelope_changed = true;
            }
            if let Some(value) = &fields.planned_by {
                bundle.envelope.planned_by = value.clone();
                envelope_changed = true;
            }
            if let Some(value) = &fields.implemented_by {
                bundle.envelope.implemented_by = value.clone();
                envelope_changed = true;
            }
            if let Some(value) = fields.priority {
                bundle.envelope.priority = value;
                envelope_changed = true;
            }
            if let Some(value) = fields.complexity {
                bundle.envelope.complexity = Some(value);
                envelope_changed = true;
            }
            if let Some(value) = fields.task_type {
                bundle.envelope.task_type = value;
                envelope_changed = true;
            }
            if let Some(value) = &fields.dependencies {
                replace_relations(
                    &mut bundle.envelope.relations,
                    TaskRelationType::BlockedBy,
                    value.iter().cloned().map(|target| TaskRelation {
                        relation_type: TaskRelationType::BlockedBy,
                        target,
                    }),
                );
                envelope_changed = true;
            }
            if let Some(value) = &fields.source_task_id {
                replace_relations(
                    &mut bundle.envelope.relations,
                    TaskRelationType::RegressionFrom,
                    value.iter().cloned().map(|target| TaskRelation {
                        relation_type: TaskRelationType::RegressionFrom,
                        target,
                    }),
                );
                envelope_changed = true;
            }
            if let Some(value) = &fields.job_run_id {
                bundle.envelope.job_run_id = value.clone();
                envelope_changed = true;
            }
            if let Some(value) = &fields.external_refs {
                bundle.envelope.external_refs = value.clone();
                envelope_changed = true;
            }

            if let Some(value) = &fields.description {
                self.bundle_store
                    .rewrite_document(id, TaskDocumentV2::Description, value)?;
            }
            if let Some(value) = &fields.acceptance_criteria {
                self.bundle_store.rewrite_document(
                    id,
                    TaskDocumentV2::Acceptance,
                    &render_acceptance(value),
                )?;
            }
            if let Some(value) = &fields.plan {
                self.bundle_store
                    .rewrite_document(id, TaskDocumentV2::Plan, value)?;
            }
            if let Some(value) = &fields.execution_summary {
                self.bundle_store
                    .rewrite_document(id, TaskDocumentV2::ExecutionSummary, value)?;
            }

            if title_changed {
                let now = Utc::now();
                let event = TaskEventRowV2 {
                    schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                    event_id: next_event_id(&bundle.events),
                    at: now,
                    by: fields.actor.clone(),
                    event_type: "renamed".to_string(),
                    note: None,
                    from_status: None,
                    to_status: None,
                };
                self.bundle_store.append_event(id, &event)?;
                bundle.events.push(event);
            }

            if envelope_changed
                || fields.description.is_some()
                || fields.acceptance_criteria.is_some()
                || fields.plan.is_some()
                || fields.execution_summary.is_some()
            {
                bundle.envelope.updated_at = Utc::now();
                self.bundle_store.rewrite_envelope(id, &bundle.envelope)?;
                self.replace_index_best_effort(&bundle.envelope, "task document update");
            }
            Ok(())
        })
    }

    pub(crate) fn update_task_history(
        &self,
        id: &str,
        fields: &TaskHistoryUpdateParams,
    ) -> Result<(), OrbitError> {
        orbit_common::types::validate_orb_task_id(id)?;
        if fields.actor.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task actor must not be empty".to_string(),
            ));
        }

        self.with_task_lock(id, || {
            let mut bundle = self.read_existing_bundle(id)?;
            let now = Utc::now();
            let current_status = bundle.envelope.status;
            let target_status = fields.status.unwrap_or(current_status);
            let status_transition =
                (target_status != current_status).then_some((current_status, target_status));
            let mut next_event = next_sequence(&bundle.events, "EV-");
            let mut next_comment = next_sequence(&bundle.comments, "C-");

            for entry in &fields.append_history {
                let event = TaskEventRowV2 {
                    schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                    event_id: format!("EV-{next_event:04}"),
                    at: entry.at,
                    by: entry.by.clone(),
                    event_type: entry.event.clone(),
                    note: entry.note.clone(),
                    from_status: entry.from_status,
                    to_status: entry.to_status,
                };
                self.bundle_store.append_event(id, &event)?;
                bundle.events.push(event);
                next_event += 1;
            }

            for comment in &fields.append_comments {
                let row = TaskCommentRowV2 {
                    schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                    comment_id: format!("C-{next_comment:04}"),
                    at: comment.at,
                    by: comment.by.clone(),
                    body: comment.message.clone(),
                };
                self.bundle_store.append_comment(id, &row)?;
                bundle.comments.push(row);
                next_comment += 1;
            }

            let event_type = fields
                .status_event
                .clone()
                .or_else(|| status_transition.map(|_| "status_changed".to_string()));
            if let Some(event_type) = event_type {
                let event = TaskEventRowV2 {
                    schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                    event_id: format!("EV-{next_event:04}"),
                    at: now,
                    by: fields.actor.clone(),
                    event_type,
                    note: fields.status_note.clone(),
                    from_status: status_transition.map(|(from, _)| from),
                    to_status: status_transition.map(|(_, to)| to),
                };
                self.bundle_store.append_event(id, &event)?;
                bundle.events.push(event);
            }

            if !fields.append_history.is_empty()
                || !fields.append_comments.is_empty()
                || fields.status.is_some()
                || fields.status_event.is_some()
                || fields.status_note.is_some()
            {
                bundle.envelope.status = target_status;
                bundle.envelope.updated_at = now;
                self.bundle_store.rewrite_envelope(id, &bundle.envelope)?;
                self.replace_index_best_effort(&bundle.envelope, "task history update");
            }
            Ok(())
        })
    }

    pub(crate) fn update_task_reviews(
        &self,
        id: &str,
        fields: &TaskReviewUpdateParams,
    ) -> Result<(), OrbitError> {
        orbit_common::types::validate_orb_task_id(id)?;
        self.with_task_lock(id, || {
            let mut bundle = self.read_existing_bundle(id)?;
            let mut threads = std::mem::take(&mut bundle.review_threads)
                .into_iter()
                .map(review_thread_from_v2)
                .collect::<Vec<_>>();

            if let Some(replacement) = &fields.replace_review_threads {
                threads = replacement.clone();
            } else if !fields.append_review_threads.is_empty() {
                merge_review_threads_v2(&mut threads, fields.append_review_threads.clone());
            }

            let threads = threads
                .into_iter()
                .map(review_thread_to_v2)
                .collect::<Result<Vec<_>, _>>()?;
            self.bundle_store.rewrite_review_threads(id, &threads)?;
            bundle.envelope.updated_at = Utc::now();
            self.bundle_store.rewrite_envelope(id, &bundle.envelope)?;
            self.replace_index_best_effort(&bundle.envelope, "task review update");
            Ok(())
        })
    }

    pub(crate) fn get_task_artifacts(
        &self,
        id: &str,
    ) -> Result<Option<Vec<TaskArtifact>>, OrbitError> {
        orbit_common::types::validate_orb_task_id(id)?;
        let bundle = match self.bundle_store.read_bundle(id) {
            Ok(bundle) => bundle,
            Err(OrbitError::NotFound {
                kind: NotFoundKind::Task,
                ..
            }) => return Ok(None),
            Err(err) => return Err(err),
        };
        let Some(manifest) = bundle.artifact_manifest else {
            return Ok(Some(Vec::new()));
        };
        let bundle_dir = self.bundle_store.bundle_path(id)?;
        let mut artifacts = Vec::new();
        for file in manifest.files {
            let artifact_file = bundle_dir.join(TASK_ARTIFACTS_DIR_NAME).join(&file.blob);
            let content =
                fs::read(&artifact_file).map_err(|err| OrbitError::Io(err.to_string()))?;
            artifacts.push(TaskArtifact {
                path: file.path,
                media_type: file.media_type,
                content,
            });
        }
        artifacts.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Some(artifacts))
    }

    pub(crate) fn upsert_task_artifacts(
        &self,
        id: &str,
        fields: &TaskArtifactUpdateParams,
    ) -> Result<(), OrbitError> {
        orbit_common::types::validate_orb_task_id(id)?;
        if fields.upsert_artifacts.is_empty() {
            return Ok(());
        }
        if fields.actor.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task actor must not be empty".to_string(),
            ));
        }

        self.with_task_lock(id, || {
            let mut bundle = self.read_existing_bundle(id)?;
            let bundle_dir = self.bundle_store.bundle_path(id)?;
            let files_dir = bundle_dir
                .join(TASK_ARTIFACTS_DIR_NAME)
                .join(TASK_ARTIFACT_FILES_DIR_NAME);
            fs::create_dir_all(&files_dir).map_err(|err| OrbitError::Io(err.to_string()))?;

            let mut by_path = bundle
                .artifact_manifest
                .take()
                .unwrap_or(ArtifactManifestV2 {
                    schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                    files: Vec::new(),
                })
                .files
                .into_iter()
                .map(|file| (file.path.clone(), file))
                .collect::<BTreeMap<_, _>>();

            let now = Utc::now();
            for artifact in &fields.upsert_artifacts {
                let path = normalize_v2_artifact_path(&artifact.path)?;
                let blob = format!("{TASK_ARTIFACT_FILES_DIR_NAME}/{path}");
                let destination = files_dir.join(&path);
                atomic_write_bytes(&destination, &artifact.content)
                    .map_err(|err| OrbitError::Io(err.to_string()))?;
                by_path.insert(
                    path.clone(),
                    ArtifactManifestFileV2 {
                        path: path.clone(),
                        blob,
                        sha256: format!("{:x}", Sha256::digest(&artifact.content)),
                        media_type: artifact.media_type.clone(),
                        size_bytes: artifact.content.len() as u64,
                        created_by: fields.actor.clone(),
                        created_at: now,
                    },
                );
            }

            let manifest = ArtifactManifestV2 {
                schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                files: by_path.into_values().collect(),
            };
            self.bundle_store.rewrite_artifact_manifest(id, &manifest)?;
            bundle.envelope.updated_at = now;
            self.bundle_store.rewrite_envelope(id, &bundle.envelope)?;
            self.replace_index_best_effort(&bundle.envelope, "task artifact update");
            Ok(())
        })
    }

    fn indexed_tasks(&self, filter: TaskIndexFilter) -> Result<Option<Vec<Task>>, OrbitError> {
        if !self.index_is_usable()? {
            return Ok(None);
        }
        let ids = self
            .registry
            .indexed_task_ids_filtered(&self.workspace_id, &filter)?;
        self.tasks_from_ids(ids).map(Some)
    }

    fn index_is_usable(&self) -> Result<bool, OrbitError> {
        let registered = self.registry.tasks_for_workspace(&self.workspace_id)?;
        let indexed = self
            .registry
            .indexed_task_versions_for_workspace(&self.workspace_id)?;
        if registered.len() != indexed.len() {
            return self.rebuild_index_best_effort("index count mismatch");
        }

        for binding in registered {
            let Some(indexed_updated_at) = indexed.get(&binding.task_id) else {
                return self.rebuild_index_best_effort("missing index row");
            };
            let bundle = self.read_existing_bundle(&binding.task_id)?;
            if bundle.envelope.updated_at.to_rfc3339() != *indexed_updated_at {
                return self.rebuild_index_best_effort("stale index row");
            }
        }
        Ok(true)
    }

    fn rebuild_index_best_effort(&self, reason: &str) -> Result<bool, OrbitError> {
        let bundles = self.bundle_store.list_bundles()?;
        let envelopes = bundles
            .into_iter()
            .map(|bundle| bundle.envelope)
            .collect::<Vec<_>>();
        match self
            .registry
            .replace_workspace_task_indexes(&self.workspace_id, &envelopes)
        {
            Ok(()) => Ok(true),
            Err(err) => {
                orbit_common::tracing::warn!(
                    target: "orbit.store.task_v2",
                    workspace_id = %self.workspace_id,
                    reason,
                    error = %err,
                    "generated task index rebuild failed; falling back to bundle scan",
                );
                Ok(false)
            }
        }
    }

    fn tasks_from_ids(&self, ids: Vec<String>) -> Result<Vec<Task>, OrbitError> {
        ids.into_iter()
            .map(|id| {
                let bundle = self.read_existing_bundle(&id)?;
                self.task_from_bundle(bundle)
            })
            .collect()
    }

    fn task_matches_query(&self, task: &Task, lowered: &str) -> Result<bool, OrbitError> {
        if task_in_memory_fields_match_query(task, lowered) {
            return Ok(true);
        }

        if self.task_sidecars_match_query(&task.id, lowered)? {
            return Ok(true);
        }

        // Phase 5 bridge: artifact search reads text artifact files on demand until
        // generated full-text indexes carry artifact paths, content, and snippets.
        self.task_artifacts_match_query(&task.id, lowered)
    }

    fn task_sidecars_match_query(&self, id: &str, lowered: &str) -> Result<bool, OrbitError> {
        let Some(comments) = self.get_task_comments(id)? else {
            return Ok(false);
        };
        if comments
            .iter()
            .any(|comment| comment.message.to_lowercase().contains(lowered))
        {
            return Ok(true);
        }
        let Some(review_threads) = self.get_task_review_threads(id)? else {
            return Ok(false);
        };
        Ok(review_threads.iter().any(|thread| {
            thread.messages.iter().any(|message| {
                message.body.to_lowercase().contains(lowered)
                    || message.by.to_lowercase().contains(lowered)
            }) || thread
                .path
                .as_deref()
                .is_some_and(|path| path.to_lowercase().contains(lowered))
        }))
    }

    fn task_artifacts_match_query(&self, id: &str, lowered: &str) -> Result<bool, OrbitError> {
        let Some(artifacts) = self.get_task_artifacts(id)? else {
            // A task may be deleted after the indexed/listed candidate set is built.
            return Ok(false);
        };
        Ok(artifacts.iter().any(|artifact| {
            artifact.path.to_lowercase().contains(lowered)
                || (is_text_artifact_media_type(&artifact.media_type)
                    && artifact
                        .text_content()
                        .is_some_and(|content| content.to_lowercase().contains(lowered)))
        }))
    }

    fn replace_index_best_effort(&self, envelope: &TaskEnvelopeV2, operation: &str) {
        if let Err(err) = self
            .registry
            .replace_task_index(&self.workspace_id, envelope)
        {
            orbit_common::tracing::warn!(
                target: "orbit.store.task_v2",
                task_id = %envelope.id,
                workspace_id = %self.workspace_id,
                operation,
                error = %err,
                "task bundle was updated but generated task index update failed",
            );
        }
    }

    fn task_from_bundle(&self, bundle: TaskBundleV2) -> Result<Task, OrbitError> {
        let status = bundle.envelope.status;
        Ok(Task {
            id: bundle.envelope.id,
            title: bundle.envelope.title,
            description: bundle.description,
            acceptance_criteria: parse_acceptance(&bundle.acceptance),
            tags: normalize_task_tags(bundle.envelope.tags),
            plan: bundle.plan,
            execution_summary: bundle.execution_summary,
            context_files: bundle.envelope.context_files,
            created_by: bundle.envelope.created_by,
            planned_by: bundle.envelope.planned_by,
            implemented_by: bundle.envelope.implemented_by,
            status,
            priority: bundle.envelope.priority,
            complexity: bundle.envelope.complexity,
            task_type: bundle.envelope.task_type,
            pr_status: None,
            external_refs: bundle.envelope.external_refs,
            relations: bundle.envelope.relations,
            job_run_id: bundle.envelope.job_run_id,
            created_at: bundle.envelope.created_at,
            updated_at: bundle.envelope.updated_at,
        })
    }

    pub(crate) fn get_task_comments(
        &self,
        id: &str,
    ) -> Result<Option<Vec<TaskComment>>, OrbitError> {
        orbit_common::types::validate_orb_task_id(id)?;
        match self.bundle_store.read_bundle(id) {
            Ok(bundle) => Ok(Some(
                bundle
                    .comments
                    .into_iter()
                    .map(|comment| TaskComment {
                        at: comment.at,
                        by: comment.by,
                        message: comment.body,
                    })
                    .collect(),
            )),
            Err(OrbitError::NotFound {
                kind: NotFoundKind::Task,
                ..
            }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub(crate) fn get_task_history(
        &self,
        id: &str,
    ) -> Result<Option<Vec<TaskHistoryEntry>>, OrbitError> {
        orbit_common::types::validate_orb_task_id(id)?;
        match self.bundle_store.read_bundle(id) {
            Ok(bundle) => Ok(Some(
                bundle
                    .events
                    .into_iter()
                    .map(|event| TaskHistoryEntry {
                        at: event.at,
                        by: event.by,
                        event: event.event_type,
                        note: event.note,
                        from_status: event.from_status,
                        to_status: event.to_status,
                    })
                    .collect(),
            )),
            Err(OrbitError::NotFound {
                kind: NotFoundKind::Task,
                ..
            }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub(crate) fn get_task_review_threads(
        &self,
        id: &str,
    ) -> Result<Option<Vec<ReviewThread>>, OrbitError> {
        orbit_common::types::validate_orb_task_id(id)?;
        match self.bundle_store.read_bundle(id) {
            Ok(bundle) => Ok(Some(
                bundle
                    .review_threads
                    .into_iter()
                    .map(review_thread_from_v2)
                    .collect(),
            )),
            Err(OrbitError::NotFound {
                kind: NotFoundKind::Task,
                ..
            }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn read_existing_bundle(&self, id: &str) -> Result<TaskBundleV2, OrbitError> {
        self.bundle_store.read_bundle(id).map_err(|err| match err {
            OrbitError::NotFound {
                kind: NotFoundKind::Task,
                ..
            } => OrbitError::not_found(NotFoundKind::Task, id.to_string()),
            other => other,
        })
    }

    fn with_task_lock<T, F>(&self, id: &str, op: F) -> Result<T, OrbitError>
    where
        F: FnOnce() -> Result<T, OrbitError>,
    {
        let lock_target = self.bundle_store.bundle_path(id)?.join("task.yaml");
        with_exclusive_file_lock(&lock_target, "task artifact v2", op)
    }
}

fn task_in_memory_fields_match_query(task: &Task, lowered: &str) -> bool {
    task.title.to_lowercase().contains(lowered)
        || task.description.to_lowercase().contains(lowered)
        || task.plan.to_lowercase().contains(lowered)
        || task.execution_summary.to_lowercase().contains(lowered)
        || task
            .acceptance_criteria
            .iter()
            .any(|criterion| criterion.to_lowercase().contains(lowered))
        || task.external_refs.iter().any(|external_ref| {
            external_ref.system.to_lowercase().contains(lowered)
                || external_ref.id.to_lowercase().contains(lowered)
        })
}

fn is_text_artifact_media_type(media_type: &str) -> bool {
    let base = media_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    base.starts_with("text/")
        || matches!(
            base.as_str(),
            "application/json"
                | "application/javascript"
                | "application/toml"
                | "application/x-toml"
                | "application/x-yaml"
                | "application/xml"
                | "application/yaml"
        )
        || base.ends_with("+json")
        || base.ends_with("+xml")
}

pub(crate) fn unsupported_v2_operation(operation: &str) -> OrbitError {
    OrbitError::Store(format!(
        "task artifact v2 operation '{operation}' is not supported yet"
    ))
}

fn reject_unsupported_document_fields(fields: &TaskDocumentUpdateParams) -> Result<(), OrbitError> {
    if fields.pr_status.as_ref().is_some_and(Option::is_some) {
        return Err(unsupported_v2_operation("update_task_document.pr_status"));
    }
    Ok(())
}

fn relations_from_create_params(
    params: &TaskCreateParams,
) -> Result<Vec<TaskRelation>, OrbitError> {
    let mut relations = Vec::new();
    if let Some(parent_id) = &params.parent_id {
        relations.push(TaskRelation {
            relation_type: TaskRelationType::ChildOf,
            target: parent_id.clone(),
        });
    }
    for dependency in &params.dependencies {
        relations.push(TaskRelation {
            relation_type: TaskRelationType::BlockedBy,
            target: dependency.clone(),
        });
    }
    if let Some(source_task_id) = &params.source_task_id {
        relations.push(TaskRelation {
            relation_type: TaskRelationType::RegressionFrom,
            target: source_task_id.clone(),
        });
    }
    Ok(relations)
}

fn replace_relations(
    relations: &mut Vec<TaskRelation>,
    relation_type: TaskRelationType,
    replacements: impl IntoIterator<Item = TaskRelation>,
) {
    relations.retain(|relation| relation.relation_type != relation_type);
    relations.extend(replacements);
}

fn review_thread_from_v2(thread: TaskReviewThreadV2) -> ReviewThread {
    let bodies = review_message_bodies(&thread.body);
    let mut fallback_body = Some(thread.body);
    ReviewThread {
        thread_id: thread.metadata.thread_id,
        path: thread.metadata.path,
        line: thread.metadata.line,
        status: thread.metadata.status,
        messages: thread
            .metadata
            .messages
            .into_iter()
            .map(|message| ReviewMessage {
                body: bodies
                    .get(&message.message_id)
                    .cloned()
                    .unwrap_or_else(|| fallback_body.take().unwrap_or_default()),
                message_id: message.message_id,
                at: message.at,
                by: message.by,
                github_comment_id: message.github_comment_id,
            })
            .collect(),
        github_thread_id: thread.metadata.github_thread_id,
    }
}

fn review_thread_to_v2(thread: ReviewThread) -> Result<TaskReviewThreadV2, OrbitError> {
    let now = Utc::now();
    let created_at = thread
        .messages
        .first()
        .map(|message| message.at)
        .unwrap_or(now);
    let updated_at = thread
        .messages
        .last()
        .map(|message| message.at)
        .unwrap_or(now);
    let body = render_review_thread_body(&thread.messages);
    let metadata = ReviewThreadMetadataV2 {
        schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
        thread_id: thread.thread_id,
        status: thread.status,
        path: thread
            .path
            .as_deref()
            .map(normalize_v2_artifact_path)
            .transpose()?,
        line: thread.line,
        github_thread_id: thread.github_thread_id,
        messages: thread
            .messages
            .into_iter()
            .map(|message| ReviewThreadMessageMetadataV2 {
                message_id: message.message_id,
                at: message.at,
                by: message.by,
                github_comment_id: message.github_comment_id,
            })
            .collect(),
        created_at,
        updated_at,
    };
    metadata.validate()?;
    Ok(TaskReviewThreadV2 { metadata, body })
}

fn merge_review_threads_v2(existing: &mut Vec<ReviewThread>, incoming: Vec<ReviewThread>) {
    for thread in incoming {
        if let Some(existing_thread) = existing
            .iter_mut()
            .find(|candidate| candidate.thread_id == thread.thread_id)
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

fn render_review_thread_body(messages: &[ReviewMessage]) -> String {
    let mut out = String::new();
    for message in messages {
        out.push_str("<!-- orbit-review-message:");
        out.push_str(&message.message_id);
        out.push_str(" -->\n");
        for line in message.body.trim_end().lines() {
            if review_message_anchor_id(line).is_some() {
                out.push('\\');
            }
            out.push_str(line);
            out.push('\n');
        }
        out.push_str("\n\n");
    }
    out
}

fn review_message_bodies(body: &str) -> BTreeMap<String, String> {
    let mut bodies = BTreeMap::new();
    let mut current_id: Option<String> = None;
    let mut current_body = String::new();
    for line in body.lines() {
        if let Some(message_id) = review_message_anchor_id(line) {
            if let Some(id) = current_id.replace(message_id) {
                bodies.insert(id, current_body.trim_end().to_string());
                current_body.clear();
            }
            continue;
        }
        if current_id.is_some() {
            if let Some(escaped) = line.strip_prefix('\\')
                && review_message_anchor_id(escaped).is_some()
            {
                current_body.push_str(escaped);
            } else {
                current_body.push_str(line);
            }
            current_body.push('\n');
        }
    }
    if let Some(id) = current_id {
        bodies.insert(id, current_body.trim_end().to_string());
    }
    bodies
}

fn review_message_anchor_id(line: &str) -> Option<String> {
    line.trim()
        .strip_prefix("<!-- orbit-review-message:")
        .and_then(|value| value.strip_suffix(" -->"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn next_event_id(events: &[TaskEventRowV2]) -> String {
    format!("EV-{:04}", next_sequence(events, "EV-"))
}

trait SequencedRow {
    fn row_id(&self) -> &str;
}

impl SequencedRow for TaskEventRowV2 {
    fn row_id(&self) -> &str {
        &self.event_id
    }
}

impl SequencedRow for TaskCommentRowV2 {
    fn row_id(&self) -> &str {
        &self.comment_id
    }
}

fn next_sequence<T: SequencedRow>(rows: &[T], prefix: &str) -> usize {
    rows.iter()
        .filter_map(|row| row.row_id().strip_prefix(prefix))
        .filter_map(|suffix| suffix.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

fn normalize_v2_artifact_path(raw: &str) -> Result<String, OrbitError> {
    let mut trimmed = raw.trim();
    while let Some(rest) = trimmed.strip_prefix("./") {
        trimmed = rest;
    }
    validate_relative_artifact_path(trimmed)?;
    let mut parts = Vec::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_str().ok_or_else(|| {
                    OrbitError::InvalidInput(format!(
                        "artifact path '{trimmed}' must be valid UTF-8"
                    ))
                })?;
                parts.push(part.to_string());
            }
            _ => {
                return Err(OrbitError::InvalidInput(format!(
                    "artifact path '{trimmed}' must be canonical"
                )));
            }
        }
    }
    Ok(parts.join("/"))
}

fn render_acceptance(criteria: &[String]) -> String {
    if criteria.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for criterion in criteria {
        out.push_str("- [ ] ");
        out.push_str(criterion.trim());
        out.push('\n');
    }
    out
}

fn parse_acceptance(content: &str) -> Vec<OrbitId> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            line.strip_prefix("- [ ] ")
                .or_else(|| line.strip_prefix("- [x] "))
                .or_else(|| line.strip_prefix("- [X] "))
                .or_else(|| line.strip_prefix("* [ ] "))
                .or_else(|| line.strip_prefix("* [x] "))
                .or_else(|| line.strip_prefix("* [X] "))
                .or_else(|| line.strip_prefix("- "))
                .or_else(|| line.strip_prefix("* "))
                .unwrap_or(line)
                .to_string()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use orbit_common::types::{
        ExternalRef, ReviewMessage, ReviewThread, ReviewThreadStatus, TaskArtifact, TaskComment,
        TaskHistoryEntry, TaskPriority, TaskStatus, TaskType,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::backend::{
        TaskArtifactUpdateParams, TaskCreateParams, TaskDocumentUpdateParams,
        TaskHistoryUpdateParams, TaskReviewUpdateParams,
    };
    use crate::sqlite::task_registry::{
        BindWorkspaceParams, TaskRegistryStore, task_registry_path,
    };

    fn store(temp: &TempDir) -> TaskV2Store {
        let registry =
            TaskRegistryStore::open(&task_registry_path(temp.path())).expect("open registry");
        let repo_dir = temp.path().join("repo");
        let orbit_dir = repo_dir.join(".orbit");
        std::fs::create_dir_all(&orbit_dir).expect("create orbit dir");
        let binding = registry
            .bind_workspace(BindWorkspaceParams {
                workspace_id: Some("orbit-test-123456".to_string()),
                slug: "Orbit Test".to_string(),
                repo_root: repo_dir.clone(),
                workspace_path: repo_dir.clone(),
                orbit_dir: orbit_dir.clone(),
                repo_fingerprint: None,
            })
            .expect("bind workspace");
        TaskV2Store::new(
            registry,
            binding.workspace_id,
            orbit_dir,
            Some(repo_dir.to_string_lossy().into_owned()),
            Some(repo_dir.to_string_lossy().into_owned()),
        )
    }

    fn create_params(title: &str, status: TaskStatus) -> TaskCreateParams {
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
        TaskCreateParams {
            actor: "codex:gpt-5.5".to_string(),
            parent_id: None,
            title: title.to_string(),
            description: "Detailed task description".to_string(),
            acceptance_criteria: vec![
                "First criterion".to_string(),
                "Second criterion".to_string(),
            ],
            dependencies: Vec::new(),
            tags: vec!["task-artifacts".to_string(), "v2".to_string()],
            plan: "1. Do the work".to_string(),
            execution_summary: String::new(),
            context_files: vec!["docs/design/task-artifacts/_plan.md".to_string()],
            workspace_path: None,
            repo_root: None,
            created_by: Some("codex:gpt-5.5".to_string()),
            planned_by: None,
            implemented_by: None,
            status,
            priority: TaskPriority::High,
            complexity: None,
            task_type: TaskType::Feature,
            external_refs: vec![
                ExternalRef::try_new("linear".to_string(), "ENG-123".to_string(), None).unwrap(),
            ],
            source_task_id: None,
            comments: vec![TaskComment {
                at: now,
                by: "daniel".to_string(),
                message: "Please build this.".to_string(),
            }],
        }
    }

    #[test]
    fn create_get_and_list_task_round_trip_through_v2_bundle() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);

        let created = store
            .create_task(create_params("Build task v2", TaskStatus::Backlog))
            .expect("create task");

        assert_eq!(created.id, "ORB-00000");
        assert_eq!(
            created.acceptance_criteria,
            vec!["First criterion", "Second criterion"]
        );
        assert_eq!(
            store
                .get_task_comments("ORB-00000")
                .expect("get comments")
                .expect("task exists")
                .len(),
            1
        );
        assert_eq!(
            store
                .get_task_history("ORB-00000")
                .expect("get history")
                .expect("task exists")
                .len(),
            1
        );

        let fetched = store
            .get_task("ORB-00000")
            .expect("get task")
            .expect("task exists");
        assert_eq!(fetched.title, created.title);

        let listed = store.list_tasks().expect("list tasks");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "ORB-00000");
    }

    #[test]
    fn create_task_allocates_globally_monotonic_orb_ids() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);

        let first = store
            .create_task(create_params("First", TaskStatus::Backlog))
            .expect("create first");
        let second = store
            .create_task(create_params("Second", TaskStatus::Backlog))
            .expect("create second");

        assert_eq!(first.id, "ORB-00000");
        assert_eq!(second.id, "ORB-00001");
    }

    #[test]
    fn delete_task_removes_v2_bundle_and_index_rows() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        store
            .create_task(create_params("Delete me", TaskStatus::Rejected))
            .expect("create task");

        assert!(store.delete_task("ORB-00000").expect("delete task"));
        assert_eq!(store.get_task("ORB-00000").expect("get deleted"), None);
        assert!(store.list_tasks().expect("list tasks").is_empty());
        assert_eq!(
            store
                .registry
                .indexed_task_count_for_workspace(&store.workspace_id)
                .expect("index count"),
            0
        );
        assert!(!store.delete_task("ORB-00000").expect("delete missing"));
    }

    #[test]
    fn create_task_persists_lineage_relations() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        let parent = store
            .create_task(create_params("Parent", TaskStatus::Backlog))
            .expect("create parent");
        let dependency = store
            .create_task(create_params("Dependency", TaskStatus::Backlog))
            .expect("create dependency");
        let source = store
            .create_task(create_params("Source", TaskStatus::Done))
            .expect("create source");
        let mut params = create_params("Child", TaskStatus::Backlog);
        params.parent_id = Some(parent.id.clone());
        params.dependencies = vec![dependency.id.clone()];
        params.source_task_id = Some(source.id.clone());

        let child = store.create_task(params).expect("create related task");
        assert_eq!(child.parent_id(), Some(parent.id.as_str()));
        assert_eq!(child.dependencies(), vec![dependency.id.clone()]);
        assert_eq!(child.source_task_id(), Some(source.id.as_str()));

        let envelope = store
            .bundle_store
            .read_bundle(&child.id)
            .expect("read related bundle")
            .envelope;
        assert!(envelope.relations.iter().any(|relation| {
            relation.relation_type == TaskRelationType::ChildOf && relation.target == parent.id
        }));
        assert!(envelope.relations.iter().any(|relation| {
            relation.relation_type == TaskRelationType::BlockedBy
                && relation.target == dependency.id
        }));
        assert!(envelope.relations.iter().any(|relation| {
            relation.relation_type == TaskRelationType::RegressionFrom
                && relation.target == source.id
        }));
    }

    #[test]
    fn filters_and_searches_v2_tasks() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        store
            .create_task(create_params("Backlog task", TaskStatus::Backlog))
            .expect("create backlog");
        let mut review = create_params("Review task", TaskStatus::Review);
        review.tags = vec!["review".to_string()];
        review.description = "Contains a rare search phrase".to_string();
        store.create_task(review).expect("create review");

        assert_eq!(
            store
                .list_tasks_filtered(Some(TaskStatus::Review), None, None, None, None, None,)
                .expect("filtered")
                .into_iter()
                .map(|task| task.title)
                .collect::<Vec<_>>(),
            vec!["Review task"]
        );
        assert_eq!(
            store
                .list_tasks_by_tags(&["task-artifacts".to_string()])
                .expect("tagged")
                .len(),
            1
        );
        assert_eq!(
            store
                .search_tasks("rare search")
                .expect("search")
                .into_iter()
                .map(|task| task.title)
                .collect::<Vec<_>>(),
            vec!["Review task"]
        );
        assert_eq!(
            store
                .search_tasks_filtered("task", &["review".to_string()])
                .expect("search filtered")
                .into_iter()
                .map(|task| task.title)
                .collect::<Vec<_>>(),
            vec!["Review task"]
        );
    }

    #[test]
    fn search_tasks_matches_review_threads_and_text_artifacts() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        store
            .create_task(create_params("Searchable", TaskStatus::Backlog))
            .expect("create task");
        let at = Utc.with_ymd_and_hms(2026, 5, 11, 14, 0, 0).unwrap();
        store
            .update_task_reviews(
                "ORB-00000",
                &TaskReviewUpdateParams {
                    append_review_threads: vec![ReviewThread {
                        thread_id: "rt-search".to_string(),
                        path: Some("src/lib.rs".to_string()),
                        line: Some(7),
                        status: ReviewThreadStatus::Open,
                        messages: vec![ReviewMessage {
                            message_id: "rm-search".to_string(),
                            at,
                            by: "reviewer".to_string(),
                            body: "needle-review-body".to_string(),
                            github_comment_id: None,
                        }],
                        github_thread_id: None,
                    }],
                    replace_review_threads: None,
                },
            )
            .expect("add review thread");
        store
            .upsert_task_artifacts(
                "ORB-00000",
                &TaskArtifactUpdateParams {
                    actor: "codex:gpt-5.5".to_string(),
                    upsert_artifacts: vec![TaskArtifact::from_text(
                        "reports/search.md",
                        "needle-artifact-body\n",
                    )],
                },
            )
            .expect("upsert artifact");

        for query in [
            "needle-review-body",
            "needle-artifact-body",
            "reports/search.md",
            "src/lib.rs",
            "reviewer",
            "linear",
        ] {
            assert_eq!(
                store
                    .search_tasks(query)
                    .expect("search")
                    .into_iter()
                    .map(|task| task.id)
                    .collect::<Vec<_>>(),
                vec!["ORB-00000"],
                "query {query} should match v2 non-envelope content"
            );
        }
        assert!(
            store
                .search_tasks("definitely-missing-query")
                .expect("search no match")
                .is_empty()
        );
    }

    #[test]
    fn search_tasks_skips_binary_artifacts_without_poisoning_results() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        store
            .create_task(create_params("Binary artifact task", TaskStatus::Backlog))
            .expect("create binary task");
        store
            .create_task(create_params("Text artifact task", TaskStatus::Backlog))
            .expect("create text task");

        let at = Utc.with_ymd_and_hms(2026, 5, 11, 15, 0, 0).unwrap();
        let binary = vec![0xff, 0xfe, 0xfd, 0x00];
        let binary_dir = store
            .bundle_store
            .bundle_path("ORB-00000")
            .expect("binary bundle path")
            .join(TASK_ARTIFACTS_DIR_NAME)
            .join(TASK_ARTIFACT_FILES_DIR_NAME);
        std::fs::create_dir_all(&binary_dir).expect("create binary artifact dir");
        std::fs::write(binary_dir.join("payload.bin"), &binary).expect("write binary artifact");
        store
            .bundle_store
            .rewrite_artifact_manifest(
                "ORB-00000",
                &ArtifactManifestV2 {
                    schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                    files: vec![ArtifactManifestFileV2 {
                        path: "payload.bin".to_string(),
                        blob: "files/payload.bin".to_string(),
                        sha256: format!("{:x}", Sha256::digest(&binary)),
                        media_type: "application/octet-stream".to_string(),
                        size_bytes: binary.len() as u64,
                        created_by: "codex:gpt-5.5".to_string(),
                        created_at: at,
                    }],
                },
            )
            .expect("write binary manifest");

        store
            .upsert_task_artifacts(
                "ORB-00001",
                &TaskArtifactUpdateParams {
                    actor: "codex:gpt-5.5".to_string(),
                    upsert_artifacts: vec![TaskArtifact::from_text(
                        "reports/text.txt",
                        "needle-safe-text\n",
                    )],
                },
            )
            .expect("upsert text artifact");

        assert_eq!(
            store
                .search_tasks("needle-safe-text")
                .expect("search skips binary")
                .into_iter()
                .map(|task| task.id)
                .collect::<Vec<_>>(),
            vec!["ORB-00001"]
        );
    }

    #[test]
    fn list_tasks_filtered_supports_lineage_and_job_run_filters() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        let parent = store
            .create_task(create_params("Parent", TaskStatus::Backlog))
            .expect("create parent");
        let mut child = create_params("Child", TaskStatus::Backlog);
        child.parent_id = Some(parent.id.clone());
        let child = store.create_task(child).expect("create child");
        store
            .update_task_document(
                &child.id,
                &TaskDocumentUpdateParams {
                    actor: "codex:gpt-5.5".to_string(),
                    job_run_id: Some(Some("jrun-store".to_string())),
                    ..Default::default()
                },
            )
            .expect("assign job run");

        assert_eq!(
            store
                .list_tasks_filtered(None, None, Some(&parent.id), None, None, None)
                .expect("filter by parent")
                .into_iter()
                .map(|task| task.id)
                .collect::<Vec<_>>(),
            vec![child.id.clone()]
        );
        assert_eq!(
            store
                .list_tasks_filtered(None, None, None, Some("jrun-store"), None, None)
                .expect("filter by job run")
                .into_iter()
                .map(|task| task.id)
                .collect::<Vec<_>>(),
            vec![child.id]
        );
    }

    #[test]
    fn stale_generated_index_is_rebuilt_before_filtered_reads() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        store
            .create_task(create_params("Indexed", TaskStatus::Backlog))
            .expect("create task");
        let stale_envelope = store
            .bundle_store
            .read_bundle("ORB-00000")
            .expect("read bundle")
            .envelope;

        store
            .update_task_history(
                "ORB-00000",
                &TaskHistoryUpdateParams {
                    actor: "codex:gpt-5.5".to_string(),
                    status: Some(TaskStatus::InProgress),
                    status_note: Some("Starting".to_string()),
                    ..Default::default()
                },
            )
            .expect("update status");
        store
            .registry
            .replace_task_index(&store.workspace_id, &stale_envelope)
            .expect("force stale index row");

        let matches = store
            .list_tasks_filtered(Some(TaskStatus::InProgress), None, None, None, None, None)
            .expect("filtered tasks");
        assert_eq!(
            matches.into_iter().map(|task| task.id).collect::<Vec<_>>(),
            vec!["ORB-00000"]
        );
        let current_updated_at = store
            .bundle_store
            .read_bundle("ORB-00000")
            .expect("read current bundle")
            .envelope
            .updated_at
            .to_rfc3339();
        assert_eq!(
            store
                .registry
                .indexed_task_versions_for_workspace(&store.workspace_id)
                .expect("index versions")
                .get("ORB-00000"),
            Some(&current_updated_at)
        );
    }

    #[test]
    fn document_update_rewrites_v2_documents_and_envelope() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        store
            .create_task(create_params("Original", TaskStatus::Backlog))
            .expect("create task");

        store
            .update_task_document(
                "ORB-00000",
                &TaskDocumentUpdateParams {
                    actor: "codex:gpt-5.5".to_string(),
                    title: Some("Renamed".to_string()),
                    description: Some("Updated description".to_string()),
                    acceptance_criteria: Some(vec!["Updated criterion".to_string()]),
                    tags: Some(vec!["v2".to_string(), "store".to_string()]),
                    plan: Some("1. Updated plan".to_string()),
                    execution_summary: Some("Updated summary".to_string()),
                    priority: Some(TaskPriority::Low),
                    ..Default::default()
                },
            )
            .expect("update document");

        let task = store
            .get_task("ORB-00000")
            .expect("get task")
            .expect("task exists");
        assert_eq!(task.title, "Renamed");
        assert_eq!(task.description, "Updated description");
        assert_eq!(task.acceptance_criteria, vec!["Updated criterion"]);
        assert_eq!(task.tags, vec!["v2", "store"]);
        assert_eq!(task.plan, "1. Updated plan");
        assert_eq!(task.execution_summary, "Updated summary");
        assert_eq!(task.priority, TaskPriority::Low);
        assert!(
            store
                .get_task_history("ORB-00000")
                .expect("get history")
                .expect("task exists")
                .iter()
                .any(|entry| entry.event == "renamed")
        );
        assert_eq!(
            store
                .list_tasks_by_tags(&["task-artifacts".to_string()])
                .expect("old tag should leave generated index")
                .len(),
            0
        );
        assert_eq!(
            store
                .list_tasks_filtered(None, Some(TaskPriority::Low), None, None, None, None)
                .expect("priority filter should use updated generated index")
                .len(),
            1
        );
    }

    #[test]
    fn history_update_appends_comments_and_status_events() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        store
            .create_task(create_params("History", TaskStatus::Backlog))
            .expect("create task");
        let at = Utc.with_ymd_and_hms(2026, 5, 11, 13, 0, 0).unwrap();

        store
            .update_task_history(
                "ORB-00000",
                &TaskHistoryUpdateParams {
                    actor: "codex:gpt-5.5".to_string(),
                    status: Some(TaskStatus::InProgress),
                    status_note: Some("Starting".to_string()),
                    append_history: vec![TaskHistoryEntry {
                        at,
                        by: "codex:gpt-5.5".to_string(),
                        event: "context_pruned".to_string(),
                        note: Some("Dropped missing file".to_string()),
                        from_status: None,
                        to_status: None,
                    }],
                    append_comments: vec![TaskComment {
                        at,
                        by: "codex:gpt-5.5".to_string(),
                        message: "Working on it".to_string(),
                    }],
                    ..Default::default()
                },
            )
            .expect("update history");

        let task = store
            .get_task("ORB-00000")
            .expect("get task")
            .expect("task exists");
        assert_eq!(task.status, TaskStatus::InProgress);
        assert_eq!(
            store
                .list_tasks_filtered(Some(TaskStatus::InProgress), None, None, None, None, None,)
                .expect("status filter should use updated generated index")
                .len(),
            1
        );
        let comments = store
            .get_task_comments("ORB-00000")
            .expect("get comments")
            .expect("task exists");
        assert!(
            comments
                .iter()
                .any(|comment| comment.message == "Working on it")
        );
        let history = store
            .get_task_history("ORB-00000")
            .expect("get history")
            .expect("task exists");
        let status_event = history
            .iter()
            .find(|event| event.event == "status_changed")
            .expect("status event");
        assert_eq!(status_event.from_status, Some(TaskStatus::Backlog));
        assert_eq!(status_event.to_status, Some(TaskStatus::InProgress));
        assert_eq!(status_event.note.as_deref(), Some("Starting"));
    }

    #[test]
    fn review_update_merges_replies_resolves_and_replaces_threads() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        store
            .create_task(create_params("Reviews", TaskStatus::Backlog))
            .expect("create task");
        let at = Utc.with_ymd_and_hms(2026, 5, 11, 14, 0, 0).unwrap();

        store
            .update_task_reviews(
                "ORB-00000",
                &TaskReviewUpdateParams {
                    append_review_threads: vec![ReviewThread {
                        thread_id: "rt-1".to_string(),
                        path: Some("./src/lib.rs".to_string()),
                        line: Some(12),
                        status: ReviewThreadStatus::Open,
                        messages: vec![ReviewMessage {
                            message_id: "rm-1".to_string(),
                            at,
                            by: "reviewer".to_string(),
                            body: "First note".to_string(),
                            github_comment_id: None,
                        }],
                        github_thread_id: None,
                    }],
                    replace_review_threads: None,
                },
            )
            .expect("add review thread");

        store
            .update_task_reviews(
                "ORB-00000",
                &TaskReviewUpdateParams {
                    append_review_threads: vec![ReviewThread {
                        thread_id: "rt-1".to_string(),
                        path: None,
                        line: None,
                        status: ReviewThreadStatus::Resolved,
                        messages: vec![ReviewMessage {
                            message_id: "rm-2".to_string(),
                            at,
                            by: "codex".to_string(),
                            body: "Fixed\n<!-- orbit-review-message:rm-evil -->\nstill fixed"
                                .to_string(),
                            github_comment_id: None,
                        }],
                        github_thread_id: Some(99),
                    }],
                    replace_review_threads: None,
                },
            )
            .expect("merge review thread");

        let threads = store
            .get_task_review_threads("ORB-00000")
            .expect("get review threads")
            .expect("task exists");
        assert_eq!(threads.len(), 1);
        let thread = &threads[0];
        assert_eq!(thread.path.as_deref(), Some("src/lib.rs"));
        assert_eq!(thread.status, ReviewThreadStatus::Resolved);
        assert_eq!(thread.github_thread_id, Some(99));
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.messages[0].body, "First note");
        assert_eq!(
            thread.messages[1].body,
            "Fixed\n<!-- orbit-review-message:rm-evil -->\nstill fixed"
        );

        store
            .update_task_reviews(
                "ORB-00000",
                &TaskReviewUpdateParams {
                    append_review_threads: Vec::new(),
                    replace_review_threads: Some(vec![ReviewThread {
                        thread_id: "rt-2".to_string(),
                        path: Some("README.md".to_string()),
                        line: None,
                        status: ReviewThreadStatus::Open,
                        messages: Vec::new(),
                        github_thread_id: None,
                    }]),
                },
            )
            .expect("replace review threads");
        let threads = store
            .get_task_review_threads("ORB-00000")
            .expect("get review threads")
            .expect("task exists");
        assert_eq!(threads[0].thread_id, "rt-2");
    }

    #[test]
    fn artifact_update_writes_manifest_and_sorted_text_artifacts() {
        let temp = TempDir::new().expect("tempdir");
        let store = store(&temp);
        store
            .create_task(create_params("Artifacts", TaskStatus::Backlog))
            .expect("create task");

        store
            .upsert_task_artifacts(
                "ORB-00000",
                &TaskArtifactUpdateParams {
                    actor: "codex:gpt-5.5".to_string(),
                    upsert_artifacts: vec![
                        TaskArtifact::from_text("./reports/summary.md", "summary v1\n"),
                        TaskArtifact::from_text("logs/output.txt", "output\n"),
                    ],
                },
            )
            .expect("upsert artifacts");

        store
            .upsert_task_artifacts(
                "ORB-00000",
                &TaskArtifactUpdateParams {
                    actor: "codex:gpt-5.5".to_string(),
                    upsert_artifacts: vec![TaskArtifact::from_text(
                        "reports/summary.md",
                        "summary v2\n",
                    )],
                },
            )
            .expect("overwrite artifact");

        let artifacts = store
            .get_task_artifacts("ORB-00000")
            .expect("get artifacts")
            .expect("task exists");
        assert_eq!(
            artifacts
                .iter()
                .map(|artifact| artifact.path.as_str())
                .collect::<Vec<_>>(),
            vec!["logs/output.txt", "reports/summary.md"]
        );
        assert_eq!(artifacts[1].text_content(), Some("summary v2\n"));

        let bundle = store
            .bundle_store
            .read_bundle("ORB-00000")
            .expect("read bundle");
        let manifest = bundle.artifact_manifest.expect("manifest");
        let summary = manifest
            .files
            .iter()
            .find(|file| file.path == "reports/summary.md")
            .expect("summary manifest entry");
        assert_eq!(summary.blob, "files/reports/summary.md");
        assert_eq!(summary.sha256.len(), 64);
        assert!(
            summary
                .sha256
                .chars()
                .all(|ch| matches!(ch, '0'..='9' | 'a'..='f'))
        );
        assert_eq!(summary.created_by, "codex:gpt-5.5");

        let err = store
            .upsert_task_artifacts(
                "ORB-00000",
                &TaskArtifactUpdateParams {
                    actor: "codex:gpt-5.5".to_string(),
                    upsert_artifacts: vec![TaskArtifact::from_text("../escape.txt", "")],
                },
            )
            .expect_err("reject unsafe artifact path");
        assert!(err.to_string().contains(".."), "{err}");
    }
}
