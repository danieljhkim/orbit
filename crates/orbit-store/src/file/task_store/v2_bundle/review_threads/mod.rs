use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use orbit_common::types::{OrbitError, ReviewThreadMetadataV2, TASK_REVIEW_THREADS_DIR_NAME};
use orbit_common::utility::fs::atomic_write_text;

use super::bundle_io::{read_required_text, read_yaml_file, write_yaml_file};
use super::task_bundle_types::TaskReviewThreadV2;

pub(crate) const REVIEW_THREAD_TOMBSTONES_FILE: &str = ".tombstones";

pub(crate) fn write_review_threads(
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

pub(crate) fn rewrite_review_threads(
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
        .collect::<BTreeSet<_>>();
    let expected_ids = threads
        .iter()
        .map(|thread| thread.metadata.thread_id.clone())
        .collect::<BTreeSet<_>>();
    let stale_ids = review_thread_ids_on_disk(&thread_dir)?
        .into_iter()
        .filter(|thread_id| !expected_ids.contains(thread_id))
        .collect::<BTreeSet<_>>();
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

pub(crate) fn read_review_threads(
    bundle_dir: &Path,
) -> Result<Vec<TaskReviewThreadV2>, OrbitError> {
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

fn review_thread_ids_on_disk(thread_dir: &Path) -> Result<BTreeSet<String>, OrbitError> {
    let mut ids = BTreeSet::new();
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
    stale_ids: &BTreeSet<String>,
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

fn read_review_thread_tombstones(thread_dir: &Path) -> Result<BTreeSet<String>, OrbitError> {
    let path = thread_dir.join(REVIEW_THREAD_TOMBSTONES_FILE);
    match fs::read_to_string(&path) {
        Ok(raw) => Ok(raw
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(err) => Err(OrbitError::Io(err.to_string())),
    }
}

#[cfg(test)]
mod tests;
