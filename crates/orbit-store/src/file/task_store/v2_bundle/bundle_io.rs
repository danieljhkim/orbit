use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use orbit_common::migration::Plan;
use orbit_common::types::{
    ArtifactManifestV2, NotFoundKind, OrbitError, TASK_ACCEPTANCE_FILE_NAME,
    TASK_ARTIFACT_FILES_DIR_NAME, TASK_ARTIFACT_MANIFEST_FILE_NAME, TASK_ARTIFACTS_DIR_NAME,
    TASK_COMMENTS_FILE_NAME, TASK_DESCRIPTION_FILE_NAME, TASK_ENVELOPE_FILE_NAME,
    TASK_EVENTS_FILE_NAME, TASK_EXECUTION_SUMMARY_FILE_NAME, TASK_PLAN_FILE_NAME,
    TASK_REVIEW_THREADS_DIR_NAME, TaskCommentRowV2, TaskEnvelopeV2, TaskEventRowV2,
};
use orbit_common::utility::fs::{atomic_write_text, with_exclusive_file_lock};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use super::review_threads::{read_review_threads, write_review_threads};
use super::task_bundle_types::TaskBundleV2;
use crate::file::task_store::task_migrations;

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

pub(crate) fn write_yaml_file<T>(path: &Path, value: &T) -> Result<(), OrbitError>
where
    T: serde::Serialize,
{
    let yaml = serde_yaml::to_string(value).map_err(|err| OrbitError::Store(err.to_string()))?;
    atomic_write_text(path, &yaml).map_err(|err| OrbitError::Io(err.to_string()))
}

pub(crate) fn read_yaml_file<T>(path: &Path) -> Result<T, OrbitError>
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

pub(crate) fn read_required_text(path: &Path) -> Result<String, OrbitError> {
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

pub(crate) fn append_jsonl_row<T>(path: &Path, row: &T) -> Result<(), OrbitError>
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

pub(crate) fn cleanup_partial_bundle_best_effort(
    bundle_dir: &Path,
    phase: &str,
    original: &OrbitError,
) {
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
#[path = "bundle_io_tests.rs"]
mod tests;
