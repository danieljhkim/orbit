mod author;
mod git_ops;
mod message;
mod scope;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use orbit_common::types::{NO_DIFF_EXPECTED_TAG, OrbitError};
use serde_json::{Value, json};

use crate::context::{DeterministicActionHost, TaskHost};

use super::super::input::{canonicalize_existing_dir, input_string_field, required_job_run_id};
use super::git::{git_command_success, git_output, git_success};
use super::handoff::reject_failed_delivery;
use author::{append_co_author_trailers, commit_author_for_tasks};
use git_ops::{
    ensure_named_branch, ensure_no_unmerged_changes, git_commit_with_identity, stage_paths,
    staged_changed_files,
};
use message::{batch_commit_message, finalize_commit_message, task_commit_message};
use scope::{changed_files_for_task, collect_worktree_changes, filter_changed_files_for_task};

pub(in crate::executor::automation) fn git_commit<
    H: TaskHost + DeterministicActionHost + ?Sized,
>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let scope = input.get("scope").and_then(Value::as_str).unwrap_or("all");
    match scope {
        "per_task" => commit_task_artifact_changes(host, input),
        "per_task_finalize" => commit_finalize_artifact_changes(host, input),
        "all" => commit_batch_changes(host, input),
        other => Err(OrbitError::InvalidInput(format!(
            "git_commit: unknown scope '{other}'; expected per_task, per_task_finalize, or all"
        ))),
    }
}

pub(super) fn commit_task_artifact_changes<H: TaskHost + DeterministicActionHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = required_job_run_id(input, "commit_task_artifact_changes")?;
    let explicit_completed_task_ids = completed_task_ids_field(input);
    if explicit_completed_task_ids
        .as_ref()
        .is_some_and(|task_ids| task_ids.is_empty())
    {
        return Ok(json!({
            "committed_task_ids": [],
            "skipped_task_ids": [],
        }));
    }

    let fallback_batch_tasks = if explicit_completed_task_ids.is_none() {
        Some(host.list_tasks_filtered(None, None, None, Some(batch_id), None, None)?)
    } else {
        None
    };
    if fallback_batch_tasks
        .as_ref()
        .is_some_and(|batch_tasks| batch_tasks.is_empty())
    {
        return Ok(json!({
            "committed_task_ids": [],
            "skipped_task_ids": [],
        }));
    }

    let workspace_path = resolve_workspace_path(host, input, batch_id)?;
    ensure_named_branch(&workspace_path)?;
    ensure_no_unmerged_changes(&workspace_path)?;
    let resolved_model = host.resolved_crew_model(batch_id)?;

    let task_ids = match explicit_completed_task_ids {
        Some(task_ids) => task_ids,
        None => fallback_batch_tasks
            .unwrap_or_default()
            .into_iter()
            .map(|task| task.id)
            .collect(),
    };

    let mut committed_task_ids = Vec::new();
    let mut skipped_task_ids = Vec::new();

    for task_id in task_ids {
        let task = host.get_task(&task_id)?;
        let changed_files = changed_files_for_task(&workspace_path, &task)?;
        if changed_files.is_empty() {
            skipped_task_ids.push(task_id);
            continue;
        }

        stage_paths(&workspace_path, &changed_files)?;
        let staged_files = staged_changed_files(&workspace_path)?;
        if staged_files.is_empty() {
            skipped_task_ids.push(task.id);
            continue;
        }

        let message = task_commit_message(&task);
        git_commit_with_identity(&workspace_path, &message, resolved_model.as_deref())?;
        committed_task_ids.push(task.id);
    }

    Ok(json!({
        "workspace_path": workspace_path.to_string_lossy().to_string(),
        "committed_task_ids": committed_task_ids,
        "skipped_task_ids": skipped_task_ids,
    }))
}

pub(super) fn commit_finalize_artifact_changes<H: TaskHost + DeterministicActionHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = required_job_run_id(input, "commit_finalize_artifact_changes")?;
    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id), None, None)?;
    if batch_tasks.is_empty() {
        return Ok(json!({}));
    }

    let workspace_path = resolve_workspace_path(host, input, batch_id)?;
    ensure_named_branch(&workspace_path)?;
    ensure_no_unmerged_changes(&workspace_path)?;

    let changed_files = collect_worktree_changes(&workspace_path)?;
    if changed_files.is_empty() {
        return Ok(json!({}));
    }

    let mut affected_tasks = Vec::new();
    let mut files_to_commit = BTreeSet::new();
    for task in batch_tasks {
        let task_files = filter_changed_files_for_task(&changed_files, &workspace_path, &task);
        if task_files.is_empty() {
            continue;
        }
        files_to_commit.extend(task_files);
        affected_tasks.push(task);
    }

    if affected_tasks.is_empty() {
        return Ok(json!({}));
    }

    let files_to_commit: Vec<String> = files_to_commit.into_iter().collect();
    stage_paths(&workspace_path, &files_to_commit)?;
    let staged_files = staged_changed_files(&workspace_path)?;
    if staged_files.is_empty() {
        return Ok(json!({}));
    }

    let mut message = finalize_commit_message(&affected_tasks);
    let (_, coauthors) = commit_author_for_tasks(&affected_tasks);
    append_co_author_trailers(&mut message, &coauthors);
    let resolved_model = host.resolved_crew_model(batch_id)?;
    git_commit_with_identity(&workspace_path, &message, resolved_model.as_deref())?;

    Ok(json!({
        "workspace_path": workspace_path.to_string_lossy().to_string(),
        "committed_task_ids": affected_tasks.into_iter().map(|task| task.id).collect::<Vec<_>>(),
        "committed_files": staged_files,
    }))
}

pub(super) fn commit_batch_changes<H: TaskHost + DeterministicActionHost + ?Sized>(
    host: &H,
    input: &Value,
) -> Result<Value, OrbitError> {
    let batch_id = required_job_run_id(input, "commit_batch_changes")?;
    let batch_tasks = host.list_tasks_filtered(None, None, None, Some(batch_id), None, None)?;
    let [task] = batch_tasks.as_slice() else {
        return Err(OrbitError::InvalidInput(format!(
            "commit_batch_changes expected exactly one task for job_run_id '{batch_id}', got {}",
            batch_tasks.len()
        )));
    };

    // ORB-10313: fail closed on the durable execution outcome before resolving
    // the delivery checkout, staging files, mutating the index, or committing.
    reject_failed_delivery(task)?;

    let workspace_path = resolve_workspace_path(host, input, batch_id)?;
    ensure_named_branch(&workspace_path)?;

    ensure_no_unmerged_changes(&workspace_path)?;

    // ADR-0219: an explicitly side-effect-only task skips this phase instead of
    // failing it. Read the tag before any gate so the carve-out is reachable
    // from every failure branch below, not just the empty-stage one (ORB-10380).
    let no_diff_expected = task.tags.iter().any(|tag| tag == NO_DIFF_EXPECTED_TAG);

    let checkpoint = match resolve_base_checkpoint(&workspace_path, input)? {
        BaseResolution::Reconciled(checkpoint) => Some(checkpoint),
        BaseResolution::Unpinned => None,
        BaseResolution::UnrelatedHistories { base_sha, head_sha } => {
            if no_diff_expected {
                return Ok(skipped_no_diff_expected_result(&task.id));
            }
            return Err(unrelated_history_error(
                &task.id,
                &workspace_path,
                &base_sha,
                &head_sha,
            ));
        }
    };
    let (base_sha, mut commit_shas) = match checkpoint {
        Some(checkpoint) => {
            warn_on_commits_above_pinned_base(&task.id, &workspace_path, &checkpoint);
            (Some(checkpoint.base_sha), checkpoint.commit_shas)
        }
        None => (None, Vec::new()),
    };

    git_success(&workspace_path, &["add", "--all", "--", "."])?;

    let changed_files = staged_changed_files(&workspace_path)?;
    if changed_files.is_empty() {
        if !commit_shas.is_empty() {
            let commit_sha = git_output(&workspace_path, &["rev-parse", "HEAD"])?;
            let mut result = json!({
                "phase": "commit",
                "decision": "adopted_existing_commits",
                "committed": false,
                "adopted_commits": true,
                "commit_sha": commit_sha.trim(),
                "commit_shas": commit_shas,
                "job_run_id": batch_id,
                "skipped_no_diff_expected": false,
                "task_id": task.id,
            });
            if let Some(base_sha) = base_sha {
                result["base_sha"] = json!(base_sha);
            }
            return Ok(result);
        }

        // ORB-10380: no failure path mutates worktree state on its way out.
        // `git add --all` staged nothing here, so the index already matches
        // HEAD and the former `git reset HEAD` was both pointless and a
        // mutation performed while erroring.
        if no_diff_expected {
            return Ok(skipped_no_diff_expected_result(&task.id));
        }
        return Err(empty_stage_error(
            &task.id,
            &workspace_path,
            base_sha.as_deref(),
        )?);
    }

    let message = batch_commit_message(task);
    let resolved_model = host.resolved_crew_model(batch_id)?;

    git_commit_with_identity(&workspace_path, &message, resolved_model.as_deref())?;
    let commit_sha = git_output(&workspace_path, &["rev-parse", "HEAD"])?;
    commit_shas.push(commit_sha.trim().to_string());
    let mut result = json!({
        "phase": "commit",
        "decision": "performed",
        "committed": true,
        "adopted_commits": commit_shas.len() > 1,
        "commit_sha": commit_sha.trim(),
        "commit_shas": commit_shas,
        "job_run_id": batch_id,
        "skipped_no_diff_expected": false,
        "task_id": task.id,
    });
    if let Some(base_sha) = base_sha {
        result["base_sha"] = json!(base_sha);
    }
    Ok(result)
}

/// Commit a terminally-failed shipment's dirty candidate without consulting
/// the normal success-summary delivery gate. ADR-0246 confines this bypass to
/// the failure handoff, which blocks rather than promotes the task.
pub(super) fn commit_failure_candidate<H: DeterministicActionHost + ?Sized>(
    host: &H,
    run_id: &str,
    workspace_path: &Path,
    task: &orbit_common::types::Task,
) -> Result<(String, Vec<String>), OrbitError> {
    ensure_named_branch(workspace_path)?;
    ensure_no_unmerged_changes(workspace_path)?;
    git_success(workspace_path, &["add", "--all", "--", "."])?;
    let changed_files = staged_changed_files(workspace_path)?;
    if !changed_files.is_empty() {
        let message = batch_commit_message(task);
        let resolved_model = host.resolved_crew_model(run_id)?;
        git_commit_with_identity(workspace_path, &message, resolved_model.as_deref())?;
    }
    let head_sha = git_output(workspace_path, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    Ok((head_sha.trim().to_string(), changed_files))
}

/// The pinned setup-time base reconciled against this worktree's HEAD.
struct BaseCheckpoint {
    /// The commit `worktree_setup` created this worktree at, verified present.
    base_sha: String,
    /// Commits reachable from HEAD but not from the base the count started at.
    commit_shas: Vec<String>,
    /// Set when HEAD is not a descendant of `base_sha`; holds the merge base
    /// the commit count fell back to.
    diverged_at: Option<String>,
}

/// Outcome of reconciling `input.base_sha` with the worktree's HEAD.
enum BaseResolution {
    /// No base was pinned by the caller, so this step attributes no history.
    Unpinned,
    Reconciled(BaseCheckpoint),
    /// The pinned base and HEAD have no common ancestor.
    UnrelatedHistories {
        base_sha: String,
        head_sha: String,
    },
}

/// Reconcile the base checkpoint `worktree_setup` pinned for this run.
///
/// ORB-10380: the input is a commit id resolved once at worktree creation, not
/// a ref name. `refs/remotes/origin/<base>` is shared by every worktree off the
/// one `.git`, so any sibling run's fetch or any merge moves it mid-run; a
/// commit step that re-resolved the name failed every older in-flight run by
/// construction. Nothing here resolves a ref.
fn resolve_base_checkpoint(
    workspace_path: &Path,
    input: &Value,
) -> Result<BaseResolution, OrbitError> {
    let Some(pinned) = input_string_field(input, "base_sha") else {
        return Ok(BaseResolution::Unpinned);
    };
    let pinned = pinned_object_id(&pinned)?;

    let base_sha = git_output(
        workspace_path,
        &["rev-parse", "--verify", &format!("{pinned}^{{commit}}")],
    )
    .map_err(|error| {
        OrbitError::Execution(format!(
            "commit_batch_changes: pinned base commit '{pinned}' is not present in worktree \
             '{}': {error}",
            workspace_path.display()
        ))
    })?;
    let head_sha = git_output(workspace_path, &["rev-parse", "--verify", "HEAD^{commit}"])?;

    if git_command_success(
        workspace_path,
        &["merge-base", "--is-ancestor", &base_sha, "HEAD"],
    )? {
        let commit_shas = commits_since(workspace_path, &base_sha)?;
        return Ok(BaseResolution::Reconciled(BaseCheckpoint {
            base_sha,
            commit_shas,
            diverged_at: None,
        }));
    }

    // The branch was rewritten or reset below the pinned base after setup. Count
    // this run's commits from the common ancestor instead of failing: only a
    // genuinely unrelated history leaves nothing to attribute.
    if !git_command_success(workspace_path, &["merge-base", &base_sha, "HEAD"])? {
        return Ok(BaseResolution::UnrelatedHistories { base_sha, head_sha });
    }
    let merge_base = git_output(workspace_path, &["merge-base", &base_sha, "HEAD"])?;
    let commit_shas = commits_since(workspace_path, &merge_base)?;
    tracing::warn!(
        workspace_path = %workspace_path.display(),
        base_sha = %base_sha,
        head_sha = %head_sha,
        merge_base = %merge_base,
        commit_count = commit_shas.len(),
        "task branch is not a descendant of its pinned base; counting commits from the merge base"
    );
    Ok(BaseResolution::Reconciled(BaseCheckpoint {
        base_sha,
        commit_shas,
        diverged_at: Some(merge_base),
    }))
}

/// Reject anything that is not a full Git object id.
///
/// The commit step's contract is a base pinned at worktree setup; accepting a
/// ref name here would silently restore the moving-base failure (ORB-10380).
fn pinned_object_id(value: &str) -> Result<String, OrbitError> {
    let candidate = value.trim();
    let is_object_id =
        matches!(candidate.len(), 40 | 64) && candidate.chars().all(|c| c.is_ascii_hexdigit());
    if !is_object_id {
        return Err(OrbitError::InvalidInput(format!(
            "git_commit: input.base_sha must be the full commit id pinned by worktree_setup, got \
             '{value}'; the commit step never resolves a ref name because the shared base ref \
             moves while a run is in flight"
        )));
    }
    Ok(candidate.to_ascii_lowercase())
}

fn commits_since(workspace_path: &Path, from: &str) -> Result<Vec<String>, OrbitError> {
    let range = format!("{from}..HEAD");
    Ok(
        git_output(workspace_path, &["rev-list", "--reverse", &range])?
            .lines()
            .map(str::trim)
            .filter(|sha| !sha.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    )
}

/// Commits above the pinned base mean some actor committed inside the assigned
/// worktree during implementation. The pipeline owns Git context, so no
/// sanctioned actor does that today; the known live source is an external
/// editor `Stop` hook that auto-commits. Adopting them preserves the work and
/// its authorship, which is strictly better than discarding or rewriting it —
/// but it is an interim, so it is loud. Revisit once implementing agents and
/// their host hooks are provably non-committing (ORB-10381 owns the activity
/// contract); at that point this becomes a hard failure instead of a warning.
fn warn_on_commits_above_pinned_base(
    task_id: &str,
    workspace_path: &Path,
    checkpoint: &BaseCheckpoint,
) {
    if checkpoint.commit_shas.is_empty() {
        return;
    }
    tracing::warn!(
        task_id,
        workspace_path = %workspace_path.display(),
        base_sha = %checkpoint.base_sha,
        diverged_at = checkpoint.diverged_at.as_deref(),
        commit_count = checkpoint.commit_shas.len(),
        commit_shas = ?checkpoint.commit_shas,
        "adopting commits authored in the assigned worktree above the pinned base; the pipeline \
         owns Git context, so these came from an actor outside the commit step"
    );
}

fn skipped_no_diff_expected_result(task_id: &str) -> Value {
    json!({
        "phase": "commit",
        "decision": "skipped_no_diff_expected",
        "committed": false,
        "skipped_no_diff_expected": true,
        "task_id": task_id,
    })
}

/// The worktree carries no committable work. Reports only what was observed —
/// this message shares no wording with [`unrelated_history_error`] so a reader
/// can tell the two conditions apart (ORB-10380).
fn empty_stage_error(
    task_id: &str,
    workspace_path: &Path,
    base_sha: Option<&str>,
) -> Result<OrbitError, OrbitError> {
    let counts = worktree_status_counts(workspace_path)?;
    let head_sha = git_output(workspace_path, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let base = match base_sha {
        Some(base_sha) => format!("pinned base {base_sha}"),
        None => "no pinned base in this step's input".to_string(),
    };
    Ok(OrbitError::Execution(format!(
        "commit_batch_changes: nothing to commit for task '{task_id}' in worktree '{}'. \
         Observed after `git add --all`: {} staged, {} unstaged, {} untracked file(s); \
         HEAD {head_sha}; {base}; 0 commit(s) above it. Orbit did not inspect, stage, reset, \
         or reconcile any other checkout",
        workspace_path.display(),
        counts.staged,
        counts.unstaged,
        counts.untracked
    )))
}

/// The pinned base and this worktree's HEAD are disjoint histories, so no
/// commit range describes what this run authored.
fn unrelated_history_error(
    task_id: &str,
    workspace_path: &Path,
    base_sha: &str,
    head_sha: &str,
) -> OrbitError {
    OrbitError::Execution(format!(
        "commit_batch_changes: task '{task_id}' worktree '{}' shares no history with the base it \
         was created at. Observed: pinned base {base_sha}; HEAD {head_sha}; `git merge-base` \
         reports no common ancestor. Orbit did not inspect, stage, reset, or reconcile any \
         other checkout",
        workspace_path.display()
    ))
}

#[derive(Default)]
struct WorktreeStatusCounts {
    staged: usize,
    unstaged: usize,
    untracked: usize,
}

fn worktree_status_counts(workspace_path: &Path) -> Result<WorktreeStatusCounts, OrbitError> {
    let status = git_output(
        workspace_path,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    let mut counts = WorktreeStatusCounts::default();
    for line in status.lines() {
        let mut code = line.chars();
        let (Some(index_state), Some(worktree_state)) = (code.next(), code.next()) else {
            continue;
        };
        if index_state == '?' {
            counts.untracked += 1;
            continue;
        }
        if index_state != ' ' {
            counts.staged += 1;
        }
        if worktree_state != ' ' {
            counts.unstaged += 1;
        }
    }
    Ok(counts)
}

fn resolve_workspace_path<H: DeterministicActionHost + ?Sized>(
    host: &H,
    input: &Value,
    batch_id: &str,
) -> Result<PathBuf, OrbitError> {
    match input_string_field(input, "workspace_path") {
        Some(ws) => canonicalize_existing_dir(&ws, "workspace_path"),
        None => {
            let repo_root_str = host.repo_root()?;
            let repo_root = Path::new(&repo_root_str);
            super::worktree::resolve_shared_worktree_path(repo_root, batch_id)
        }
    }
}

fn completed_task_ids_field(input: &Value) -> Option<Vec<String>> {
    let items = input.get("completed_task_ids")?.as_array()?;
    Some(
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests;
