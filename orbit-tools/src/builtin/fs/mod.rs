pub mod delete;
pub mod list;
pub mod read;
pub mod write;

use std::path::{Path, PathBuf};

use orbit_types::OrbitError;

use crate::{ToolContext, ToolRegistry};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(read::FsReadTool);
    registry.register(write::FsWriteTool);
    registry.register(delete::FsDeleteTool);
    registry.register(list::FsListTool);
}

/// Checks that `path` resolves inside the context workspace root.
///
/// Symlink escapes are blocked because the path is canonicalized before the check.
/// For paths that do not yet exist (e.g. `fs.write` creating a new file), the
/// nearest existing ancestor is canonicalized and the remaining components are
/// appended before the check.
///
/// Returns `Err(PolicyDenied)` when no workspace root is set (fail-closed) or
/// when the canonical path is outside the root. Returns `Ok` only when the
/// canonical path is inside the workspace root.
pub(super) fn check_workspace_boundary(
    ctx: &ToolContext,
    path: &Path,
) -> Result<PathBuf, OrbitError> {
    let workspace_root = match &ctx.workspace_root {
        Some(root) => root,
        None => {
            return Err(OrbitError::PolicyDenied(
                "workspace_root is not set; filesystem access denied".to_string(),
            ))
        }
    };

    let canonical = if path.exists() {
        path.canonicalize()
            .map_err(|e| OrbitError::Io(format!("failed to canonicalize path: {e}")))?
    } else {
        // Path does not exist yet (e.g. write target). Canonicalize the parent
        // so we can still enforce the boundary.
        let parent = path
            .parent()
            .ok_or_else(|| OrbitError::InvalidInput("path has no parent directory".to_string()))?;
        let canonical_parent = parent
            .canonicalize()
            .map_err(|e| OrbitError::Io(format!("failed to canonicalize parent directory: {e}")))?;
        let file_name = path
            .file_name()
            .ok_or_else(|| OrbitError::InvalidInput("path has no file name".to_string()))?;
        canonical_parent.join(file_name)
    };

    // Canonicalize the workspace root so symlinks (e.g. /var -> /private/var on
    // macOS) don't cause false negatives when comparing against the canonical path.
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.clone());

    if !canonical.starts_with(&canonical_root) {
        return Err(OrbitError::PolicyDenied(format!(
            "path is outside workspace: {}",
            canonical.display()
        )));
    }

    Ok(canonical)
}
