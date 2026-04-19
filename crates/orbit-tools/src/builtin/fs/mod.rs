pub mod copy;
pub mod delete;
pub mod ls;
pub mod mkdir;
pub mod move_file;
pub mod patch;
pub mod read;
pub mod write;

use std::path::{Path, PathBuf};

use orbit_types::OrbitError;

use crate::{ToolContext, ToolRegistry};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(copy::FsCopyTool);
    registry.register(read::FsReadTool);
    registry.register(ls::FsLsTool);
    registry.register(write::FsWriteTool);
    registry.register(delete::FsDeleteTool);
    registry.register(move_file::FsMoveTool);
    registry.register(mkdir::FsMkdirTool);
    registry.register(patch::FsPatchTool);
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
pub(crate) fn check_workspace_boundary(
    ctx: &ToolContext,
    path: &Path,
) -> Result<PathBuf, OrbitError> {
    let workspace_root = match &ctx.workspace_root {
        Some(root) => root,
        None => {
            return Err(OrbitError::PolicyDenied(
                "workspace_root is not set; filesystem access denied".to_string(),
            ));
        }
    };

    let canonical = canonicalize_with_missing_tail(path)?;

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

fn canonicalize_with_missing_tail(path: &Path) -> Result<PathBuf, OrbitError> {
    if path.exists() {
        return path
            .canonicalize()
            .map_err(|e| OrbitError::Io(format!("failed to canonicalize path: {e}")));
    }

    let mut missing_components = Vec::new();
    let mut existing_ancestor = path;
    while !existing_ancestor.exists() {
        let name = existing_ancestor
            .file_name()
            .ok_or_else(|| OrbitError::InvalidInput("path has no file name".to_string()))?;
        missing_components.push(name.to_os_string());
        existing_ancestor = existing_ancestor.parent().ok_or_else(|| {
            OrbitError::InvalidInput("path has no existing parent directory".to_string())
        })?;
    }

    let mut canonical = existing_ancestor
        .canonicalize()
        .map_err(|e| OrbitError::Io(format!("failed to canonicalize parent directory: {e}")))?;
    for component in missing_components.iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

pub(crate) fn check_file_lock(
    _ctx: &ToolContext,
    _canonical_path: &Path,
) -> Result<(), OrbitError> {
    // File-level locking removed; graph-level locking is handled by
    // the shared lock store at .orbit/knowledge/graph_locks.json.
    Ok(())
}
