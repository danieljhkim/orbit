use std::path::Path;

use orbit_common::types::{OrbitError, TaskStatus};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::context::{RuntimeHost, TaskAutomationUpdate, TaskHost};
use crate::executor::automation::input::{input_string_field, required_input_string};

use super::super::git::{
    base_sync_mode_from_input, git_command_success, git_output, git_success,
    resolve_worktree_start_point,
};
use super::resolve_worktree_path_from_prefix;

const DEFAULT_BASE: &str = "main";
const DEFAULT_BRANCH_PREFIX: &str = "orbit";

/// Create a worktree and branch for a single task or task bundle, stamp
/// `job_run_id` and `workspace_path` on every task in scope, and move them to
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
                job_run_id: Some(run_id.clone()),
                ..TaskAutomationUpdate::default()
            },
        )?;
    }

    Ok(worktree_setup_output(
        &run_id,
        workspace_path_str,
        branch_name,
        start_point,
    ))
}

fn worktree_setup_output(
    run_id: &str,
    workspace_path: String,
    head_ref: String,
    base_ref: String,
) -> Value {
    json!({
        "job_run_id": run_id,
        "batch_id": run_id,
        "workspace_path": workspace_path,
        "head_ref": head_ref,
        "base_ref": base_ref,
    })
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
    let target = git_output(
        repo_root,
        &[
            "rev-parse",
            "--verify",
            &format!("{start_point}^{{commit}}"),
        ],
    )?;

    if worktree_path.exists() {
        if git_command_success(worktree_path, &["rev-parse", "--is-inside-work-tree"])? {
            git_success(worktree_path, &["checkout", "-B", branch_name, &target])?;
            git_success(worktree_path, &["clean", "-fd"])?;
            return Ok(());
        }

        if is_empty_dir(worktree_path)? {
            std::fs::remove_dir(worktree_path).map_err(|error| {
                OrbitError::Execution(format!(
                    "failed to remove empty invalid worktree path '{}': {error}",
                    worktree_path.display()
                ))
            })?;
        } else {
            return Err(OrbitError::Execution(format!(
                "worktree path '{}' exists but is not a Git worktree; move it aside or remove it before retrying",
                worktree_path.display()
            )));
        }
    }

    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            OrbitError::Execution(format!(
                "failed to create worktree directory '{}': {error}",
                parent.display()
            ))
        })?;
    }

    git_success(repo_root, &["worktree", "prune"])?;
    let worktree_path_arg = worktree_path.to_string_lossy();
    git_success(
        repo_root,
        &[
            "worktree",
            "add",
            "-B",
            branch_name,
            &worktree_path_arg,
            &target,
        ],
    )
}

fn is_empty_dir(path: &Path) -> Result<bool, OrbitError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        OrbitError::Execution(format!(
            "failed to inspect worktree path '{}': {error}",
            path.display()
        ))
    })?;
    if !metadata.is_dir() {
        return Ok(false);
    }

    let mut entries = std::fs::read_dir(path).map_err(|error| {
        OrbitError::Execution(format!(
            "failed to read worktree path '{}': {error}",
            path.display()
        ))
    })?;
    Ok(entries.next().is_none())
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

    #[test]
    fn ensure_worktree_reuses_orphan_branch_from_failed_attempt() {
        let temp = tempdir().unwrap();
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        init_repo(&repo, "agent-main");
        let first_base = commit_file(&repo, "base.txt", "v1");
        git(&repo, &["branch", "orbit/test", &first_base]);

        let second_base = commit_file(&repo, "base.txt", "v2");
        ensure_worktree(&repo, &worktree, &second_base, "orbit/test").unwrap();

        assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), second_base);
    }

    #[test]
    fn ensure_worktree_prunes_dangling_metadata_from_failed_attempt() {
        let temp = tempdir().unwrap();
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        init_repo(&repo, "agent-main");
        let base = commit_file(&repo, "base.txt", "v1");

        ensure_worktree(&repo, &worktree, &base, "orbit/test").unwrap();
        fs::remove_dir_all(&worktree).unwrap();

        ensure_worktree(&repo, &worktree, &base, "orbit/test").unwrap();

        assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), base);
    }

    #[test]
    fn ensure_worktree_reuses_empty_path_from_failed_attempt() {
        let temp = tempdir().unwrap();
        let repo = temp.path().join("repo");
        let worktree = temp.path().join("worktree");
        init_repo(&repo, "agent-main");
        let base = commit_file(&repo, "base.txt", "v1");
        fs::create_dir_all(&worktree).unwrap();

        ensure_worktree(&repo, &worktree, &base, "orbit/test").unwrap();

        assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), base);
    }

    #[test]
    fn ensure_worktree_uses_commit_start_point_without_upstream_config() {
        let temp = tempdir().unwrap();
        let remote = temp.path().join("remote.git");
        let seed = temp.path().join("seed");
        let local = temp.path().join("local");
        let worktree = temp.path().join("worktree");

        git(temp.path(), &["init", "--bare", remote.to_str().unwrap()]);
        init_repo(&seed, "agent-main");
        let remote_head = commit_file(&seed, "base.txt", "v1");
        git(
            &seed,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&seed, &["push", "-u", "origin", "agent-main"]);
        git(
            temp.path(),
            &[
                "clone",
                "--branch",
                "agent-main",
                remote.to_str().unwrap(),
                local.to_str().unwrap(),
            ],
        );

        ensure_worktree(&local, &worktree, "origin/agent-main", "orbit/test").unwrap();

        assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), remote_head);
        assert_git_fails(&local, &["config", "--get", "branch.orbit/test.remote"]);
        assert_git_fails(&local, &["config", "--get", "branch.orbit/test.merge"]);
    }

    #[test]
    fn worktree_setup_output_includes_legacy_batch_id_alias() {
        let output = worktree_setup_output(
            "jrun-test",
            "/tmp/orbit-worktree".to_string(),
            "orbit/ORB-00010".to_string(),
            "main".to_string(),
        );

        assert_eq!(output["job_run_id"], json!("jrun-test"));
        assert_eq!(output["batch_id"], output["job_run_id"]);
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

    fn assert_git_fails(current_dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(current_dir)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "git {} unexpectedly succeeded in {}:\nstdout: {}\nstderr: {}",
            args.join(" "),
            current_dir.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
