use std::path::Path;
use std::process::Command;

use crate::types::OrbitError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurrentBranchStatus {
    Named(String),
    DetachedHead,
    NoCurrentBranch,
}

pub fn current_branch(workspace_path: &Path) -> Result<CurrentBranchStatus, OrbitError> {
    let symbolic = run_git(
        workspace_path,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
    )?;
    if symbolic.success {
        let branch = symbolic.stdout.trim();
        if branch.is_empty() {
            return Ok(CurrentBranchStatus::NoCurrentBranch);
        }
        return Ok(CurrentBranchStatus::Named(branch.to_string()));
    }

    let verify_head = run_git(workspace_path, &["rev-parse", "--verify", "-q", "HEAD"])?;
    if verify_head.success {
        return Ok(CurrentBranchStatus::DetachedHead);
    }

    Ok(CurrentBranchStatus::NoCurrentBranch)
}

pub fn default_branch(workspace_path: &Path) -> Result<Option<String>, OrbitError> {
    for remote in preferred_remotes(workspace_path)? {
        if let Some(branch) = remote_default_branch(workspace_path, &remote)? {
            return Ok(Some(branch));
        }
    }

    let local_branches = local_branches(workspace_path)?;
    for branch in ["main", "master", "trunk", "develop", "development", "dev"] {
        if local_branches.iter().any(|candidate| candidate == branch) {
            return Ok(Some(branch.to_string()));
        }
    }

    if local_branches.len() == 1 {
        return Ok(local_branches.into_iter().next());
    }

    Ok(None)
}

fn preferred_remotes(workspace_path: &Path) -> Result<Vec<String>, OrbitError> {
    let remotes = run_git(workspace_path, &["remote"])?;
    if !remotes.success {
        return Ok(Vec::new());
    }

    let mut names: Vec<String> = remotes
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    names.sort();
    if let Some(origin_index) = names.iter().position(|name| name == "origin") {
        let origin = names.remove(origin_index);
        names.insert(0, origin);
    }
    Ok(names)
}

fn remote_default_branch(
    workspace_path: &Path,
    remote: &str,
) -> Result<Option<String>, OrbitError> {
    let remote_head = run_git(
        workspace_path,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            &format!("refs/remotes/{remote}/HEAD"),
        ],
    )?;
    if !remote_head.success {
        return Ok(None);
    }

    let head = remote_head.stdout.trim();
    if let Some(branch) = head.strip_prefix(&format!("{remote}/")) {
        return Ok(Some(branch.to_string()));
    }
    if !head.is_empty() {
        return Ok(Some(head.to_string()));
    }
    Ok(None)
}

fn local_branches(workspace_path: &Path) -> Result<Vec<String>, OrbitError> {
    let branches = run_git(
        workspace_path,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )?;
    if !branches.success {
        return Ok(Vec::new());
    }

    Ok(branches
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

pub fn run_git(workspace_path: &Path, args: &[&str]) -> Result<GitCommandOutput, OrbitError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_path)
        .output()
        .map_err(|error| {
            OrbitError::Execution(format!(
                "failed to run `git {}` in '{}': {error}",
                args.join(" "),
                workspace_path.display()
            ))
        })?;

    Ok(GitCommandOutput {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

pub struct GitCommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}
