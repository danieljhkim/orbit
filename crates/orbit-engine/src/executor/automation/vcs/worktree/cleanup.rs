use std::path::Path;

use orbit_common::types::OrbitError;

use super::super::git::git_success;

/// Shared sanctioned worktree removal sequence. Pipeline cleanup may force
/// removal because it owns the just-created checkout; out-of-band GC must
/// always pass `force = false`.
pub(super) fn remove_worktree(
    repo_root: &Path,
    workspace_path: &Path,
    branch_name: Option<&str>,
    force: bool,
) -> Result<(), OrbitError> {
    if workspace_path.exists() {
        let workspace_path = workspace_path.to_string_lossy();
        if force {
            git_success(
                repo_root,
                &["worktree", "remove", "--force", workspace_path.as_ref()],
            )?;
        } else {
            git_success(repo_root, &["worktree", "remove", workspace_path.as_ref()])?;
        }
    }
    git_success(repo_root, &["worktree", "prune"])?;
    if let Some(branch_name) = branch_name {
        git_success(repo_root, &["branch", "-D", branch_name])?;
    }
    Ok(())
}
