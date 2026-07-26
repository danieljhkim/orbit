use std::path::Path;

use orbit_common::types::OrbitError;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::context::{
    DeterministicActionHost, TaskAutomationUpdate, TaskHost, ensure_task_can_enter_workflow,
};
use crate::executor::automation::input::input_string_field;

use super::super::git::{
    base_sync_mode_from_input, git_command_success, git_output, git_success,
    resolve_worktree_start_point,
};
use super::WorktreeIdentity;
use super::dependency_delivery::{
    DependencyDeliveryMode, dependency_delivery_mode_from_input,
    ensure_dependencies_delivered_into_base,
};

const DEFAULT_BASE: &str = "main";

/// Create a worktree and branch for a single task or task bundle, stamp
/// `job_run_id` and `workspace_path` on every task in scope, and move them to
/// `in_progress`.
///
/// Generic automation — not tied to duel or any specific workflow. Any
/// pipeline can reuse this by passing a `branch_prefix`.
pub(in crate::executor::automation) fn setup_worktree<
    H: DeterministicActionHost + TaskHost + ?Sized,
>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    // The worktree's identity is derived once, here and in gc, from the same
    // rule (ORB-10427) — never re-spelled per call site.
    let identity = WorktreeIdentity::from_input(input, None)?;
    let task_ids = &identity.task_ids;
    let run_id = &identity.run_id;
    let base = input_string_field(input, "base")
        .or_else(|| input_string_field(input, "base_branch"))
        .unwrap_or_else(|| DEFAULT_BASE.to_string());
    let base_sync_mode = base_sync_mode_from_input(input)?;
    let dependency_delivery_mode = dependency_delivery_mode_from_input(input)?;

    let repo_root_str = host.repo_root()?;
    let repo_root = Path::new(&repo_root_str);

    for task_id in task_ids {
        ensure_task_can_enter_workflow(host, task_id, "worktree_setup")?;
    }

    let start_point = resolve_worktree_start_point(repo_root, &base, base_sync_mode)?;
    // ORB-10380: `start_point` is a moving name (`origin/<base>`) shared by every
    // worktree off this `.git`; a sibling run's fetch or a merge can move it
    // mid-run. Resolve it exactly once here and hand the resulting commit id to
    // both the worktree creation and every downstream step that needs the base.
    let base_sha = git_output(
        repo_root,
        &[
            "rev-parse",
            "--verify",
            &format!("{start_point}^{{commit}}"),
        ],
    )?;

    // ORB-10464 (F2026-07-038): a dependency being `done` says nothing about
    // whether its commits reached this base. Verify that before anything
    // exists on disk, so a refusal leaves no worktree, no branch, and every
    // task still in its pre-run status.
    if dependency_delivery_mode == DependencyDeliveryMode::Enforce {
        ensure_dependencies_delivered_into_base(
            host,
            repo_root,
            task_ids,
            &start_point,
            &base_sha,
        )?;
    }

    let branch_name = branch_name_for_tasks(&identity.branch_prefix, task_ids);

    let worktree_path = identity.path(repo_root)?;

    ensure_worktree(repo_root, &worktree_path, &base_sha, &branch_name)?;

    let workspace_path_str = worktree_path.to_string_lossy().to_string();

    for task_id in task_ids {
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
        run_id,
        workspace_path_str,
        branch_name,
        start_point,
        base_sha,
    ))
}

// pub(crate) widened for tests/ layout migration (ORB-00240); test reaches via
// exposed surface per docs/design-patterns/test_layout.md. (Logged via
// orbit.task.update model=grok on ORB-00240 before this edit for the visibility
// change on internal test helpers.)
pub(crate) fn worktree_setup_output(
    run_id: &str,
    workspace_path: String,
    head_ref: String,
    base_ref: String,
    base_sha: String,
) -> Value {
    json!({
        "job_run_id": run_id,
        "batch_id": run_id,
        "workspace_path": workspace_path,
        "head_ref": head_ref,
        // The moving name, for steps that legitimately reconcile against the
        // live base (`sync_base`, `pr_open`).
        "base_ref": base_ref,
        // ORB-10380: the immutable commit this worktree was created at. Steps
        // that must reason about the history this run authored consume this.
        "base_sha": base_sha,
    })
}

// pub(crate) widened for tests/ layout migration (ORB-00240); test reaches via
// exposed surface per docs/design-patterns/test_layout.md. (Logged via
// orbit.task.update model=grok on ORB-00240 before this edit for the visibility
// change on internal test helpers.)
pub(crate) fn ensure_worktree(
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
