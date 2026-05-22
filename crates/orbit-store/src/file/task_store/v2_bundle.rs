//! Task bundle v2 persistence is split into focused submodules while this root keeps store orchestration and re-exports.
//! The `task_bundle_types` submodule owns bundle value types and document filename mapping.
//! The `bundle_io` submodule owns bundle assembly, validation, JSONL repair, artifact manifest checks, and partial-bundle cleanup.
//! The `review_threads` submodule owns review-thread sidecar files and tombstone recovery for interrupted rewrites.
//! The `lock` submodule owns bundle create/delete lock ordering, sentinel cleanup, and projection-entry crash recovery checks.

use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::{
    ArtifactManifestV2, OrbitError, TASK_ARTIFACT_MANIFEST_FILE_NAME, TASK_ARTIFACTS_DIR_NAME,
    TASK_COMMENTS_FILE_NAME, TASK_ENVELOPE_FILE_NAME, TASK_EVENTS_FILE_NAME, TaskCommentRowV2,
    TaskEnvelopeV2, TaskEventRowV2,
};
use orbit_common::utility::fs::{atomic_write_text, with_exclusive_file_lock};

use crate::sqlite::task_registry::{ProjectionRebuildResult, TaskRegistryStore};

pub(crate) mod bundle_io;
pub(crate) mod lock;
pub(crate) mod review_threads;
pub(crate) mod task_bundle_types;

pub(crate) use bundle_io::{
    append_jsonl_row, cleanup_partial_bundle_best_effort, read_bundle_at, write_bundle_at,
    write_yaml_file,
};
pub(crate) use lock::{
    ensure_projection_entry_removable, remove_projection_entry, remove_task_bundle_lock_sentinel,
    task_bundle_lock_sentinel_path,
};
pub(crate) use review_threads::rewrite_review_threads;
pub(crate) use task_bundle_types::{
    TaskBundleCreateResult, TaskBundleV2, TaskDocumentV2, TaskReviewThreadV2,
};

pub(crate) struct TaskBundleStoreV2 {
    // pub(crate) fields widened to allow sibling `tests/v2_bundle.rs` (and promoted
    // `tests/test_support.rs`) to access internal state for durability and projection
    // assertions. See ORB-00247 and docs/design-patterns/test_layout.md (widen
    // deliberately rather than keep nested anti-pattern).
    pub(crate) registry: TaskRegistryStore,
    pub(crate) workspace_id: String,
    pub(crate) workspace_orbit_dir: PathBuf,
}

impl TaskBundleStoreV2 {
    pub(crate) fn new(
        registry: TaskRegistryStore,
        workspace_id: String,
        workspace_orbit_dir: PathBuf,
    ) -> Self {
        Self {
            registry,
            workspace_id,
            workspace_orbit_dir,
        }
    }

    pub(crate) fn bundle_path(&self, task_id: &str) -> Result<PathBuf, OrbitError> {
        self.registry
            .canonical_task_bundle_path(&self.workspace_id, task_id)
    }

    pub(crate) fn create_bundle(
        &self,
        bundle: &TaskBundleV2,
    ) -> Result<TaskBundleCreateResult, OrbitError> {
        let task_id = bundle.envelope.id.clone();
        let bundle_dir = self.bundle_path(&task_id)?;
        let lock_sentinel_path = task_bundle_lock_sentinel_path(&bundle_dir)?;
        with_exclusive_file_lock(&bundle_dir, "task bundle create", || {
            let result = self.create_bundle_locked(&task_id, &bundle_dir, bundle);
            match (
                result,
                remove_task_bundle_lock_sentinel(&lock_sentinel_path),
            ) {
                (Ok(created), Ok(())) => Ok(created),
                (Err(err), Ok(())) => Err(err),
                (Ok(_), Err(cleanup_err)) => Err(cleanup_err),
                (Err(err), Err(cleanup_err)) => {
                    orbit_common::tracing::warn!(
                        target: "orbit.store.task_bundle_v2",
                        task_id,
                        lock_path = %lock_sentinel_path.display(),
                        original_error = %err,
                        cleanup_error = %cleanup_err,
                        "failed to clean up task bundle lock sentinel",
                    );
                    Err(err)
                }
            }
        })
    }

    fn create_bundle_locked(
        &self,
        task_id: &str,
        bundle_dir: &Path,
        bundle: &TaskBundleV2,
    ) -> Result<TaskBundleCreateResult, OrbitError> {
        if bundle_dir.exists() {
            return Err(OrbitError::Store(format!(
                "task bundle already exists at {}",
                bundle_dir.display()
            )));
        }

        if let Err(err) = write_bundle_at(bundle_dir, bundle) {
            cleanup_partial_bundle_best_effort(bundle_dir, "bundle write", &err);
            return Err(err);
        }

        let binding =
            match self
                .registry
                .register_task_bundle(task_id, &self.workspace_id, bundle_dir)
            {
                Ok(binding) => binding,
                Err(err) => {
                    cleanup_partial_bundle_best_effort(bundle_dir, "registry registration", &err);
                    return Err(err);
                }
            };
        let projection = match self
            .registry
            .rebuild_projection(&self.workspace_orbit_dir, &self.workspace_id)
        {
            Ok(projection) => projection,
            Err(err) => {
                orbit_common::tracing::warn!(
                    target: "orbit.store.task_bundle_v2",
                    task_id,
                    workspace_id = %self.workspace_id,
                    error = %err,
                    "task bundle created, but projection rebuild failed; continuing in degraded mode",
                );
                ProjectionRebuildResult {
                    projected: 0,
                    repaired: 0,
                    degraded_reason: Some(format!(
                        "projection rebuild failed after bundle creation: {err}"
                    )),
                }
            }
        };

        Ok(TaskBundleCreateResult {
            binding,
            projection,
        })
    }

    pub(crate) fn read_bundle(&self, task_id: &str) -> Result<TaskBundleV2, OrbitError> {
        let bundle_dir = self.bundle_path(task_id)?;
        read_bundle_at(&bundle_dir)
    }

    pub(crate) fn delete_bundle(&self, task_id: &str) -> Result<bool, OrbitError> {
        orbit_common::types::validate_orb_task_id(task_id)?;
        let bundle_dir = self.bundle_path(task_id)?;

        ensure_projection_entry_removable(&self.workspace_orbit_dir, task_id)?;
        let unregistered = self
            .registry
            .unregister_task_bundle(task_id, &self.workspace_id)?;
        let removed_bundle = match fs::remove_dir_all(&bundle_dir) {
            Ok(()) => true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
            Err(err) => return Err(OrbitError::Io(err.to_string())),
        };
        let removed_projection = remove_projection_entry(&self.workspace_orbit_dir, task_id)?;
        Ok(unregistered || removed_bundle || removed_projection)
    }

    /// List bundles registered to this workspace.
    ///
    /// This is intentionally fail-fast for now: one corrupt registered bundle
    /// makes the list fail so early wiring does not silently hide store damage.
    pub(crate) fn list_bundles(&self) -> Result<Vec<TaskBundleV2>, OrbitError> {
        let bindings = self.registry.tasks_for_workspace(&self.workspace_id)?;
        bindings
            .iter()
            .map(|binding| read_bundle_at(&binding.canonical_path))
            .collect()
    }

    pub(crate) fn rewrite_document(
        &self,
        task_id: &str,
        document: TaskDocumentV2,
        content: &str,
    ) -> Result<(), OrbitError> {
        let path = self.bundle_path(task_id)?.join(document.file_name());
        atomic_write_text(&path, content).map_err(|err| OrbitError::Io(err.to_string()))
    }

    pub(crate) fn rewrite_envelope(
        &self,
        task_id: &str,
        envelope: &TaskEnvelopeV2,
    ) -> Result<(), OrbitError> {
        if envelope.id != task_id {
            return Err(OrbitError::InvalidInput(format!(
                "task envelope id '{}' does not match target task id '{task_id}'",
                envelope.id
            )));
        }
        envelope.validate()?;
        write_yaml_file(
            &self.bundle_path(task_id)?.join(TASK_ENVELOPE_FILE_NAME),
            envelope,
        )
    }

    pub(crate) fn rewrite_review_threads(
        &self,
        task_id: &str,
        threads: &[TaskReviewThreadV2],
    ) -> Result<(), OrbitError> {
        let bundle_dir = self.bundle_path(task_id)?;
        rewrite_review_threads(&bundle_dir, threads)
    }

    pub(crate) fn rewrite_artifact_manifest(
        &self,
        task_id: &str,
        manifest: &ArtifactManifestV2,
    ) -> Result<(), OrbitError> {
        manifest.validate()?;
        write_yaml_file(
            &self
                .bundle_path(task_id)?
                .join(TASK_ARTIFACTS_DIR_NAME)
                .join(TASK_ARTIFACT_MANIFEST_FILE_NAME),
            manifest,
        )
    }

    pub(crate) fn append_event(
        &self,
        task_id: &str,
        event: &TaskEventRowV2,
    ) -> Result<(), OrbitError> {
        event.validate()?;
        append_jsonl_row(
            &self.bundle_path(task_id)?.join(TASK_EVENTS_FILE_NAME),
            event,
        )
    }

    pub(crate) fn append_comment(
        &self,
        task_id: &str,
        comment: &TaskCommentRowV2,
    ) -> Result<(), OrbitError> {
        comment.validate()?;
        append_jsonl_row(
            &self.bundle_path(task_id)?.join(TASK_COMMENTS_FILE_NAME),
            comment,
        )
    }
}
