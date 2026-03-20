use std::path::Path;

use orbit_exec::{EnvironmentMode, ExecRequest, NoSandbox, StdinMode, run_process};
use orbit_types::OrbitError;

pub(super) fn git_output_paths(
    current_dir: &Path,
    args: &[&str],
) -> Result<Vec<String>, OrbitError> {
    let raw = git_output_raw(current_dir, args)?;
    Ok(raw
        .split('\0')
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

pub(super) fn git_output(current_dir: &Path, args: &[&str]) -> Result<String, OrbitError> {
    Ok(git_output_raw(current_dir, args)?.trim().to_string())
}

pub(super) fn git_output_raw(current_dir: &Path, args: &[&str]) -> Result<String, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            current_dir: Some(current_dir.to_string_lossy().to_string()),
            timeout_ms: Some(30_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "git {} failed in '{}': {}",
            args.join(" "),
            current_dir.display(),
            result.stderr.trim()
        )));
    }

    Ok(result.stdout)
}

pub(super) fn git_success(current_dir: &Path, args: &[&str]) -> Result<(), OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            current_dir: Some(current_dir.to_string_lossy().to_string()),
            timeout_ms: Some(30_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;

    if !result.success {
        return Err(OrbitError::Execution(format!(
            "git {} failed in '{}': {}",
            args.join(" "),
            current_dir.display(),
            result.stderr.trim()
        )));
    }

    Ok(())
}

pub(super) fn git_command_success(
    current_dir: &Path,
    args: &[&str],
) -> Result<bool, OrbitError> {
    let result = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            current_dir: Some(current_dir.to_string_lossy().to_string()),
            timeout_ms: Some(30_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    )?;
    Ok(result.success)
}

pub(super) fn fetch_remote_base(repo_root: &Path, base: &str) {
    let _ = run_process(
        &ExecRequest {
            program: "git".to_string(),
            args: vec!["fetch".to_string(), "origin".to_string(), base.to_string()],
            current_dir: Some(repo_root.to_string_lossy().to_string()),
            timeout_ms: Some(60_000),
            stdin_mode: StdinMode::Null,
            environment_mode: EnvironmentMode::Inherit,
            debug: false,
        },
        &NoSandbox,
    );
}

/// Advance the local base branch to match the freshly-fetched remote ref.
///
/// - If the local branch is checked out and clean, run `git pull --rebase`.
/// - If the local branch is not checked out and is a strict ancestor of
///   `origin/<base>`, fast-forward it with `git branch -f`.
/// - Skips silently when either ref is missing or when the working tree is dirty.
pub(super) fn refresh_local_base_branch(repo_root: &Path, base: &str) -> Result<(), OrbitError> {
    let local_base_exists = git_command_success(
        repo_root,
        &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
    )?;
    let remote_base = format!("origin/{base}");
    let remote_base_exists = git_command_success(
        repo_root,
        &[
            "rev-parse",
            "--verify",
            &format!("{remote_base}^{{commit}}"),
        ],
    )?;

    if !local_base_exists || !remote_base_exists {
        return Ok(());
    }

    let current_branch = git_output(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if current_branch == base {
        if !git_worktree_is_clean(repo_root)? {
            return Ok(());
        }
        git_success(repo_root, &["pull", "--rebase", "origin", base])?;
        return Ok(());
    }

    // Fast-forward a non-checked-out local base branch to the fetched remote ref when possible.
    if git_command_success(
        repo_root,
        &["merge-base", "--is-ancestor", base, &remote_base],
    )? {
        git_success(repo_root, &["branch", "-f", base, &remote_base])?;
    }

    Ok(())
}

pub(super) fn resolve_worktree_start_point(
    repo_root: &Path,
    base: &str,
) -> Result<String, OrbitError> {
    // Prefer the local branch: after refresh_local_base_branch it reflects
    // the best-known state (remote updates + any local commits not yet pushed).
    if git_command_success(
        repo_root,
        &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
    )? {
        return Ok(base.to_string());
    }

    let remote_base = format!("origin/{base}");
    if git_command_success(
        repo_root,
        &[
            "rev-parse",
            "--verify",
            &format!("{remote_base}^{{commit}}"),
        ],
    )? {
        return Ok(remote_base);
    }

    Err(OrbitError::Execution(format!(
        "unable to resolve base ref '{base}' for task worktree creation"
    )))
}

fn git_worktree_is_clean(current_dir: &Path) -> Result<bool, OrbitError> {
    Ok(git_output(current_dir, &["status", "--porcelain"])?.is_empty())
}
