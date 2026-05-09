use std::path::{Path, PathBuf};

use chrono::Utc;
use orbit_common::types::OrbitError;
use serde::{Deserialize, Serialize};

use crate::executor::automation::vcs::git::{
    git_command_success, git_output, git_output_paths, git_success,
};

/// Scratch-branch metadata for a single Groundhog day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSnapshotRef {
    task_id: String,
    day_n: u32,
    workspace_path: PathBuf,
    task_branch: String,
    scratch_branch: String,
    snapshot_ref: String,
    preserved_untracked_paths: Vec<String>,
}

impl WorkspaceSnapshotRef {
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn day_n(&self) -> u32 {
        self.day_n
    }

    pub fn workspace_path(&self) -> &Path {
        &self.workspace_path
    }

    pub fn task_branch(&self) -> &str {
        &self.task_branch
    }

    pub fn scratch_branch(&self) -> &str {
        &self.scratch_branch
    }

    pub fn snapshot_ref(&self) -> &str {
        &self.snapshot_ref
    }
}

/// Git-backed workspace snapshots for Groundhog day execution.
///
/// Callers are expected to start from a clean task branch with no tracked
/// changes. Existing untracked files are preserved and excluded from the
/// scratch-branch capture. Groundhog day execution also assumes exclusive
/// logical ownership of the task branch: if the task branch head changes away
/// from the saved snapshot during the day, `rewind` and `commit_success`
/// abort and require manual intervention instead of clobbering the newer
/// commits.
pub struct WorkspaceSnapshot;

impl WorkspaceSnapshot {
    pub fn create(
        task_id: &str,
        day_n: u32,
        workspace_path: impl AsRef<Path>,
    ) -> Result<WorkspaceSnapshotRef, OrbitError> {
        let task_id = task_id.trim();
        if task_id.is_empty() {
            return Err(OrbitError::InvalidInput(
                "task_id cannot be blank".to_string(),
            ));
        }
        if day_n == 0 {
            return Err(OrbitError::InvalidInput(
                "day_n must be greater than zero".to_string(),
            ));
        }

        let workspace_path = canonicalize_workspace_path(workspace_path.as_ref())?;
        let task_branch = current_branch(&workspace_path)?;
        ensure_clean_tracked_workspace(&workspace_path)?;
        let scratch_branch = scratch_branch_name(task_id, day_n);
        ensure_scratch_branch_absent(&workspace_path, &scratch_branch)?;

        let snapshot_ref = git_output(&workspace_path, &["rev-parse", "HEAD"])?;
        let preserved_untracked_paths = list_untracked_paths(&workspace_path)?;
        git_success(&workspace_path, &["checkout", "-b", &scratch_branch])?;

        Ok(WorkspaceSnapshotRef {
            task_id: task_id.to_string(),
            day_n,
            workspace_path,
            task_branch,
            scratch_branch,
            snapshot_ref,
            preserved_untracked_paths,
        })
    }

    pub fn rewind(snapshot: &WorkspaceSnapshotRef) -> Result<(), OrbitError> {
        match current_branch(snapshot.workspace_path())?.as_str() {
            branch if branch == snapshot.scratch_branch() => {
                capture_scratch_branch_changes(snapshot)?;
                ensure_task_branch_still_at_snapshot(snapshot)?;
                checkout_branch(snapshot.workspace_path(), snapshot.task_branch())?;
            }
            branch if branch == snapshot.task_branch() => {}
            branch => {
                return Err(OrbitError::Execution(format!(
                    "workspace '{}' is on branch '{branch}', expected '{}' or '{}' to rewind Groundhog snapshot",
                    snapshot.workspace_path().display(),
                    snapshot.task_branch(),
                    snapshot.scratch_branch()
                )));
            }
        }

        git_success(
            snapshot.workspace_path(),
            &["reset", "--hard", snapshot.snapshot_ref()],
        )
    }

    pub fn commit_success(
        snapshot: &WorkspaceSnapshotRef,
        summary: &str,
    ) -> Result<(), OrbitError> {
        let summary = summary.trim();
        if summary.is_empty() {
            return Err(OrbitError::InvalidInput(
                "commit_success summary cannot be blank".to_string(),
            ));
        }

        let current_branch = current_branch(snapshot.workspace_path())?;
        if current_branch != snapshot.scratch_branch() {
            return Err(OrbitError::Execution(format!(
                "workspace '{}' is on branch '{current_branch}', expected '{}' before committing Groundhog success",
                snapshot.workspace_path().display(),
                snapshot.scratch_branch()
            )));
        }

        capture_scratch_branch_changes(snapshot)?;
        ensure_task_branch_still_at_snapshot(snapshot)?;
        checkout_branch(snapshot.workspace_path(), snapshot.task_branch())?;
        git_success(
            snapshot.workspace_path(),
            &["reset", "--hard", snapshot.snapshot_ref()],
        )?;

        if !scratch_branch_has_changes(snapshot)? {
            delete_scratch_branch(snapshot)?;
            return Ok(());
        }

        git_success(
            snapshot.workspace_path(),
            &["merge", "--squash", snapshot.scratch_branch()],
        )?;
        git_success(snapshot.workspace_path(), &["commit", "-m", summary])?;
        delete_scratch_branch(snapshot)
    }
}

fn canonicalize_workspace_path(workspace_path: &Path) -> Result<PathBuf, OrbitError> {
    if !workspace_path.is_dir() {
        return Err(OrbitError::InvalidInput(format!(
            "workspace path '{}' is not a directory",
            workspace_path.display()
        )));
    }

    std::fs::canonicalize(workspace_path).map_err(Into::into)
}

fn current_branch(workspace_path: &Path) -> Result<String, OrbitError> {
    let branch = git_output(workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch == "HEAD" {
        return Err(OrbitError::Execution(format!(
            "workspace '{}' has detached HEAD; expected a named task branch",
            workspace_path.display()
        )));
    }
    Ok(branch)
}

fn ensure_clean_tracked_workspace(workspace_path: &Path) -> Result<(), OrbitError> {
    let has_unstaged_changes = !git_command_success(
        workspace_path,
        &["diff", "--quiet", "--ignore-submodules", "--"],
    )?;
    let has_staged_changes = !git_command_success(
        workspace_path,
        &["diff", "--cached", "--quiet", "--ignore-submodules", "--"],
    )?;

    if has_unstaged_changes || has_staged_changes {
        return Err(OrbitError::Execution(format!(
            "workspace '{}' has tracked changes; Groundhog snapshots must start from a clean task branch",
            workspace_path.display()
        )));
    }

    Ok(())
}

fn scratch_branch_name(task_id: &str, day_n: u32) -> String {
    format!("groundhog/{task_id}/day-{day_n}")
}

fn ensure_task_branch_still_at_snapshot(snapshot: &WorkspaceSnapshotRef) -> Result<(), OrbitError> {
    let current_task_head = git_output(
        snapshot.workspace_path(),
        &["rev-parse", snapshot.task_branch()],
    )?;
    if current_task_head != snapshot.snapshot_ref() {
        return Err(OrbitError::Execution(format!(
            "task branch '{}' moved from '{}' to '{}' during Groundhog day-{} for task '{}'; manual intervention required",
            snapshot.task_branch(),
            snapshot.snapshot_ref(),
            current_task_head,
            snapshot.day_n(),
            snapshot.task_id()
        )));
    }

    Ok(())
}

fn ensure_scratch_branch_absent(
    workspace_path: &Path,
    scratch_branch: &str,
) -> Result<(), OrbitError> {
    if git_command_success(
        workspace_path,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{scratch_branch}"),
        ],
    )? {
        return Err(OrbitError::Execution(format!(
            "scratch branch '{scratch_branch}' already exists in '{}'",
            workspace_path.display()
        )));
    }
    Ok(())
}

fn list_untracked_paths(workspace_path: &Path) -> Result<Vec<String>, OrbitError> {
    git_output_paths(
        workspace_path,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ],
    )
}

fn checkout_branch(workspace_path: &Path, branch: &str) -> Result<(), OrbitError> {
    git_success(workspace_path, &["checkout", branch])
}

fn capture_scratch_branch_changes(snapshot: &WorkspaceSnapshotRef) -> Result<(), OrbitError> {
    let workspace_path = snapshot.workspace_path();
    git_success(workspace_path, &["add", "--all", "--", "."])?;
    unstage_preserved_untracked(snapshot)?;

    if !has_staged_changes(workspace_path)? {
        return Ok(());
    }

    let timestamp = Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let snapshot_short_ref: String = snapshot.snapshot_ref().chars().take(12).collect();
    let message = format!(
        "groundhog: capture {} day-{} from {} @ {} {}",
        snapshot.task_id(),
        snapshot.day_n(),
        snapshot.task_branch(),
        snapshot_short_ref,
        timestamp
    );
    git_success(workspace_path, &["commit", "-m", &message])
}

fn unstage_preserved_untracked(snapshot: &WorkspaceSnapshotRef) -> Result<(), OrbitError> {
    if snapshot.preserved_untracked_paths.is_empty() {
        return Ok(());
    }

    let mut args = vec!["rm", "--cached", "--quiet", "--ignore-unmatch", "--"];
    for path in &snapshot.preserved_untracked_paths {
        args.push(path.as_str());
    }
    git_success(snapshot.workspace_path(), &args)
}

fn has_staged_changes(workspace_path: &Path) -> Result<bool, OrbitError> {
    Ok(!git_command_success(
        workspace_path,
        &["diff", "--cached", "--quiet", "--"],
    )?)
}

fn scratch_branch_has_changes(snapshot: &WorkspaceSnapshotRef) -> Result<bool, OrbitError> {
    Ok(!git_command_success(
        snapshot.workspace_path(),
        &[
            "diff",
            "--quiet",
            snapshot.snapshot_ref(),
            snapshot.scratch_branch(),
            "--",
        ],
    )?)
}

fn delete_scratch_branch(snapshot: &WorkspaceSnapshotRef) -> Result<(), OrbitError> {
    git_success(
        snapshot.workspace_path(),
        &["branch", "-D", snapshot.scratch_branch()],
    )
}
