pub mod copy;
pub mod delete;
pub mod mkdir;
pub mod move_file;
pub mod patch;
pub mod write;

use std::path::{Path, PathBuf};

use orbit_types::OrbitError;

use crate::{ToolContext, ToolRegistry};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(copy::FsCopyTool);
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

pub(crate) fn check_file_lock(ctx: &ToolContext, canonical_path: &Path) -> Result<(), OrbitError> {
    let Some(task_id) = ctx.task_id.as_deref() else {
        return Ok(());
    };
    let Some(checker) = ctx.file_lock_checker.as_ref() else {
        return Ok(());
    };
    let workspace_root = ctx.workspace_root.as_ref().ok_or_else(|| {
        OrbitError::PolicyDenied("workspace_root is not set; filesystem access denied".to_string())
    })?;
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.clone());
    let repo_root = canonical_root.to_string_lossy().into_owned();
    let relative_path = canonical_path
        .strip_prefix(&canonical_root)
        .map_err(|error| {
            OrbitError::PolicyDenied(format!(
                "path '{}' is outside workspace root '{}': {error}",
                canonical_path.display(),
                canonical_root.display()
            ))
        })?
        .to_string_lossy()
        .into_owned();

    checker.check_write_allowed(Some(task_id), &repo_root, &relative_path)?;
    checker.auto_acquire(task_id, &repo_root, &relative_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use orbit_lock::{FileLockChecker, FileLockStore, apply_lock_schema};
    use orbit_types::OrbitError;
    use rusqlite::Connection;

    use crate::ToolContext;

    use super::{check_file_lock, check_workspace_boundary};

    fn lock_store() -> Arc<FileLockStore> {
        let conn = Connection::open_in_memory().expect("sqlite");
        apply_lock_schema(&conn).expect("schema");
        Arc::new(FileLockStore::new(Arc::new(Mutex::new(conn))))
    }

    #[test]
    fn file_lock_uses_workspace_relative_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_root = dir.path().join("repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("mkdir");
        let file = repo_root.join("src/lib.rs");
        std::fs::write(&file, "old").expect("write");

        let canonical = check_workspace_boundary(
            &ToolContext {
                workspace_root: Some(repo_root.clone()),
                ..Default::default()
            },
            Path::new(&file),
        )
        .expect("boundary");

        let ctx = ToolContext {
            workspace_root: Some(repo_root.clone()),
            task_id: Some("T1".to_string()),
            file_lock_checker: Some(lock_store()),
            ..Default::default()
        };

        check_file_lock(&ctx, &canonical).expect("lock");
        let holder = ctx.file_lock_checker.as_ref().expect("checker").as_ref();
        holder
            .check_write_allowed(Some("T1"), &repo_root.to_string_lossy(), "src/lib.rs")
            .expect("same task");
    }

    #[test]
    fn file_lock_denies_other_task() {
        let repo_root = PathBuf::from("/repo");
        let store = lock_store();
        store
            .auto_acquire("T1", &repo_root.to_string_lossy(), "src/lib.rs")
            .expect("acquire");

        let err = check_file_lock(
            &ToolContext {
                workspace_root: Some(repo_root.clone()),
                task_id: Some("T2".to_string()),
                file_lock_checker: Some(store),
                ..Default::default()
            },
            Path::new("/repo/src/lib.rs"),
        )
        .expect_err("other task should be denied");
        assert!(matches!(err, OrbitError::PolicyDenied(_)));
    }

    #[test]
    fn boundary_check_supports_nested_nonexistent_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested/path/file.txt");

        let canonical = check_workspace_boundary(
            &ToolContext {
                workspace_root: Some(dir.path().to_path_buf()),
                ..Default::default()
            },
            &path,
        )
        .expect("boundary");

        let expected = dir
            .path()
            .canonicalize()
            .expect("canonical root")
            .join("nested/path/file.txt");
        assert_eq!(canonical, expected);
    }
}
