use std::path::Path;

use orbit_common::types::OrbitError;

use super::super::git::{git_output, git_output_paths, git_success};
use super::author::GitAuthor;

pub(super) fn git_commit_with_author(
    workspace_path: &Path,
    message: &str,
    author: Option<&GitAuthor>,
) -> Result<(), OrbitError> {
    let mut args = vec!["commit".to_string()];
    if let Some(author) = author {
        args.push("--author".to_string());
        args.push(author.spec());
    }
    args.extend(["-m".to_string(), message.to_string()]);
    git_success_dynamic(workspace_path, &args)
}

pub(super) fn stage_paths(workspace_path: &Path, files: &[String]) -> Result<(), OrbitError> {
    if files.is_empty() {
        return Ok(());
    }

    let mut args = vec!["add".to_string(), "-A".to_string(), "--".to_string()];
    args.extend(files.iter().cloned());
    git_success_dynamic(workspace_path, &args)
}

pub(super) fn staged_changed_files(workspace_path: &Path) -> Result<Vec<String>, OrbitError> {
    git_output_paths(
        workspace_path,
        &["diff", "--cached", "--name-only", "-z", "--relative"],
    )
}

fn git_success_dynamic(current_dir: &Path, args: &[String]) -> Result<(), OrbitError> {
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    git_success(current_dir, &args)
}

pub(super) fn ensure_named_branch(workspace_path: &Path) -> Result<(), OrbitError> {
    let actual_branch = git_output(workspace_path, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let actual_branch = actual_branch.trim();
    if actual_branch == "HEAD" {
        return Err(OrbitError::Execution(format!(
            "workspace '{}' has detached HEAD; expected a named branch",
            workspace_path.display(),
        )));
    }
    Ok(())
}

pub(super) fn ensure_no_unmerged_changes(workspace_path: &Path) -> Result<(), OrbitError> {
    let status = git_output(workspace_path, &["status", "--porcelain"])?;
    for line in status.lines() {
        if line.len() < 2 {
            continue;
        }
        let bytes = line.as_bytes();
        let x = bytes[0] as char;
        let y = bytes[1] as char;
        if x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D') {
            return Err(OrbitError::Execution(format!(
                "task worktree '{}' has unresolved merge conflicts",
                workspace_path.display()
            )));
        }
    }
    Ok(())
}
