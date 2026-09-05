//! Landing-branch source snapshot for task-pilot prepare/apply.
//!
//! Inspection and deterministic selector validation share one pinned revision
//! so a newly merged target is not reported as missing when the primary
//! checkout lags origin. The only Git write this module will perform is a
//! fast-forward of a clean primary that already sits on the landing branch.

use std::path::Path;
use std::process::Command;

use orbit_common::fs::git::{CurrentBranchStatus, current_branch, run_git};
use orbit_engine::DispatchError;
use serde_json::{Value, json};

use crate::OrbitRuntime;

use super::action_failed;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SourceSnapshot {
    pub base_branch: String,
    pub source_ref: String,
    pub source_revision: String,
    pub fast_forwarded: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GitPathKind {
    Blob,
    Tree,
    Missing,
    Other,
}

impl SourceSnapshot {
    pub(super) fn to_json(&self) -> Value {
        json!({
            "base_branch": self.base_branch,
            "source_ref": self.source_ref,
            "source_revision": self.source_revision,
            "fast_forwarded": self.fast_forwarded,
        })
    }

    pub(super) fn from_prepared(
        prepared: &Value,
        action: &str,
    ) -> Result<Option<Self>, DispatchError> {
        let Some(source) = prepared.get("source") else {
            return Ok(None);
        };
        if source.is_null() {
            return Ok(None);
        }
        let object = source
            .as_object()
            .ok_or_else(|| action_failed(action, "prepared.source must be an object or null"))?;
        let source_revision = object
            .get("source_revision")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(source_revision) = source_revision else {
            return Ok(None);
        };
        if !is_commit_id(source_revision) {
            return Err(action_failed(
                action,
                format!(
                    "prepared.source.source_revision {source_revision:?} is not a full commit id"
                ),
            ));
        }
        let base_branch = object
            .get("base_branch")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                action_failed(
                    action,
                    "prepared.source.base_branch must be a non-empty string",
                )
            })?;
        let source_ref = object
            .get("source_ref")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                action_failed(
                    action,
                    "prepared.source.source_ref must be a non-empty string",
                )
            })?;
        Ok(Some(Self {
            base_branch: base_branch.to_string(),
            source_ref: source_ref.to_string(),
            source_revision: source_revision.to_string(),
            fast_forwarded: object
                .get("fast_forwarded")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }))
    }

    pub(super) fn ensure_commit(
        &self,
        action: &str,
        workspace: &Path,
    ) -> Result<(), DispatchError> {
        let spec = format!("{}^{{commit}}", self.source_revision);
        let output = git(action, workspace, &["rev-parse", "--verify", &spec])?;
        if output.success {
            Ok(())
        } else {
            Err(action_failed(
                action,
                format!(
                    "prepared source revision {} ({}) is not a commit in this repository: {}",
                    self.source_revision,
                    self.source_ref,
                    output.stderr.trim()
                ),
            ))
        }
    }

    pub(super) fn path_kind(
        &self,
        action: &str,
        workspace: &Path,
        anchor: &Path,
    ) -> Result<GitPathKind, DispatchError> {
        let relative = git_tree_path(anchor).ok_or_else(|| {
            action_failed(
                action,
                format!(
                    "selector anchor {} is not a repository-relative path",
                    anchor.display()
                ),
            )
        })?;
        let object = format!("{}:{relative}", self.source_revision);
        let output = git(action, workspace, &["cat-file", "-t", &object])?;
        if !output.success {
            return Ok(GitPathKind::Missing);
        }
        Ok(match output.stdout.trim() {
            "blob" => GitPathKind::Blob,
            "tree" => GitPathKind::Tree,
            _ => GitPathKind::Other,
        })
    }
}

pub(super) fn resolve_source_snapshot(
    runtime: &OrbitRuntime,
    action: &str,
    input: &Value,
    workspace_root: &Path,
) -> Result<Option<SourceSnapshot>, DispatchError> {
    if !is_git_work_tree(action, workspace_root)? {
        return Ok(None);
    }

    let base_branch = requested_base_branch(runtime, input);
    let base_branch = normalize_base_branch(action, &base_branch)?;
    let (source_ref, fetched_remote) = if has_origin_remote(action, workspace_root)? {
        fetch_origin_branch(action, workspace_root, &base_branch)?;
        (format!("origin/{base_branch}"), true)
    } else {
        (base_branch.clone(), false)
    };
    let source_revision = rev_parse_commit(action, workspace_root, &source_ref)?;
    let head = rev_parse_commit(action, workspace_root, "HEAD")?;
    let fast_forwarded = align_clean_primary(
        action,
        workspace_root,
        &base_branch,
        &source_ref,
        &source_revision,
        &head,
        fetched_remote,
    )?;

    Ok(Some(SourceSnapshot {
        base_branch,
        source_ref,
        source_revision,
        fast_forwarded,
    }))
}

pub(super) fn requested_base_branch(runtime: &OrbitRuntime, input: &Value) -> String {
    input
        .get("base_branch")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| runtime.workflow_base_branch().to_string())
}

fn normalize_base_branch(action: &str, base: &str) -> Result<String, DispatchError> {
    let branch = base
        .trim()
        .strip_prefix("origin/")
        .unwrap_or_else(|| base.trim())
        .trim();
    if branch.is_empty() {
        return Err(action_failed(
            action,
            "base_branch must be a non-empty branch name",
        ));
    }
    if branch.starts_with('-') {
        return Err(action_failed(
            action,
            format!("base_branch {base:?} must not start with '-'"),
        ));
    }
    Ok(branch.to_string())
}

fn align_clean_primary(
    action: &str,
    workspace: &Path,
    base_branch: &str,
    source_ref: &str,
    source_revision: &str,
    head: &str,
    fetched_remote: bool,
) -> Result<bool, DispatchError> {
    if head == source_revision {
        return Ok(false);
    }

    let dirty = working_tree_status(action, workspace)?;
    let branch = current_branch(workspace)
        .map_err(|error| action_failed(action, format!("read current branch: {error}")))?;
    let on_landing_branch =
        matches!(branch, CurrentBranchStatus::Named(ref name) if name == base_branch);
    let can_fast_forward = is_ancestor(action, workspace, head, source_revision)?;

    if dirty.is_empty() && on_landing_branch && can_fast_forward {
        let merge = git(
            action,
            workspace,
            &["merge", "--ff-only", "--no-edit", source_revision],
        )?;
        if !merge.success {
            return Err(source_stale(
                action,
                base_branch,
                source_ref,
                source_revision,
                head,
                fetched_remote,
                &format!("clean fast-forward failed: {}", merge.stderr.trim()),
            ));
        }
        let after = rev_parse_commit(action, workspace, "HEAD")?;
        if after != source_revision {
            return Err(source_stale(
                action,
                base_branch,
                source_ref,
                source_revision,
                &after,
                fetched_remote,
                "fast-forward completed but HEAD is not the pinned source revision",
            ));
        }
        return Ok(true);
    }

    let reason = if !dirty.is_empty() {
        format!(
            "primary working tree is dirty or has untracked files ({}); refusing checkout, pull, or reset",
            dirty.join(", ")
        )
    } else if !on_landing_branch {
        format!(
            "primary is not on landing branch {base_branch} (current: {}); refusing to switch branches",
            match branch {
                CurrentBranchStatus::Named(name) => name,
                CurrentBranchStatus::DetachedHead => "detached HEAD".to_string(),
                CurrentBranchStatus::NoCurrentBranch => "no current branch".to_string(),
            }
        )
    } else if !can_fast_forward {
        "primary HEAD is not an ancestor of the landing-branch revision; refusing a non-fast-forward update"
            .to_string()
    } else {
        "primary is not at the verified landing-branch revision".to_string()
    };
    Err(source_stale(
        action,
        base_branch,
        source_ref,
        source_revision,
        head,
        fetched_remote,
        &reason,
    ))
}

fn source_stale(
    action: &str,
    base_branch: &str,
    source_ref: &str,
    source_revision: &str,
    head: &str,
    fetched_remote: bool,
    reason: &str,
) -> DispatchError {
    let sync_hint = if fetched_remote {
        format!(
            "make the primary clean and fast-forward `{base_branch}` to `{source_ref}` ({source_revision}) before piloting"
        )
    } else {
        format!("update the local `{base_branch}` checkout to {source_revision} before piloting")
    };
    action_failed(
        action,
        format!(
            "source-staleness: landing branch `{base_branch}` is {source_revision} at `{source_ref}`, primary HEAD is {head}. {reason}. {sync_hint}"
        ),
    )
}

fn is_git_work_tree(action: &str, workspace: &Path) -> Result<bool, DispatchError> {
    let output = git(action, workspace, &["rev-parse", "--is-inside-work-tree"])?;
    Ok(output.success && output.stdout.trim() == "true")
}

fn has_origin_remote(action: &str, workspace: &Path) -> Result<bool, DispatchError> {
    let remotes = git(action, workspace, &["remote"])?;
    if !remotes.success {
        return Ok(false);
    }
    Ok(remotes
        .stdout
        .lines()
        .map(str::trim)
        .any(|name| name == "origin"))
}

fn fetch_origin_branch(action: &str, workspace: &Path, branch: &str) -> Result<(), DispatchError> {
    let spec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
    let output = Command::new("git")
        .args(["fetch", "origin", &spec])
        .current_dir(workspace)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| {
            action_failed(
                action,
                format!(
                    "failed to fetch origin/{branch} in '{}': {error}",
                    workspace.display()
                ),
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(action_failed(
        action,
        format!(
            "remote failure: could not fetch origin/{branch} in '{}': {}",
            workspace.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    ))
}

fn rev_parse_commit(action: &str, workspace: &Path, rev: &str) -> Result<String, DispatchError> {
    let spec = format!("{rev}^{{commit}}");
    let output = git(action, workspace, &["rev-parse", "--verify", &spec])?;
    if !output.success {
        return Err(action_failed(
            action,
            format!(
                "unable to resolve source revision `{rev}`: {}",
                output.stderr.trim()
            ),
        ));
    }
    let sha = output.stdout.trim();
    if !is_commit_id(sha) {
        return Err(action_failed(
            action,
            format!("resolved `{rev}` to a non-commit id {sha:?}"),
        ));
    }
    Ok(sha.to_string())
}

fn is_ancestor(
    action: &str,
    workspace: &Path,
    ancestor: &str,
    descendant: &str,
) -> Result<bool, DispatchError> {
    let output = git(
        action,
        workspace,
        &["merge-base", "--is-ancestor", ancestor, descendant],
    )?;
    Ok(output.success)
}

fn working_tree_status(action: &str, workspace: &Path) -> Result<Vec<String>, DispatchError> {
    let output = git(action, workspace, &["status", "--porcelain=v1", "-uall"])?;
    if !output.success {
        return Err(action_failed(
            action,
            format!(
                "unable to read git status in '{}': {}",
                workspace.display(),
                output.stderr.trim()
            ),
        ));
    }
    Ok(output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(8)
        .map(ToOwned::to_owned)
        .collect())
}

fn git_tree_path(anchor: &Path) -> Option<String> {
    if anchor.is_absolute() {
        return None;
    }
    let mut parts = Vec::new();
    for component in anchor.components() {
        match component {
            std::path::Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn is_commit_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn git(
    action: &str,
    workspace: &Path,
    args: &[&str],
) -> Result<orbit_common::fs::git::GitCommandOutput, DispatchError> {
    run_git(workspace, args).map_err(|error| {
        action_failed(
            action,
            format!(
                "git {} failed in '{}': {error}",
                args.join(" "),
                workspace.display()
            ),
        )
    })
}
