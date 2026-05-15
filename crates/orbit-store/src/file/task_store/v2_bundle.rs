use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use orbit_common::migration::Plan;
use orbit_common::types::{
    ArtifactManifestV2, NotFoundKind, OrbitError, ReviewThreadMetadataV2,
    TASK_ACCEPTANCE_FILE_NAME, TASK_ARTIFACT_FILES_DIR_NAME, TASK_ARTIFACT_MANIFEST_FILE_NAME,
    TASK_ARTIFACTS_DIR_NAME, TASK_COMMENTS_FILE_NAME, TASK_DESCRIPTION_FILE_NAME,
    TASK_ENVELOPE_FILE_NAME, TASK_EVENTS_FILE_NAME, TASK_EXECUTION_SUMMARY_FILE_NAME,
    TASK_PLAN_FILE_NAME, TASK_REVIEW_THREADS_DIR_NAME, TaskCommentRowV2, TaskEnvelopeV2,
    TaskEventRowV2,
};
use orbit_common::utility::fs::{atomic_write_text, with_exclusive_file_lock};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::file::task_store::task_migrations;
use crate::sqlite::task_registry::{ProjectionRebuildResult, TaskBundleBinding, TaskRegistryStore};

const REVIEW_THREAD_TOMBSTONES_FILE: &str = ".tombstones";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskBundleV2 {
    pub(crate) envelope: TaskEnvelopeV2,
    pub(crate) description: String,
    pub(crate) acceptance: String,
    pub(crate) plan: String,
    pub(crate) execution_summary: String,
    pub(crate) events: Vec<TaskEventRowV2>,
    pub(crate) comments: Vec<TaskCommentRowV2>,
    pub(crate) review_threads: Vec<TaskReviewThreadV2>,
    pub(crate) artifact_manifest: Option<ArtifactManifestV2>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskReviewThreadV2 {
    pub(crate) metadata: ReviewThreadMetadataV2,
    pub(crate) body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskDocumentV2 {
    Description,
    Acceptance,
    Plan,
    ExecutionSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskBundleCreateResult {
    pub(crate) binding: TaskBundleBinding,
    pub(crate) projection: ProjectionRebuildResult,
}

pub(crate) struct TaskBundleStoreV2 {
    registry: TaskRegistryStore,
    workspace_id: String,
    workspace_orbit_dir: PathBuf,
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

fn task_bundle_lock_sentinel_path(bundle_dir: &Path) -> Result<PathBuf, OrbitError> {
    let file_name = bundle_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            OrbitError::Store(format!(
                "task bundle path {} has no file name",
                bundle_dir.display()
            ))
        })?;
    Ok(bundle_dir.with_file_name(format!(".{file_name}.lock")))
}

fn remove_task_bundle_lock_sentinel(lock_path: &Path) -> Result<(), OrbitError> {
    match fs::remove_file(lock_path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

fn ensure_projection_entry_removable(
    workspace_orbit_dir: &Path,
    task_id: &str,
) -> Result<(), OrbitError> {
    let projection_path = workspace_orbit_dir.join("tasks").join(task_id);
    match fs::symlink_metadata(&projection_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(OrbitError::Store(format!(
            "projection entry '{}' already exists and is not a symlink",
            projection_path.display()
        ))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

fn remove_projection_entry(workspace_orbit_dir: &Path, task_id: &str) -> Result<bool, OrbitError> {
    let projection_path = workspace_orbit_dir.join("tasks").join(task_id);
    match fs::symlink_metadata(&projection_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::remove_file(&projection_path).map_err(|err| OrbitError::Io(err.to_string()))?;
            Ok(true)
        }
        Ok(_) => Err(OrbitError::Store(format!(
            "projection entry '{}' already exists and is not a symlink",
            projection_path.display()
        ))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

impl TaskDocumentV2 {
    fn file_name(self) -> &'static str {
        match self {
            Self::Description => TASK_DESCRIPTION_FILE_NAME,
            Self::Acceptance => TASK_ACCEPTANCE_FILE_NAME,
            Self::Plan => TASK_PLAN_FILE_NAME,
            Self::ExecutionSummary => TASK_EXECUTION_SUMMARY_FILE_NAME,
        }
    }
}

/// Write a new v2 bundle at `bundle_dir`.
///
/// This is a creation-only primitive and refuses to write into an existing
/// bundle directory. Use narrower update helpers for later mutations.
pub(crate) fn write_bundle_at(bundle_dir: &Path, bundle: &TaskBundleV2) -> Result<(), OrbitError> {
    if bundle_dir.exists() {
        return Err(OrbitError::Store(format!(
            "task bundle already exists at {}",
            bundle_dir.display()
        )));
    }
    validate_bundle_dir_matches_task_id(bundle_dir, &bundle.envelope.id)?;
    validate_bundle(bundle)?;
    ensure_bundle_dirs(bundle_dir)?;

    write_yaml_file(&bundle_dir.join(TASK_ENVELOPE_FILE_NAME), &bundle.envelope)?;
    atomic_write_text(
        &bundle_dir.join(TASK_DESCRIPTION_FILE_NAME),
        &bundle.description,
    )
    .map_err(|err| OrbitError::Io(err.to_string()))?;
    atomic_write_text(
        &bundle_dir.join(TASK_ACCEPTANCE_FILE_NAME),
        &bundle.acceptance,
    )
    .map_err(|err| OrbitError::Io(err.to_string()))?;
    atomic_write_text(&bundle_dir.join(TASK_PLAN_FILE_NAME), &bundle.plan)
        .map_err(|err| OrbitError::Io(err.to_string()))?;
    atomic_write_text(
        &bundle_dir.join(TASK_EXECUTION_SUMMARY_FILE_NAME),
        &bundle.execution_summary,
    )
    .map_err(|err| OrbitError::Io(err.to_string()))?;

    write_jsonl_file(&bundle_dir.join(TASK_EVENTS_FILE_NAME), &bundle.events)?;
    write_jsonl_file(&bundle_dir.join(TASK_COMMENTS_FILE_NAME), &bundle.comments)?;
    write_review_threads(bundle_dir, &bundle.review_threads)?;
    if let Some(manifest) = &bundle.artifact_manifest {
        write_yaml_file(
            &bundle_dir
                .join(TASK_ARTIFACTS_DIR_NAME)
                .join(TASK_ARTIFACT_MANIFEST_FILE_NAME),
            manifest,
        )?;
    }

    Ok(())
}

pub(crate) fn read_bundle_at(bundle_dir: &Path) -> Result<TaskBundleV2, OrbitError> {
    let expected_task_id = task_id_from_bundle_dir(bundle_dir)?;
    let envelope_path = bundle_dir.join(TASK_ENVELOPE_FILE_NAME);
    if !envelope_path.is_file() {
        return Err(OrbitError::not_found(NotFoundKind::Task, expected_task_id));
    }
    let envelope: TaskEnvelopeV2 =
        read_migrated_yaml_file(&envelope_path, task_migrations::envelope_plan())?;
    validate_bundle_dir_matches_task_id(bundle_dir, &envelope.id)?;

    let bundle = TaskBundleV2 {
        envelope,
        description: read_required_text(&bundle_dir.join(TASK_DESCRIPTION_FILE_NAME))?,
        acceptance: read_required_text(&bundle_dir.join(TASK_ACCEPTANCE_FILE_NAME))?,
        plan: read_required_text(&bundle_dir.join(TASK_PLAN_FILE_NAME))?,
        execution_summary: read_required_text(&bundle_dir.join(TASK_EXECUTION_SUMMARY_FILE_NAME))?,
        events: read_task_events(&bundle_dir.join(TASK_EVENTS_FILE_NAME))?,
        comments: read_task_comments(&bundle_dir.join(TASK_COMMENTS_FILE_NAME))?,
        review_threads: read_review_threads(bundle_dir)?,
        artifact_manifest: read_artifact_manifest(bundle_dir)?,
    };
    validate_bundle(&bundle)?;
    Ok(bundle)
}

fn validate_bundle(bundle: &TaskBundleV2) -> Result<(), OrbitError> {
    bundle.envelope.validate()?;
    for event in &bundle.events {
        event.validate()?;
    }
    for comment in &bundle.comments {
        comment.validate()?;
    }
    for thread in &bundle.review_threads {
        thread.metadata.validate()?;
    }
    if let Some(manifest) = &bundle.artifact_manifest {
        manifest.validate()?;
    }
    validate_bundle_consistency(bundle)?;
    Ok(())
}

fn validate_bundle_consistency(bundle: &TaskBundleV2) -> Result<(), OrbitError> {
    if let Some(last_status) = bundle.events.iter().rev().find_map(|event| event.to_status)
        && last_status != bundle.envelope.status
    {
        return Err(OrbitError::Store(format!(
            "task event log status '{}' does not match envelope status '{}' for {}",
            last_status, bundle.envelope.status, bundle.envelope.id
        )));
    }
    Ok(())
}

fn ensure_bundle_dirs(bundle_dir: &Path) -> Result<(), OrbitError> {
    fs::create_dir_all(bundle_dir).map_err(|err| OrbitError::Io(err.to_string()))?;
    fs::create_dir_all(bundle_dir.join(TASK_REVIEW_THREADS_DIR_NAME))
        .map_err(|err| OrbitError::Io(err.to_string()))?;
    fs::create_dir_all(
        bundle_dir
            .join(TASK_ARTIFACTS_DIR_NAME)
            .join(TASK_ARTIFACT_FILES_DIR_NAME),
    )
    .map_err(|err| OrbitError::Io(err.to_string()))?;
    Ok(())
}

fn validate_bundle_dir_matches_task_id(bundle_dir: &Path, task_id: &str) -> Result<(), OrbitError> {
    let name = task_id_from_bundle_dir(bundle_dir)?;
    if name != task_id {
        return Err(OrbitError::Store(format!(
            "task bundle directory {} does not match task id {}",
            bundle_dir.display(),
            task_id
        )));
    }
    Ok(())
}

fn task_id_from_bundle_dir(bundle_dir: &Path) -> Result<String, OrbitError> {
    bundle_dir
        .file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            OrbitError::Store(format!("invalid task bundle path {}", bundle_dir.display()))
        })
}

fn write_yaml_file<T>(path: &Path, value: &T) -> Result<(), OrbitError>
where
    T: serde::Serialize,
{
    let yaml = serde_yaml::to_string(value).map_err(|err| OrbitError::Store(err.to_string()))?;
    atomic_write_text(path, &yaml).map_err(|err| OrbitError::Io(err.to_string()))
}

fn read_yaml_file<T>(path: &Path) -> Result<T, OrbitError>
where
    T: DeserializeOwned,
{
    let raw = read_required_text(path)?;
    serde_yaml::from_str(&raw)
        .map_err(|err| OrbitError::Store(format!("invalid YAML at {}: {err}", path.display())))
}

fn read_migrated_yaml_file<T>(path: &Path, plan: &Plan) -> Result<T, OrbitError>
where
    T: DeserializeOwned,
{
    let raw = read_required_text(path)?;
    let value: serde_yaml::Value = serde_yaml::from_str(&raw)
        .map_err(|err| OrbitError::Store(format!("invalid YAML at {}: {err}", path.display())))?;
    let migrated = plan.migrate(value).map_err(|err| match err {
        OrbitError::Migration(msg) => {
            OrbitError::Migration(format!("{} ({})", msg, path.display()))
        }
        other => other,
    })?;
    serde_yaml::from_value(migrated)
        .map_err(|err| OrbitError::Store(format!("invalid YAML at {}: {err}", path.display())))
}

fn read_required_text(path: &Path) -> Result<String, OrbitError> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(value),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(OrbitError::Store(format!(
            "missing task bundle file {}",
            path.display()
        ))),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

fn write_jsonl_file<T>(path: &Path, rows: &[T]) -> Result<(), OrbitError>
where
    T: serde::Serialize,
{
    let mut content = String::new();
    for row in rows {
        content.push_str(
            &serde_json::to_string(row).map_err(|err| OrbitError::Store(err.to_string()))?,
        );
        content.push('\n');
    }
    atomic_write_text(path, &content).map_err(|err| OrbitError::Io(err.to_string()))
}

fn read_task_events(path: &Path) -> Result<Vec<TaskEventRowV2>, OrbitError> {
    let events: Vec<TaskEventRowV2> = read_jsonl_file(path)?;
    for event in &events {
        event.validate()?;
    }
    Ok(events)
}

fn read_task_comments(path: &Path) -> Result<Vec<TaskCommentRowV2>, OrbitError> {
    let comments: Vec<TaskCommentRowV2> = read_jsonl_file(path)?;
    for comment in &comments {
        comment.validate()?;
    }
    Ok(comments)
}

fn read_jsonl_file<T>(path: &Path) -> Result<Vec<T>, OrbitError>
where
    T: DeserializeOwned,
{
    let raw = read_required_text(path)?;
    scan_jsonl_records(path, &raw)
}

fn append_jsonl_row<T>(path: &Path, row: &T) -> Result<(), OrbitError>
where
    T: serde::Serialize,
{
    let encoded = serde_json::to_string(row).map_err(|err| OrbitError::Store(err.to_string()))?;
    with_exclusive_file_lock(path, "task jsonl", || {
        repair_jsonl_tail(path)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(OrbitError::from)?;
        file.write_all(encoded.as_bytes())
            .map_err(OrbitError::from)?;
        file.write_all(b"\n").map_err(OrbitError::from)?;
        file.flush().map_err(OrbitError::from)?;
        file.sync_all().map_err(OrbitError::from)?;
        Ok(())
    })
}

fn repair_jsonl_tail(path: &Path) -> Result<(), OrbitError> {
    let Ok(mut file) = OpenOptions::new().read(true).write(true).open(path) else {
        return Ok(());
    };

    let mut raw = String::new();
    file.read_to_string(&mut raw).map_err(OrbitError::from)?;
    let scan = scan_jsonl_tail(path, &raw)?;
    if scan.truncate_at < raw.len() as u64 {
        file.set_len(scan.truncate_at).map_err(OrbitError::from)?;
        file.seek(SeekFrom::End(0)).map_err(OrbitError::from)?;
        file.sync_all().map_err(OrbitError::from)?;
    }
    Ok(())
}

fn scan_jsonl_records<T>(path: &Path, raw: &str) -> Result<Vec<T>, OrbitError>
where
    T: DeserializeOwned,
{
    let scan = scan_jsonl_tail(path, raw)?;
    let valid = &raw[..scan.truncate_at as usize];
    valid
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).map_err(|err| {
                OrbitError::Store(format!("invalid JSONL row at {}: {err}", path.display()))
            })
        })
        .collect()
}

struct JsonlTailScan {
    truncate_at: u64,
}

fn scan_jsonl_tail(path: &Path, raw: &str) -> Result<JsonlTailScan, OrbitError> {
    if raw.is_empty() {
        return Ok(JsonlTailScan { truncate_at: 0 });
    }

    let mut offset = 0usize;
    let mut last_good = 0usize;
    for chunk in raw.split_inclusive('\n') {
        let next_offset = offset + chunk.len();
        if !chunk.ends_with('\n') {
            return Ok(JsonlTailScan {
                truncate_at: last_good as u64,
            });
        }

        let line = chunk.trim_end_matches('\n').trim_end_matches('\r');
        if line.trim().is_empty() {
            return Err(OrbitError::Store(format!(
                "blank JSONL row before tail at {}",
                path.display()
            )));
        }
        if let Err(err) = serde_json::from_str::<serde_json::Value>(line) {
            if next_offset == raw.len() {
                return Ok(JsonlTailScan {
                    truncate_at: last_good as u64,
                });
            }
            return Err(OrbitError::Store(format!(
                "invalid JSONL row before tail at {}: {err}",
                path.display()
            )));
        }

        last_good = next_offset;
        offset = next_offset;
    }

    Ok(JsonlTailScan {
        truncate_at: last_good as u64,
    })
}

fn write_review_threads(
    bundle_dir: &Path,
    threads: &[TaskReviewThreadV2],
) -> Result<(), OrbitError> {
    let thread_dir = bundle_dir.join(TASK_REVIEW_THREADS_DIR_NAME);
    fs::create_dir_all(&thread_dir).map_err(|err| OrbitError::Io(err.to_string()))?;
    for thread in threads {
        thread.metadata.validate()?;
        let base = thread_dir.join(&thread.metadata.thread_id);
        write_yaml_file(&base.with_extension("yaml"), &thread.metadata)?;
        atomic_write_text(&base.with_extension("md"), &thread.body)
            .map_err(|err| OrbitError::Io(err.to_string()))?;
    }
    Ok(())
}

fn rewrite_review_threads(
    bundle_dir: &Path,
    threads: &[TaskReviewThreadV2],
) -> Result<(), OrbitError> {
    for thread in threads {
        thread.metadata.validate()?;
    }

    let thread_dir = bundle_dir.join(TASK_REVIEW_THREADS_DIR_NAME);
    fs::create_dir_all(&thread_dir).map_err(|err| OrbitError::Io(err.to_string()))?;
    let expected_paths = threads
        .iter()
        .flat_map(|thread| {
            let base = thread_dir.join(&thread.metadata.thread_id);
            [base.with_extension("yaml"), base.with_extension("md")]
        })
        .collect::<std::collections::BTreeSet<_>>();
    let expected_ids = threads
        .iter()
        .map(|thread| thread.metadata.thread_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let stale_ids = review_thread_ids_on_disk(&thread_dir)?
        .into_iter()
        .filter(|thread_id| !expected_ids.contains(thread_id))
        .collect::<std::collections::BTreeSet<_>>();
    write_review_thread_tombstones(&thread_dir, &stale_ids)?;

    write_review_threads(bundle_dir, threads)?;

    for entry in fs::read_dir(&thread_dir).map_err(|err| OrbitError::Io(err.to_string()))? {
        let entry = entry.map_err(|err| OrbitError::Io(err.to_string()))?;
        let path = entry.path();
        if path.is_file()
            && matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("yaml" | "md")
            )
            && !expected_paths.contains(&path)
        {
            fs::remove_file(&path).map_err(|err| OrbitError::Io(err.to_string()))?;
        }
    }
    remove_review_thread_tombstones(&thread_dir)?;
    Ok(())
}

fn read_review_threads(bundle_dir: &Path) -> Result<Vec<TaskReviewThreadV2>, OrbitError> {
    let thread_dir = bundle_dir.join(TASK_REVIEW_THREADS_DIR_NAME);
    if !thread_dir.is_dir() {
        return Err(OrbitError::Store(format!(
            "missing review thread directory {}",
            thread_dir.display()
        )));
    }

    let tombstones = read_review_thread_tombstones(&thread_dir)?;
    let mut threads = Vec::new();
    for entry in fs::read_dir(&thread_dir).map_err(|err| OrbitError::Io(err.to_string()))? {
        let entry = entry.map_err(|err| OrbitError::Io(err.to_string()))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("yaml") {
            continue;
        }
        let metadata: ReviewThreadMetadataV2 = read_yaml_file(&path)?;
        metadata.validate()?;
        if tombstones.contains(&metadata.thread_id) {
            continue;
        }
        let body = read_required_text(&path.with_extension("md"))?;
        threads.push(TaskReviewThreadV2 { metadata, body });
    }
    threads.sort_by(|left, right| left.metadata.thread_id.cmp(&right.metadata.thread_id));
    Ok(threads)
}

fn review_thread_ids_on_disk(
    thread_dir: &Path,
) -> Result<std::collections::BTreeSet<String>, OrbitError> {
    let mut ids = std::collections::BTreeSet::new();
    for entry in fs::read_dir(thread_dir).map_err(|err| OrbitError::Io(err.to_string()))? {
        let entry = entry.map_err(|err| OrbitError::Io(err.to_string()))?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("yaml") {
            continue;
        }
        if let Some(thread_id) = path.file_stem().and_then(|value| value.to_str()) {
            ids.insert(thread_id.to_string());
        }
    }
    Ok(ids)
}

fn write_review_thread_tombstones(
    thread_dir: &Path,
    stale_ids: &std::collections::BTreeSet<String>,
) -> Result<(), OrbitError> {
    if stale_ids.is_empty() {
        remove_review_thread_tombstones(thread_dir)?;
        return Ok(());
    }
    let body = stale_ids.iter().cloned().collect::<Vec<_>>().join("\n") + "\n";
    atomic_write_text(&thread_dir.join(REVIEW_THREAD_TOMBSTONES_FILE), &body)
        .map_err(|err| OrbitError::Io(err.to_string()))
}

fn remove_review_thread_tombstones(thread_dir: &Path) -> Result<(), OrbitError> {
    match fs::remove_file(thread_dir.join(REVIEW_THREAD_TOMBSTONES_FILE)) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

fn read_review_thread_tombstones(
    thread_dir: &Path,
) -> Result<std::collections::BTreeSet<String>, OrbitError> {
    let path = thread_dir.join(REVIEW_THREAD_TOMBSTONES_FILE);
    match fs::read_to_string(&path) {
        Ok(raw) => Ok(raw
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(std::collections::BTreeSet::new())
        }
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

fn read_artifact_manifest(bundle_dir: &Path) -> Result<Option<ArtifactManifestV2>, OrbitError> {
    let artifact_dir = bundle_dir.join(TASK_ARTIFACTS_DIR_NAME);
    if !artifact_dir.is_dir() {
        return Err(OrbitError::Store(format!(
            "missing artifact directory {}",
            artifact_dir.display()
        )));
    }

    let manifest_path = artifact_dir.join(TASK_ARTIFACT_MANIFEST_FILE_NAME);
    match fs::read_to_string(&manifest_path) {
        Ok(raw) => {
            let manifest: ArtifactManifestV2 = serde_yaml::from_str(&raw).map_err(|err| {
                OrbitError::Store(format!(
                    "invalid artifact manifest {}: {err}",
                    manifest_path.display()
                ))
            })?;
            manifest.validate()?;
            validate_artifact_manifest_files(&artifact_dir, &manifest)?;
            Ok(Some(manifest))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

fn validate_artifact_manifest_files(
    artifact_dir: &Path,
    manifest: &ArtifactManifestV2,
) -> Result<(), OrbitError> {
    for file in &manifest.files {
        let blob_path = artifact_dir.join(&file.blob);
        let bytes = fs::read(&blob_path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                OrbitError::Store(format!(
                    "artifact manifest references missing file {}",
                    blob_path.display()
                ))
            } else {
                OrbitError::Io(err.to_string())
            }
        })?;
        if bytes.len() as u64 != file.size_bytes {
            return Err(OrbitError::Store(format!(
                "artifact manifest size mismatch for {}",
                blob_path.display()
            )));
        }
        let actual_sha256 = format!("{:x}", Sha256::digest(&bytes));
        if actual_sha256 != file.sha256 {
            return Err(OrbitError::Store(format!(
                "artifact manifest sha256 mismatch for {}",
                blob_path.display()
            )));
        }
    }
    Ok(())
}

fn cleanup_partial_bundle(bundle_dir: &Path) -> Result<(), OrbitError> {
    match fs::remove_dir_all(bundle_dir) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

fn cleanup_partial_bundle_best_effort(bundle_dir: &Path, phase: &str, original: &OrbitError) {
    if let Err(cleanup_err) = cleanup_partial_bundle(bundle_dir) {
        orbit_common::tracing::warn!(
            target: "orbit.store.task_bundle_v2",
            bundle_dir = %bundle_dir.display(),
            phase,
            original_error = %original,
            cleanup_error = %cleanup_err,
            "failed to clean up partial task bundle",
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use chrono::{TimeZone, Utc};
    use orbit_common::types::{
        ArtifactManifestFileV2, NotFoundKind, ReviewThreadMessageMetadataV2, ReviewThreadStatus,
        TASK_ARTIFACT_FILES_DIR_NAME, TASK_ARTIFACT_SCHEMA_VERSION, TaskPriority, TaskStatus,
        TaskType,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::sqlite::task_registry::{
        BindWorkspaceParams, TaskRegistryStore, task_registry_path,
    };

    fn sample_bundle(id: &str) -> TaskBundleV2 {
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
        TaskBundleV2 {
            envelope: TaskEnvelopeV2 {
                schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                id: id.to_string(),
                title: "Build v2 bundle store".to_string(),
                status: TaskStatus::Backlog,
                task_type: TaskType::Feature,
                priority: TaskPriority::High,
                complexity: None,
                job_run_id: None,
                relations: Vec::new(),
                tags: vec!["task-artifacts".to_string()],
                context_files: vec!["docs/design/task-artifacts/2_design.md".to_string()],
                external_refs: Vec::new(),
                created_by: Some("codex:gpt-5.5".to_string()),
                planned_by: None,
                implemented_by: None,
                created_at: now,
                updated_at: now,
            },
            description: "Description body".to_string(),
            acceptance: "- [ ] Bundle writes are durable".to_string(),
            plan: "1. Write bundle".to_string(),
            execution_summary: String::new(),
            events: vec![TaskEventRowV2 {
                schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                event_id: "EV-0001".to_string(),
                at: now,
                by: "codex:gpt-5.5".to_string(),
                event_type: "created".to_string(),
                note: None,
                from_status: None,
                to_status: Some(TaskStatus::Backlog),
            }],
            comments: vec![TaskCommentRowV2 {
                schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                comment_id: "C-0001".to_string(),
                at: now,
                by: "daniel".to_string(),
                body: "Looks good.".to_string(),
            }],
            review_threads: Vec::new(),
            artifact_manifest: None,
        }
    }

    fn sample_review_threads() -> Vec<TaskReviewThreadV2> {
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
        vec![
            TaskReviewThreadV2 {
                metadata: ReviewThreadMetadataV2 {
                    schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                    thread_id: "RT-0002".to_string(),
                    status: ReviewThreadStatus::Open,
                    path: Some("src/lib.rs".to_string()),
                    line: Some(42),
                    github_thread_id: None,
                    messages: vec![ReviewThreadMessageMetadataV2 {
                        message_id: "RM-0002".to_string(),
                        at: now,
                        by: "codex:gpt-5.5".to_string(),
                        github_comment_id: None,
                    }],
                    created_at: now,
                    updated_at: now,
                },
                body: "Second thread body".to_string(),
            },
            TaskReviewThreadV2 {
                metadata: ReviewThreadMetadataV2 {
                    schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                    thread_id: "RT-0001".to_string(),
                    status: ReviewThreadStatus::Resolved,
                    path: None,
                    line: None,
                    github_thread_id: Some(123),
                    messages: vec![ReviewThreadMessageMetadataV2 {
                        message_id: "RM-0001".to_string(),
                        at: now,
                        by: "daniel".to_string(),
                        github_comment_id: Some(456),
                    }],
                    created_at: now,
                    updated_at: now,
                },
                body: "First thread body".to_string(),
            },
        ]
    }

    fn bundle_store(temp: &TempDir) -> TaskBundleStoreV2 {
        let registry =
            TaskRegistryStore::open(&task_registry_path(temp.path())).expect("open registry");
        let orbit_dir = temp.path().join("repo").join(".orbit");
        fs::create_dir_all(&orbit_dir).expect("create orbit dir");
        let binding = registry
            .bind_workspace(BindWorkspaceParams {
                workspace_id: Some("orbit-test-123456".to_string()),
                slug: "Orbit Test".to_string(),
                repo_root: temp.path().join("repo"),
                workspace_path: temp.path().join("repo"),
                orbit_dir: orbit_dir.clone(),
                repo_fingerprint: None,
            })
            .expect("bind workspace");
        TaskBundleStoreV2::new(registry, binding.workspace_id, orbit_dir)
    }

    fn task_lock_path(bundle_dir: &Path) -> PathBuf {
        let file_name = bundle_dir
            .file_name()
            .and_then(|value| value.to_str())
            .expect("bundle path has file name");
        bundle_dir.with_file_name(format!(".{file_name}.lock"))
    }

    fn legacy_double_dot_lock_path(bundle_dir: &Path, task_id: &str) -> PathBuf {
        bundle_dir.with_file_name(format!("..{task_id}.create.lock"))
    }

    fn lock_entries_for_task(tasks_dir: &Path, task_id: &str) -> Vec<String> {
        let mut entries = fs::read_dir(tasks_dir)
            .expect("read task workspace dir")
            .map(|entry| {
                entry
                    .expect("read task workspace entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.contains(task_id) && name.ends_with(".lock"))
            .collect::<Vec<_>>();
        entries.sort();
        entries
    }

    #[test]
    fn write_and_read_bundle_round_trips_v2_shape() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        let mut bundle = sample_bundle("ORB-00000");
        bundle.review_threads = sample_review_threads();

        let created = store.create_bundle(&bundle).expect("create bundle");
        assert_eq!(created.binding.task_id, "ORB-00000");

        let read = store.read_bundle("ORB-00000").expect("read bundle");
        assert_eq!(read.envelope, bundle.envelope);
        assert_eq!(read.description, bundle.description);
        assert_eq!(read.acceptance, bundle.acceptance);
        assert_eq!(read.plan, bundle.plan);
        assert_eq!(read.events, bundle.events);
        assert_eq!(read.comments, bundle.comments);
        assert_eq!(
            read.review_threads
                .iter()
                .map(|thread| thread.metadata.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["RT-0001", "RT-0002"]
        );
        assert_eq!(read.review_threads[0].body, "First thread body");
        assert!(
            created
                .binding
                .canonical_path
                .join(TASK_ENVELOPE_FILE_NAME)
                .is_file()
        );
        assert!(
            created
                .binding
                .canonical_path
                .join(TASK_REVIEW_THREADS_DIR_NAME)
                .is_dir()
        );
        assert!(
            created
                .binding
                .canonical_path
                .join(TASK_ARTIFACTS_DIR_NAME)
                .join(TASK_ARTIFACT_FILES_DIR_NAME)
                .is_dir()
        );
        assert!(
            created
                .binding
                .canonical_path
                .join(TASK_REVIEW_THREADS_DIR_NAME)
                .join("RT-0001.yaml")
                .is_file()
        );
        assert!(
            created
                .binding
                .canonical_path
                .join(TASK_REVIEW_THREADS_DIR_NAME)
                .join("RT-0001.md")
                .is_file()
        );
    }

    #[test]
    fn create_bundle_removes_lock_sentinel_after_success() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        let bundle_dir = store.bundle_path("ORB-00000").expect("bundle path");
        let tasks_dir = bundle_dir.parent().expect("bundle parent");

        let created = store
            .create_bundle(&sample_bundle("ORB-00000"))
            .expect("create bundle");

        assert_eq!(created.binding.task_id, "ORB-00000");
        assert!(bundle_dir.is_dir());
        assert_eq!(
            lock_entries_for_task(tasks_dir, "ORB-00000"),
            Vec::<String>::new()
        );
        assert!(!task_lock_path(&bundle_dir).exists());
        assert!(!legacy_double_dot_lock_path(&bundle_dir, "ORB-00000").exists());
    }

    #[derive(Debug, PartialEq, Eq)]
    enum CreateOutcome {
        Created,
        AlreadyExists,
        Unexpected(String),
    }

    #[test]
    fn create_bundle_serializes_concurrent_duplicate_creators() {
        let temp = TempDir::new().expect("tempdir");
        let store = Arc::new(bundle_store(&temp));
        let bundle = Arc::new(sample_bundle("ORB-00000"));
        let bundle_dir = store.bundle_path("ORB-00000").expect("bundle path");
        let tasks_dir = bundle_dir.parent().expect("bundle parent").to_path_buf();
        let barrier = Arc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|_| {
                let store = Arc::clone(&store);
                let bundle = Arc::clone(&bundle);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    match store.create_bundle(bundle.as_ref()) {
                        Ok(_) => CreateOutcome::Created,
                        Err(OrbitError::Store(message)) if message.contains("already exists") => {
                            CreateOutcome::AlreadyExists
                        }
                        Err(err) => CreateOutcome::Unexpected(err.to_string()),
                    }
                })
            })
            .collect::<Vec<_>>();

        let outcomes = handles
            .into_iter()
            .map(|handle| handle.join().expect("join creator"))
            .collect::<Vec<_>>();

        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, CreateOutcome::Created))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, CreateOutcome::AlreadyExists))
                .count(),
            1
        );
        assert!(
            !outcomes
                .iter()
                .any(|outcome| matches!(outcome, CreateOutcome::Unexpected(_))),
            "unexpected outcomes: {outcomes:?}"
        );
        assert!(bundle_dir.is_dir());
        assert_eq!(
            lock_entries_for_task(&tasks_dir, "ORB-00000"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn bundle_store_lists_registered_bundles_from_registry() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        store
            .create_bundle(&sample_bundle("ORB-00000"))
            .expect("create first bundle");
        store
            .create_bundle(&sample_bundle("ORB-00001"))
            .expect("create second bundle");

        let ids: Vec<_> = store
            .list_bundles()
            .expect("list bundles")
            .into_iter()
            .map(|bundle| bundle.envelope.id)
            .collect();
        assert_eq!(ids, vec!["ORB-00000", "ORB-00001"]);
    }

    #[test]
    fn delete_bundle_removes_canonical_projection_and_registry_rows() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        store
            .create_bundle(&sample_bundle("ORB-00000"))
            .expect("create bundle");
        let bundle_dir = store.bundle_path("ORB-00000").expect("bundle path");
        assert!(bundle_dir.is_dir());

        assert!(store.delete_bundle("ORB-00000").expect("delete bundle"));
        assert!(!bundle_dir.exists());
        assert!(!store.workspace_orbit_dir.join("tasks/ORB-00000").exists());
        assert_eq!(
            store
                .registry
                .tasks_for_workspace(&store.workspace_id)
                .expect("registry tasks"),
            Vec::new()
        );
        assert!(matches!(
            store.read_bundle("ORB-00000"),
            Err(OrbitError::NotFound {
                kind: NotFoundKind::Task,
                ..
            })
        ));
        assert!(!store.delete_bundle("ORB-00000").expect("delete missing"));
    }

    #[test]
    fn delete_bundle_unregisters_stale_binding_when_canonical_dir_is_missing() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        store
            .create_bundle(&sample_bundle("ORB-00000"))
            .expect("create bundle");
        let bundle_dir = store.bundle_path("ORB-00000").expect("bundle path");
        fs::remove_dir_all(&bundle_dir).expect("remove canonical bundle");

        assert!(store.delete_bundle("ORB-00000").expect("delete stale"));
        assert!(fs::symlink_metadata(store.workspace_orbit_dir.join("tasks/ORB-00000")).is_err());
        assert_eq!(
            store
                .registry
                .tasks_for_workspace(&store.workspace_id)
                .expect("registry tasks"),
            Vec::new()
        );
    }

    #[test]
    fn rewrite_document_and_append_logs_are_durable() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 12, 30, 0).unwrap();
        store
            .create_bundle(&sample_bundle("ORB-00000"))
            .expect("create bundle");

        store
            .rewrite_document(
                "ORB-00000",
                TaskDocumentV2::Description,
                "New description\n",
            )
            .expect("rewrite description");
        store
            .rewrite_document("ORB-00000", TaskDocumentV2::Acceptance, "- [x] Done\n")
            .expect("rewrite acceptance");
        store
            .rewrite_document("ORB-00000", TaskDocumentV2::Plan, "1. Finish\n")
            .expect("rewrite plan");
        store
            .rewrite_document(
                "ORB-00000",
                TaskDocumentV2::ExecutionSummary,
                "Outcome: success\n",
            )
            .expect("rewrite summary");
        store
            .append_event(
                "ORB-00000",
                &TaskEventRowV2 {
                    schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                    event_id: "EV-0002".to_string(),
                    at: now,
                    by: "codex:gpt-5.5".to_string(),
                    event_type: "updated".to_string(),
                    note: Some("summary written".to_string()),
                    from_status: None,
                    to_status: None,
                },
            )
            .expect("append event");
        store
            .append_comment(
                "ORB-00000",
                &TaskCommentRowV2 {
                    schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                    comment_id: "C-0002".to_string(),
                    at: now,
                    by: "daniel".to_string(),
                    body: "Ship it.".to_string(),
                },
            )
            .expect("append comment");

        let read = store.read_bundle("ORB-00000").expect("read bundle");
        assert_eq!(read.description, "New description\n");
        assert_eq!(read.acceptance, "- [x] Done\n");
        assert_eq!(read.plan, "1. Finish\n");
        assert_eq!(read.execution_summary, "Outcome: success\n");
        assert_eq!(read.events.len(), 2);
        assert_eq!(read.comments.len(), 2);
    }

    #[test]
    fn append_jsonl_repairs_corrupt_tail_only() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        let bundle = sample_bundle("ORB-00000");
        store.create_bundle(&bundle).expect("create bundle");
        let events_path = store
            .bundle_path("ORB-00000")
            .expect("bundle path")
            .join(TASK_EVENTS_FILE_NAME);
        fs::write(&events_path, "{\"schema_version\":1,\"event_id\":\"EV-0001\",\"at\":\"2026-05-11T12:00:00Z\",\"by\":\"codex:gpt-5.5\",\"type\":\"created\",\"to_status\":\"backlog\"}\n{\"schema_version\"")
            .expect("write corrupt tail");

        store
            .append_event(
                "ORB-00000",
                &TaskEventRowV2 {
                    schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                    event_id: "EV-0002".to_string(),
                    at: Utc.with_ymd_and_hms(2026, 5, 11, 13, 0, 0).unwrap(),
                    by: "codex:gpt-5.5".to_string(),
                    event_type: "updated".to_string(),
                    note: None,
                    from_status: None,
                    to_status: None,
                },
            )
            .expect("append event");

        let events = read_task_events(&events_path).expect("read events");
        assert_eq!(
            events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["EV-0001", "EV-0002"]
        );
    }

    #[test]
    fn append_jsonl_repairs_trailing_newline_corrupt_tail() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        store
            .create_bundle(&sample_bundle("ORB-00000"))
            .expect("create bundle");
        let events_path = store
            .bundle_path("ORB-00000")
            .expect("bundle path")
            .join(TASK_EVENTS_FILE_NAME);
        fs::write(&events_path, "{\"schema_version\":1,\"event_id\":\"EV-0001\",\"at\":\"2026-05-11T12:00:00Z\",\"by\":\"codex:gpt-5.5\",\"type\":\"created\",\"to_status\":\"backlog\"}\nnot-json\n")
            .expect("write corrupt tail");

        store
            .append_event(
                "ORB-00000",
                &TaskEventRowV2 {
                    schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                    event_id: "EV-0002".to_string(),
                    at: Utc.with_ymd_and_hms(2026, 5, 11, 13, 0, 0).unwrap(),
                    by: "codex:gpt-5.5".to_string(),
                    event_type: "updated".to_string(),
                    note: None,
                    from_status: None,
                    to_status: None,
                },
            )
            .expect("append event");

        let events = read_task_events(&events_path).expect("read events");
        assert_eq!(
            events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["EV-0001", "EV-0002"]
        );
    }

    #[test]
    fn append_jsonl_serializes_concurrent_writers() {
        let temp = TempDir::new().expect("tempdir");
        let path = Arc::new(temp.path().join("events.jsonl"));
        let barrier = Arc::new(Barrier::new(8));
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 13, 0, 0).unwrap();
        let handles = (0..8)
            .map(|index| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    append_jsonl_row(
                        &path,
                        &TaskEventRowV2 {
                            schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
                            event_id: format!("EV-{index:04}"),
                            at: now,
                            by: "codex:gpt-5.5".to_string(),
                            event_type: "updated".to_string(),
                            note: None,
                            from_status: None,
                            to_status: None,
                        },
                    )
                    .expect("append event");
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle.join().expect("join writer");
        }

        let mut ids = read_task_events(&path)
            .expect("read events")
            .into_iter()
            .map(|event| event.event_id)
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(
            ids,
            vec![
                "EV-0000", "EV-0001", "EV-0002", "EV-0003", "EV-0004", "EV-0005", "EV-0006",
                "EV-0007"
            ]
        );
    }

    #[test]
    fn read_jsonl_rejects_corruption_before_tail() {
        let temp = TempDir::new().expect("tempdir");
        let path = temp.path().join("events.jsonl");
        fs::write(
            &path,
            "{\"schema_version\":1,\"event_id\":\"EV-0001\",\"at\":\"2026-05-11T12:00:00Z\",\"by\":\"codex:gpt-5.5\",\"type\":\"created\",\"to_status\":\"backlog\"}\nnot-json\n{\"schema_version\":1,\"event_id\":\"EV-0002\",\"at\":\"2026-05-11T13:00:00Z\",\"by\":\"codex:gpt-5.5\",\"type\":\"updated\"}\n",
        )
        .expect("write invalid middle");

        assert!(matches!(
            read_task_events(&path),
            Err(OrbitError::Store(message)) if message.contains("before tail")
        ));
    }

    #[test]
    fn create_bundle_cleans_partial_directory_and_lock_on_validation_error() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        let mut bundle = sample_bundle("ORB-00000");
        bundle.envelope.title = " ".to_string();
        let bundle_path = store.bundle_path("ORB-00000").expect("bundle path");
        let tasks_dir = bundle_path.parent().expect("bundle parent").to_path_buf();

        assert!(store.create_bundle(&bundle).is_err());
        assert!(!bundle_path.exists());
        assert_eq!(
            lock_entries_for_task(&tasks_dir, "ORB-00000"),
            Vec::<String>::new()
        );
        assert!(!task_lock_path(&bundle_path).exists());
        assert!(!legacy_double_dot_lock_path(&bundle_path, "ORB-00000").exists());
    }

    #[test]
    fn create_bundle_treats_projection_error_as_degraded_success() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        let projection_dir = store.workspace_orbit_dir.join("tasks");
        fs::create_dir_all(&projection_dir).expect("create projection dir");
        fs::write(projection_dir.join("ORB-00000"), "not a symlink").expect("write blocker");

        let created = store
            .create_bundle(&sample_bundle("ORB-00000"))
            .expect("create bundle");

        assert_eq!(created.binding.task_id, "ORB-00000");
        assert!(created.projection.degraded_reason.is_some());
        assert!(store.read_bundle("ORB-00000").is_ok());
        assert_eq!(
            store
                .list_bundles()
                .expect("list bundles")
                .into_iter()
                .map(|bundle| bundle.envelope.id)
                .collect::<Vec<_>>(),
            vec!["ORB-00000"]
        );
    }

    #[test]
    fn read_bundle_rejects_directory_name_that_differs_from_task_id() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        let created = store
            .create_bundle(&sample_bundle("ORB-00000"))
            .expect("create bundle");
        let renamed = created.binding.canonical_path.with_file_name("ORB-00009");
        fs::rename(&created.binding.canonical_path, &renamed).expect("rename bundle");

        assert!(matches!(
            read_bundle_at(&renamed),
            Err(OrbitError::Store(message)) if message.contains("does not match task id")
        ));
    }

    #[test]
    fn read_bundle_reports_missing_envelope_as_task_not_found() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        let created = store
            .create_bundle(&sample_bundle("ORB-00000"))
            .expect("create bundle");
        fs::remove_file(created.binding.canonical_path.join(TASK_ENVELOPE_FILE_NAME))
            .expect("remove envelope");

        assert!(matches!(
            store.read_bundle("ORB-00000"),
            Err(OrbitError::NotFound {
                kind: NotFoundKind::Task,
                id: task_id,
            }) if task_id == "ORB-00000"
        ));
    }

    #[test]
    fn read_bundle_rejects_review_thread_metadata_without_body() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        let mut bundle = sample_bundle("ORB-00000");
        bundle.review_threads = sample_review_threads();
        let created = store.create_bundle(&bundle).expect("create bundle");
        fs::remove_file(
            created
                .binding
                .canonical_path
                .join(TASK_REVIEW_THREADS_DIR_NAME)
                .join("RT-0001.md"),
        )
        .expect("remove thread body");

        assert!(matches!(
            store.read_bundle("ORB-00000"),
            Err(OrbitError::Store(message)) if message.contains("missing task bundle file")
        ));
    }

    #[test]
    fn read_bundle_rejects_manifest_entry_with_missing_artifact_file() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        store
            .create_bundle(&sample_bundle("ORB-00000"))
            .expect("create bundle");
        let now = Utc.with_ymd_and_hms(2026, 5, 11, 12, 0, 0).unwrap();
        let bundle_dir = store.bundle_path("ORB-00000").expect("bundle path");
        let blob = format!("{TASK_ARTIFACT_FILES_DIR_NAME}/result.txt");
        let blob_path = bundle_dir.join(TASK_ARTIFACTS_DIR_NAME).join(&blob);
        atomic_write_text(&blob_path, "hello").expect("write artifact blob");
        let manifest = ArtifactManifestV2 {
            schema_version: TASK_ARTIFACT_SCHEMA_VERSION,
            files: vec![ArtifactManifestFileV2 {
                path: "result.txt".to_string(),
                blob: blob.clone(),
                sha256: format!("{:x}", Sha256::digest(b"hello")),
                media_type: "text/plain".to_string(),
                size_bytes: 5,
                created_by: "codex:gpt-5.5".to_string(),
                created_at: now,
            }],
        };
        store
            .rewrite_artifact_manifest("ORB-00000", &manifest)
            .expect("write manifest");
        fs::remove_file(blob_path).expect("remove artifact blob");

        assert!(matches!(
            store.read_bundle("ORB-00000"),
            Err(OrbitError::Store(message)) if message.contains("missing file")
        ));
    }

    #[test]
    fn read_bundle_rejects_event_status_newer_than_envelope_status() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        store
            .create_bundle(&sample_bundle("ORB-00000"))
            .expect("create bundle");
        let mut envelope = sample_bundle("ORB-00000").envelope;
        envelope.status = TaskStatus::InProgress;
        store
            .rewrite_envelope("ORB-00000", &envelope)
            .expect("rewrite mismatched envelope");

        assert!(matches!(
            store.read_bundle("ORB-00000"),
            Err(OrbitError::Store(message)) if message.contains("event log status")
        ));
    }

    #[test]
    fn rewrite_review_threads_validates_before_touching_existing_files() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        let mut bundle = sample_bundle("ORB-00000");
        bundle.review_threads = sample_review_threads();
        store.create_bundle(&bundle).expect("create bundle");
        let mut invalid = sample_review_threads();
        invalid[0].metadata.path = Some("../escape.rs".to_string());

        assert!(matches!(
            store.rewrite_review_threads("ORB-00000", &invalid),
            Err(OrbitError::InvalidInput(message)) if message.contains("..")
        ));

        let read = store.read_bundle("ORB-00000").expect("read bundle");
        assert_eq!(
            read.review_threads
                .into_iter()
                .map(|thread| thread.metadata.thread_id)
                .collect::<Vec<_>>(),
            vec!["RT-0001", "RT-0002"]
        );
    }

    #[test]
    fn read_review_threads_filters_tombstoned_partial_rewrite_orphans() {
        let temp = TempDir::new().expect("tempdir");
        let store = bundle_store(&temp);
        let mut bundle = sample_bundle("ORB-00000");
        bundle.review_threads = sample_review_threads();
        store.create_bundle(&bundle).expect("create bundle");
        let thread_dir = store
            .bundle_path("ORB-00000")
            .expect("bundle path")
            .join(TASK_REVIEW_THREADS_DIR_NAME);
        atomic_write_text(&thread_dir.join(REVIEW_THREAD_TOMBSTONES_FILE), "RT-0001\n")
            .expect("write tombstones");

        let read = store.read_bundle("ORB-00000").expect("read bundle");
        assert_eq!(
            read.review_threads
                .into_iter()
                .map(|thread| thread.metadata.thread_id)
                .collect::<Vec<_>>(),
            vec!["RT-0002"]
        );
    }
}
