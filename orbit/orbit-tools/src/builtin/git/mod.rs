pub mod commit;
pub mod push;
pub mod stage_paths;

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use orbit_types::OrbitError;
use serde_json::Value;

use crate::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(stage_paths::GitStagePathsTool);
    registry.register(commit::GitCommitTool);
    registry.register(push::GitPushTool);
}

fn require_repo_root(input: &Value) -> Result<PathBuf, OrbitError> {
    let raw = input
        .get("repo_root")
        .and_then(Value::as_str)
        .ok_or_else(|| OrbitError::InvalidInput("missing `repo_root`".to_string()))?;

    let path = PathBuf::from(raw);
    if !path.exists() {
        return Err(OrbitError::InvalidInput(format!(
            "repo_root does not exist: {raw}"
        )));
    }
    if !path.is_dir() {
        return Err(OrbitError::InvalidInput(format!(
            "repo_root is not a directory: {raw}"
        )));
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| OrbitError::InvalidInput(format!("failed to canonicalize repo_root: {e}")))?;
    if !canonical.join(".git").exists() {
        return Err(OrbitError::InvalidInput(format!(
            "repo_root is not a git repository: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn require_relative_file_paths(input: &Value, repo_root: &Path) -> Result<Vec<String>, OrbitError> {
    let files = input
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| OrbitError::InvalidInput("missing `files`".to_string()))?;
    if files.is_empty() {
        return Err(OrbitError::InvalidInput(
            "`files` must contain at least one path".to_string(),
        ));
    }

    let mut seen = HashSet::new();
    let mut relative_paths = Vec::new();
    for value in files {
        let raw = value
            .as_str()
            .ok_or_else(|| OrbitError::InvalidInput("`files` entries must be strings".to_string()))?
            .trim();
        if raw.is_empty() {
            return Err(OrbitError::InvalidInput(
                "`files` must not contain empty paths".to_string(),
            ));
        }

        let candidate = Path::new(raw);
        let absolute = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            repo_root.join(candidate)
        };

        if !absolute.exists() {
            return Err(OrbitError::InvalidInput(format!(
                "file does not exist: {}",
                absolute.display()
            )));
        }
        let canonical = absolute.canonicalize().map_err(|e| {
            OrbitError::InvalidInput(format!(
                "failed to canonicalize file '{}': {e}",
                absolute.display()
            ))
        })?;
        if !canonical.starts_with(repo_root) {
            return Err(OrbitError::InvalidInput(format!(
                "file is outside repo_root: {}",
                canonical.display()
            )));
        }
        if !canonical.is_file() {
            return Err(OrbitError::InvalidInput(format!(
                "path is not a file: {}",
                canonical.display()
            )));
        }

        let relative = canonical.strip_prefix(repo_root).map_err(|e| {
            OrbitError::InvalidInput(format!("failed to relativize file path: {e}"))
        })?;
        if matches!(
            relative.components().next(),
            Some(Component::Normal(component)) if component == ".git"
        ) {
            return Err(OrbitError::InvalidInput(
                "paths inside .git are not allowed".to_string(),
            ));
        }

        let normalized = relative.to_string_lossy().replace('\\', "/");
        if seen.insert(normalized.clone()) {
            relative_paths.push(normalized);
        }
    }

    Ok(relative_paths)
}
