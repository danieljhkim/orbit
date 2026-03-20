use std::path::{Path, PathBuf};

use orbit_types::OrbitError;
use serde_json::{Value, json};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};

use super::git::{
    fetch_remote_base, git_command_success, git_output, git_success, resolve_worktree_start_point,
};
use super::input::{
    canonicalize_existing_dir, input_repo_root, input_string_field, input_workspace_path,
    required_input_string,
};

pub(super) fn create_task_worktree<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_id = required_input_string(input, "task_id")?;
    let repo_root = host.repo_root().or_else(|_| {
        let task = host.get_task(task_id)?;
        task.workspace_path.ok_or_else(|| {
            OrbitError::InvalidInput(format!(
                "task '{task_id}' must define workspace_path when Orbit cannot derive the repository root automatically"
            ))
        })
    })?;
    let repo_root = canonicalize_existing_dir(&repo_root, "repo_root")?;
    let base = input_string_field(input, "base").unwrap_or_else(|| "agent-main".to_string());
    let branch = format!("orbit/{task_id}");
    let worktree_path = resolve_task_worktree_path(&repo_root, task_id)?;

    if worktree_path.exists() {
        ensure_existing_task_worktree(&worktree_path, &branch)?;
    } else {
        fetch_remote_base(&repo_root, &base);
        let start_point = resolve_worktree_start_point(&repo_root, &base)?;
        create_or_attach_task_worktree(&repo_root, &worktree_path, &branch, &start_point)?;
    }

    let canonical_worktree = worktree_path.canonicalize().map_err(|error| {
        OrbitError::Execution(format!(
            "failed to canonicalize task worktree '{}': {error}",
            worktree_path.display()
        ))
    })?;
    let canonical_repo_root = repo_root.canonicalize().map_err(|error| {
        OrbitError::Execution(format!(
            "failed to canonicalize repo_root '{}': {error}",
            repo_root.display()
        ))
    })?;

    host.apply_task_automation_update(
        task_id,
        TaskAutomationUpdate {
            workspace_path: Some(canonical_worktree.to_string_lossy().to_string()),
            repo_root: Some(canonical_repo_root.to_string_lossy().to_string()),
            branch: Some(branch.clone()),
            ..TaskAutomationUpdate::default()
        },
    )?;

    Ok(json!({
        "workspace_path": canonical_worktree.to_string_lossy().to_string(),
        "repo_root": canonical_repo_root.to_string_lossy().to_string(),
        "branch": branch,
    }))
}

pub(super) fn finalize_task_worktree(input: &Value) -> Result<Value, OrbitError> {
    let workspace_path = canonicalize_existing_dir(
        &input_workspace_path(input).ok_or_else(|| {
            OrbitError::InvalidInput(
                "finalize_task_worktree requires input.workspace_path".to_string(),
            )
        })?,
        "workspace_path",
    )?;
    let repo_root = canonicalize_existing_dir(&input_repo_root(input)?, "repo_root")?;
    let cleanup_strategy = if workspace_path == repo_root {
        "main_checkout_unchanged"
    } else {
        "retained"
    };
    Ok(json!({
        "workspace_path": workspace_path.to_string_lossy().to_string(),
        "repo_root": repo_root.to_string_lossy().to_string(),
        "cleanup_strategy": cleanup_strategy,
    }))
}

fn resolve_task_worktree_path(repo_root: &Path, task_id: &str) -> Result<PathBuf, OrbitError> {
    let repo_name = repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            OrbitError::Execution(format!(
                "cannot derive repository name from '{}'",
                repo_root.display()
            ))
        })?;
    let base_root = match std::env::var("ORBIT_WORKTREE_ROOT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        Some(value) => PathBuf::from(value),
        None => {
            let parent = repo_root.parent().ok_or_else(|| {
                OrbitError::Execution(format!(
                    "cannot derive worktree root from '{}'",
                    repo_root.display()
                ))
            })?;
            parent.parent().unwrap_or(parent).join("worktrees")
        }
    };
    Ok(base_root.join(repo_name).join(task_id))
}

fn ensure_existing_task_worktree(
    worktree_path: &Path,
    expected_branch: &str,
) -> Result<(), OrbitError> {
    let inside = git_output(worktree_path, &["rev-parse", "--is-inside-work-tree"])?;
    if inside.trim() != "true" {
        return Err(OrbitError::Execution(format!(
            "worktree path exists but is not a git worktree: {}",
            worktree_path.display()
        )));
    }
    let current_branch = git_output(worktree_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if current_branch.trim() != expected_branch {
        return Err(OrbitError::Execution(format!(
            "existing worktree '{}' is on branch '{}' but '{}' was expected",
            worktree_path.display(),
            current_branch.trim(),
            expected_branch
        )));
    }
    Ok(())
}

fn create_or_attach_task_worktree(
    repo_root: &Path,
    worktree_path: &Path,
    branch: &str,
    start_point: &str,
) -> Result<(), OrbitError> {
    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            OrbitError::Execution(format!(
                "failed to create task worktree directory '{}': {error}",
                parent.display()
            ))
        })?;
    }

    if git_command_success(
        repo_root,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )? {
        git_success(
            repo_root,
            &["worktree", "add", &worktree_path.to_string_lossy(), branch],
        )
    } else {
        git_success(
            repo_root,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                &worktree_path.to_string_lossy(),
                start_point,
            ],
        )
    }
}
