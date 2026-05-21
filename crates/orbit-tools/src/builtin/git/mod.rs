pub mod commit;
pub mod push;
pub mod stage_paths;

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use orbit_common::types::{OrbitError, optional_string_list_alias};
use serde_json::Value;

use crate::{ToolContext, ToolRegistry};

pub fn register(registry: &mut ToolRegistry) {
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

pub(super) fn require_workspace_repo_root(
    ctx: &ToolContext,
    input: &Value,
) -> Result<PathBuf, OrbitError> {
    let repo_root = require_repo_root(input)?;
    let workspace_root = ctx.workspace_root.as_ref().ok_or_else(|| {
        OrbitError::InvalidInput("workspace_root is required for git tools".to_string())
    })?;
    let canonical_workspace_root = workspace_root.canonicalize().map_err(|e| {
        OrbitError::InvalidInput(format!("failed to canonicalize workspace_root: {e}"))
    })?;

    if !repo_root.starts_with(&canonical_workspace_root) {
        return Err(OrbitError::InvalidInput(format!(
            "repo_root is outside workspace_root: repo_root={}, workspace_root={}",
            repo_root.display(),
            canonical_workspace_root.display()
        )));
    }

    Ok(repo_root)
}

fn require_relative_file_paths(input: &Value, repo_root: &Path) -> Result<Vec<String>, OrbitError> {
    let files = optional_string_list_alias(input, &["files"])?
        .ok_or_else(|| OrbitError::InvalidInput("missing `files`".to_string()))?;
    if files.is_empty() {
        return Err(OrbitError::InvalidInput(
            "`files` must contain at least one path".to_string(),
        ));
    }

    let mut seen = HashSet::new();
    let mut relative_paths = Vec::new();
    for raw in files {
        let candidate = Path::new(&raw);
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

#[cfg(test)]
mod tests;
