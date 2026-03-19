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

/// Checks that `path` resolves inside the context workspace root (if one is set).
///
/// Symlink escapes are blocked because the path is canonicalized before the check.
/// For paths that do not yet exist (e.g. `fs.write` creating a new file), the
/// nearest existing ancestor is canonicalized and the remaining components are
/// appended before the check.
///
/// Returns `Ok` when no workspace root is set, or when the canonical path is
/// inside the root. Returns `Err(PolicyDenied)` otherwise.
pub(super) fn check_workspace_boundary(ctx: &ToolContext, path: &Path) -> Result<PathBuf, OrbitError> {
    let workspace_root = match &ctx.workspace_root {
        Some(root) => root,
        None => return Ok(path.to_path_buf()),
    };

    let canonical = if path.exists() {
        path.canonicalize()
            .map_err(|e| OrbitError::Io(format!("failed to canonicalize path: {e}")))?
    } else {
        // Path does not exist yet (e.g. write target). Canonicalize the parent
        // so we can still enforce the boundary.
        let parent = path.parent().ok_or_else(|| {
            OrbitError::InvalidInput("path has no parent directory".to_string())
        })?;
        let canonical_parent = parent
            .canonicalize()
            .map_err(|e| OrbitError::Io(format!("failed to canonicalize parent directory: {e}")))?;
        let file_name = path.file_name().ok_or_else(|| {
            OrbitError::InvalidInput("path has no file name".to_string())
        })?;
        canonical_parent.join(file_name)
    };

    if !canonical.starts_with(workspace_root) {
        return Err(OrbitError::PolicyDenied(format!(
            "path is outside workspace: {}",
            canonical.display()
        )));
    }

    Ok(canonical)
}
