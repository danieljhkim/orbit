//! Task bundle v2 persistence is split into focused submodules by operation surface.
//! The `crud` module owns task creation, listing, filtering, searching, and deletion.
//! The `updates` module owns document, history, and review-thread mutations.
//! The `artifacts` module owns task artifact reads, manifests, and upserts.
//! The `sidecars` module owns comments, history rows, and review-thread reads.
//! The `index` module owns generated index reads, rebuilds, bundle translation, and task locking helpers.
//! The `query` module owns in-memory, sidecar, and artifact query matching.
//! The `relations` module owns relation construction and replacement helpers.
//! The `review_threads` module owns conversion, merge, and markdown serialization helpers for review threads.
//! The `sequencing` module owns monotonic event and comment sequence calculations.
//! The `artifact_paths` module owns artifact path normalization and safe resolution.
//! The `acceptance` module owns acceptance-criteria rendering and parsing.
//! The `document_fields` module owns v2 unsupported-field validation.

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

mod acceptance;
mod artifact_paths;
mod artifacts;
mod crud;
mod document_fields;
mod index;
mod query;
mod relations;
mod review_threads;
mod sequencing;
mod sidecars;
mod updates;

#[cfg(test)]
mod tests;

use acceptance::{parse_acceptance, render_acceptance};
use artifact_paths::{normalize_v2_artifact_path, resolve_v2_artifact_file_path};
use document_fields::reject_unsupported_document_fields;
use relations::{relations_from_create_params, replace_relations};
use review_threads::{merge_review_threads_v2, review_thread_from_v2, review_thread_to_v2};
use sequencing::{next_event_id, next_sequence};

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
}
