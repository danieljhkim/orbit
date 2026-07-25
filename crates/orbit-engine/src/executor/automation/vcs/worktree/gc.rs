use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use chrono::{DateTime, Utc};
use orbit_common::types::{JobRun, JobRunState, OrbitError};
use serde::Serialize;
use serde_json::Value;

use super::cleanup::remove_worktree;
use super::{resolve_shared_worktree_path, resolve_worktree_path_from_prefix};

const DEFAULT_BRANCH_PREFIX: &str = "orbit";

#[derive(Debug, Clone, Default)]
pub struct WorktreeGcOptions {
    pub delete: bool,
    pub run_id: Option<String>,
    pub older_than: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorktreeGcReport {
    pub path: PathBuf,
    pub run_id: Option<String>,
    pub run_state: Option<JobRunState>,
    pub action: String,
    pub bytes_reclaimed: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorktreeGcResult {
    pub dry_run: bool,
    pub bytes_reclaimed: u64,
    pub reports: Vec<WorktreeGcReport>,
}

pub fn collect_worktrees(
    repo_root: &Path,
    runs: &[JobRun],
    options: &WorktreeGcOptions,
) -> Result<WorktreeGcResult, OrbitError> {
    let mut known_paths = BTreeMap::<PathBuf, Vec<&JobRun>>::new();
    for run in runs {
        let path = expected_path(repo_root, run)?;
        known_paths.entry(path).or_default().push(run);
    }

    let mut reports = Vec::new();
    for (path, matching_runs) in &known_paths {
        let selected_runs = matching_runs
            .iter()
            .copied()
            .filter(|run| {
                options
                    .run_id
                    .as_deref()
                    .is_none_or(|wanted| wanted == run.run_id)
            })
            .collect::<Vec<_>>();
        if selected_runs.is_empty() {
            continue;
        }
        if !path.exists() {
            continue;
        }
        if matching_runs.len() > 1 {
            reports.extend(selected_runs.into_iter().map(|run| WorktreeGcReport {
                path: path.clone(),
                run_id: Some(run.run_id.clone()),
                run_state: Some(run.state),
                action: "skipped:ambiguous_run_path".to_string(),
                bytes_reclaimed: 0,
            }));
            continue;
        }
        reports.push(classify_known(repo_root, path, selected_runs[0], options)?);
    }

    if options.run_id.is_none() {
        let known: BTreeSet<_> = known_paths.keys().cloned().collect();
        for entry in on_disk_worktrees(repo_root)? {
            if !known.contains(&entry) {
                reports.push(WorktreeGcReport {
                    path: entry,
                    run_id: None,
                    run_state: None,
                    action: "skipped:unrecognized".to_string(),
                    bytes_reclaimed: 0,
                });
            }
        }
    }

    // This repairs already-stale Git administration entries. It is safe in
    // dry-run mode because it never removes a worktree directory or branch.
    git(repo_root, &["worktree", "prune"])?;

    reports.sort_by(|left, right| left.path.cmp(&right.path));
    let bytes_reclaimed = reports.iter().map(|report| report.bytes_reclaimed).sum();
    Ok(WorktreeGcResult {
        dry_run: !options.delete,
        bytes_reclaimed,
        reports,
    })
}

fn classify_known(
    repo_root: &Path,
    path: &Path,
    run: &JobRun,
    options: &WorktreeGcOptions,
) -> Result<WorktreeGcReport, OrbitError> {
    let mut report = WorktreeGcReport {
        path: path.to_path_buf(),
        run_id: Some(run.run_id.clone()),
        run_state: Some(run.state),
        action: String::new(),
        bytes_reclaimed: 0,
    };
    if !run.state.is_terminal() {
        report.action = "skipped:run_not_terminal".to_string();
        return Ok(report);
    }
    if options
        .older_than
        .is_some_and(|cutoff| run.finished_at.unwrap_or(run.created_at) > cutoff)
    {
        report.action = "skipped:too_recent".to_string();
        return Ok(report);
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        OrbitError::Execution(format!(
            "failed to inspect worktree '{}': {error}",
            path.display()
        ))
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        report.action = "skipped:not_a_real_directory".to_string();
        return Ok(report);
    }
    if !is_registered_worktree(repo_root, path)? {
        report.action = "skipped:not_registered_worktree".to_string();
        return Ok(report);
    }
    if !git_output(path, &["status", "--porcelain", "--untracked-files=all"])?
        .trim()
        .is_empty()
    {
        report.action = "skipped:dirty_rescue_candidate".to_string();
        return Ok(report);
    }

    let branch = match branch_name(path) {
        Ok(branch) => branch,
        Err(_) => {
            report.action = "skipped:branch_unknown".to_string();
            return Ok(report);
        }
    };
    if !branch_is_merged(repo_root, &branch) && !branch_pr_is_closed(repo_root, &branch) {
        report.action = "skipped:branch_not_merged_or_pr_closed".to_string();
        return Ok(report);
    }

    let estimated_bytes = directory_bytes(path)?;
    if !options.delete {
        report.action = "would_remove".to_string();
        return Ok(report);
    }

    // Deliberately no `--force`: a last-moment dirtying of the worktree makes
    // Git fail closed. Never replace this with raw recursive deletion.
    let branch_to_delete = branch_exists(repo_root, &branch).then_some(branch.as_str());
    remove_worktree(repo_root, path, branch_to_delete, false)?;
    report.action = "removed".to_string();
    report.bytes_reclaimed = estimated_bytes;
    Ok(report)
}

fn expected_path(repo_root: &Path, run: &JobRun) -> Result<PathBuf, OrbitError> {
    let input = run.input.as_ref().unwrap_or(&Value::Null);
    if let Some(prefix) = string_field(input, "branch_prefix") {
        resolve_worktree_path_from_prefix(repo_root, prefix, &run.run_id)
    } else if string_field(input, "task_id").is_some() {
        resolve_worktree_path_from_prefix(repo_root, DEFAULT_BRANCH_PREFIX, &run.run_id)
    } else {
        resolve_shared_worktree_path(repo_root, &run.run_id)
    }
}

fn string_field<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn on_disk_worktrees(repo_root: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    let sentinel = resolve_shared_worktree_path(repo_root, "gc-root-sentinel")?;
    let Some(root) = sentinel.parent() else {
        return Ok(Vec::new());
    };
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| {
        OrbitError::Execution(format!(
            "failed to inventory worktree root '{}': {error}",
            root.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            OrbitError::Execution(format!(
                "failed to read an entry under '{}': {error}",
                root.display()
            ))
        })?;
        if entry
            .file_type()
            .map_err(|error| {
                OrbitError::Execution(format!(
                    "failed to inspect '{}': {error}",
                    entry.path().display()
                ))
            })?
            .is_dir()
        {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}

fn branch_name(worktree: &Path) -> Result<String, OrbitError> {
    let branch = git_output(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let branch = branch.trim();
    if branch.is_empty() || branch == "HEAD" {
        return Err(OrbitError::Execution(format!(
            "cannot safely collect detached worktree '{}'",
            worktree.display()
        )));
    }
    Ok(branch.to_string())
}

fn is_registered_worktree(repo_root: &Path, path: &Path) -> Result<bool, OrbitError> {
    let list = git_output(repo_root, &["worktree", "list", "--porcelain"])?;
    let expected = path.to_string_lossy();
    Ok(list
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .any(|registered| registered == expected))
}

fn branch_is_merged(repo_root: &Path, branch: &str) -> bool {
    Command::new("git")
        .current_dir(repo_root)
        .args(["merge-base", "--is-ancestor", branch, "HEAD"])
        .status()
        .is_ok_and(|status| status.success())
}

fn branch_pr_is_closed(repo_root: &Path, branch: &str) -> bool {
    let output = Command::new("gh")
        .current_dir(repo_root)
        .args([
            "pr", "list", "--head", branch, "--state", "closed", "--limit", "1", "--json", "number",
        ])
        .output();
    output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| serde_json::from_slice::<Vec<Value>>(&output.stdout).ok())
        .is_some_and(|items| !items.is_empty())
}

fn branch_exists(repo_root: &Path, branch: &str) -> bool {
    Command::new("git")
        .current_dir(repo_root)
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .status()
        .is_ok_and(|status| status.success())
}

fn directory_bytes(path: &Path) -> Result<u64, OrbitError> {
    let mut total = 0u64;
    let mut pending = vec![path.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).map_err(|error| {
            OrbitError::Execution(format!(
                "failed to measure worktree '{}': {error}",
                current.display()
            ))
        })? {
            let entry = entry.map_err(|error| {
                OrbitError::Execution(format!(
                    "failed to measure an entry under '{}': {error}",
                    current.display()
                ))
            })?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
                OrbitError::Execution(format!(
                    "failed to measure '{}': {error}",
                    entry.path().display()
                ))
            })?;
            total = total.saturating_add(metadata.len());
            if metadata.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(total)
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String, OrbitError> {
    let output = git_raw(cwd, args)?;
    String::from_utf8(output.stdout)
        .map_err(|error| OrbitError::Execution(format!("git output was not UTF-8: {error}")))
}

fn git(cwd: &Path, args: &[&str]) -> Result<(), OrbitError> {
    let _ = git_raw(cwd, args)?;
    Ok(())
}

fn git_raw(cwd: &Path, args: &[&str]) -> Result<Output, OrbitError> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(|error| OrbitError::Execution(format!("failed to run git: {error}")))?;
    if output.status.success() {
        return Ok(output);
    }
    Err(OrbitError::Execution(format!(
        "git {} failed in '{}': {}",
        args.join(" "),
        cwd.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}
