use std::fs;
use std::path::{Component, Path};

use orbit_types::{OrbitError, TaskArtifact};

use super::{TaskFileStore, bundle::bundle_read_error};

impl TaskFileStore {
    pub(super) fn write_artifacts_at(
        &self,
        task_dir: &Path,
        artifacts: &[TaskArtifact],
    ) -> Result<(), OrbitError> {
        let artifacts_dir = self.artifacts_dir(task_dir);
        fs::create_dir_all(&artifacts_dir).map_err(|e| OrbitError::Io(e.to_string()))?;
        for artifact in artifacts {
            let relative_path = normalize_artifact_path(&artifact.path)?;
            let destination = artifacts_dir.join(&relative_path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|e| OrbitError::Io(e.to_string()))?;
            }
            write_atomic(&destination, &artifact.content)?;
        }
        Ok(())
    }

    pub(super) fn read_artifacts_at(
        &self,
        task_dir: &Path,
    ) -> Result<Vec<TaskArtifact>, OrbitError> {
        let artifacts_dir = self.artifacts_dir(task_dir);
        if !artifacts_dir.exists() {
            return Ok(Vec::new());
        }

        let mut artifacts = Vec::new();
        collect_artifacts(&artifacts_dir, &artifacts_dir, &mut artifacts)?;
        artifacts.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(artifacts)
    }
}

use crate::file::fs_utils::write_atomic;

fn normalize_artifact_path(raw: &str) -> Result<String, OrbitError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(OrbitError::InvalidInput(
            "task artifact path must not be empty".to_string(),
        ));
    }

    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err(OrbitError::InvalidInput(format!(
            "task artifact path must be relative: {trimmed}"
        )));
    }

    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_str().ok_or_else(|| {
                    OrbitError::InvalidInput(format!(
                        "task artifact path must be valid UTF-8: {trimmed}"
                    ))
                })?;
                if part.is_empty() {
                    return Err(OrbitError::InvalidInput(format!(
                        "task artifact path must not contain empty segments: {trimmed}"
                    )));
                }
                parts.push(part.to_string());
            }
            _ => {
                return Err(OrbitError::InvalidInput(format!(
                    "task artifact path must stay within the task artifacts directory: {trimmed}"
                )));
            }
        }
    }

    if parts.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "task artifact path must contain a file name: {trimmed}"
        )));
    }

    Ok(parts.join("/"))
}

fn collect_artifacts(
    root: &Path,
    dir: &Path,
    artifacts: &mut Vec<TaskArtifact>,
) -> Result<(), OrbitError> {
    let mut entries = fs::read_dir(dir)
        .map_err(|e| OrbitError::Io(e.to_string()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort();

    for entry in entries {
        if entry.is_dir() {
            collect_artifacts(root, &entry, artifacts)?;
            continue;
        }
        if !entry.is_file() {
            continue;
        }
        let relative = entry
            .strip_prefix(root)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let relative = normalize_artifact_path(&relative.to_string_lossy())?;
        let content = fs::read_to_string(&entry)
            .map_err(|e| bundle_read_error(&entry, "task artifact", e))?;
        artifacts.push(TaskArtifact {
            path: relative,
            content,
        });
    }

    Ok(())
}
