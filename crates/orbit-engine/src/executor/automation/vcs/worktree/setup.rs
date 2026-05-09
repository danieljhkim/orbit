use std::path::Path;

use orbit_common::types::{OrbitError, TaskStatus};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};
use crate::executor::automation::input::{input_string_field, required_input_string};

use super::super::git::{base_sync_mode_from_input, git_success, resolve_worktree_start_point};
use super::resolve_worktree_path_from_prefix;

const DEFAULT_BASE: &str = "main";
const DEFAULT_BRANCH_PREFIX: &str = "orbit";

/// Create a worktree and branch for a single task or task bundle, stamp
/// `batch_id` and `workspace_path` on every task in scope, and move them to
/// `in_progress`.
///
/// Generic automation — not tied to duel or any specific workflow. Any
/// pipeline can reuse this by passing a `branch_prefix`.
pub(in crate::executor::automation) fn setup_worktree<H: RuntimeHost + TaskHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let task_ids = task_ids_from_input(input)?;
    let run_id = input_string_field(input, "run_id")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback_run_id_for_tasks(&task_ids));
    let base = input_string_field(input, "base")
        .or_else(|| input_string_field(input, "base_branch"))
        .unwrap_or_else(|| DEFAULT_BASE.to_string());
    let base_sync_mode = base_sync_mode_from_input(input)?;
    let branch_prefix = input_string_field(input, "branch_prefix")
        .unwrap_or_else(|| DEFAULT_BRANCH_PREFIX.to_string());

    let repo_root_str = host.repo_root()?;
    let repo_root = Path::new(&repo_root_str);

    for task_id in &task_ids {
        ensure_task_can_enter_workflow(host, task_id, "worktree_setup")?;
    }

    let start_point = resolve_worktree_start_point(repo_root, &base, base_sync_mode)?;

    let branch_name = branch_name_for_tasks(&branch_prefix, &task_ids);

    let worktree_path = resolve_worktree_path_from_prefix(repo_root, &branch_prefix, &run_id)?;

    ensure_worktree(repo_root, &worktree_path, &start_point, &branch_name)?;

    let workspace_path_str = worktree_path.to_string_lossy().to_string();

    for task_id in &task_ids {
        host.admit_task_for_workflow(task_id, "worktree_setup")?;
        host.apply_task_automation_update(
            task_id,
            TaskAutomationUpdate {
                batch_id: Some(run_id.clone()),
                workspace_path: Some(Some(workspace_path_str.clone())),
                ..TaskAutomationUpdate::default()
            },
        )?;
    }

    Ok(json!({
        "batch_id": run_id,
        "workspace_path": workspace_path_str,
        "head_ref": branch_name,
        "base_ref": start_point,
    }))
}

fn ensure_task_can_enter_workflow<H: TaskHost + ?Sized>(
    host: &H,
    task_id: &str,
    workflow: &str,
) -> Result<(), OrbitError> {
    let task = host.get_task(task_id)?;
    if matches!(
        task.status,
        TaskStatus::Proposed
            | TaskStatus::Friction
            | TaskStatus::Backlog
            | TaskStatus::Rejected
            | TaskStatus::Archived
            | TaskStatus::InProgress
    ) {
        return Ok(());
    }

    Err(OrbitError::InvalidInput(format!(
        "task '{}' is in status '{}'; workflow admission for '{workflow}' requires 'proposed', 'friction', 'backlog', 'rejected', 'archived', or 'in-progress'",
        task.id, task.status
    )))
}

fn ensure_worktree(
    repo_root: &Path,
    worktree_path: &Path,
    start_point: &str,
    branch_name: &str,
) -> Result<(), OrbitError> {
    if worktree_path.exists() {
        let target = super::super::git::git_output(repo_root, &["rev-parse", start_point])?;
        git_success(
            worktree_path,
            &["checkout", "-B", branch_name, target.trim()],
        )?;
        git_success(worktree_path, &["clean", "-fd"])?;
        return Ok(());
    }

    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            OrbitError::Execution(format!(
                "failed to create worktree directory '{}': {error}",
                parent.display()
            ))
        })?;
    }

    git_success(
        repo_root,
        &[
            "worktree",
            "add",
            "-B",
            branch_name,
            &worktree_path.to_string_lossy(),
            start_point,
        ],
    )
}

fn task_ids_from_input(input: &Value) -> Result<Vec<String>, OrbitError> {
    if let Some(items) = input.get("task_ids").and_then(Value::as_array) {
        let task_ids = items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| {
                        OrbitError::InvalidInput(
                            "setup_worktree input.task_ids entries must be non-empty strings"
                                .to_string(),
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !task_ids.is_empty() {
            return Ok(task_ids);
        }
    }

    Ok(vec![required_input_string(input, "task_id")?.to_string()])
}

fn branch_name_for_tasks(branch_prefix: &str, task_ids: &[String]) -> String {
    if task_ids.len() == 1 {
        let short_ts = format!(
            "{:08x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        return format!("{branch_prefix}/{}-{short_ts}", task_ids[0]);
    }

    let mut sorted_ids = task_ids.to_vec();
    sorted_ids.sort();
    let digest = Sha256::digest(sorted_ids.join(","));
    let bundle_hash = format!("{digest:x}");
    format!("{branch_prefix}/bundle-{}", &bundle_hash[..8])
}

fn fallback_run_id_for_tasks(task_ids: &[String]) -> String {
    if task_ids.len() == 1 {
        return format!("task-{}", task_ids[0]);
    }

    let mut sorted_ids = task_ids.to_vec();
    sorted_ids.sort();
    let digest = Sha256::digest(sorted_ids.join(","));
    format!("bundle-{}", &format!("{digest:x}")[..8])
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn ensure_worktree_resets_existing_checkout_to_supplied_start_point() {
        let temp = tempdir().unwrap();
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        init_repo(&repo, "agent-main");
        let first_base = commit_file(&repo, "base.txt", "v1");

        ensure_worktree(&repo, &worktree, &first_base, "orbit/test").unwrap();
        assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), first_base);

        let second_base = commit_file(&repo, "base.txt", "v2");
        ensure_worktree(&repo, &worktree, &second_base, "orbit/test").unwrap();

        assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), second_base);
    }

    fn init_repo(path: &Path, branch: &str) {
        fs::create_dir_all(path).unwrap();
        git(path, &["init"]);
        git(path, &["checkout", "-b", branch]);
        git(path, &["config", "user.name", "Orbit Test"]);
        git(path, &["config", "user.email", "orbit-test@example.com"]);
    }

    fn commit_file(repo: &Path, file_name: &str, contents: &str) -> String {
        fs::write(repo.join(file_name), contents).unwrap();
        git(repo, &["add", file_name]);
        git(repo, &["commit", "-m", &format!("write {file_name}")]);
        git(repo, &["rev-parse", "HEAD"])
    }

    fn git(current_dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(current_dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed in {}:\nstdout: {}\nstderr: {}",
            args.join(" "),
            current_dir.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }
}
