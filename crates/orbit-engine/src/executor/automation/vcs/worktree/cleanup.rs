use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use orbit_common::types::OrbitError;
use orbit_store::{IdAllocationKind, IdAllocationRecord, with_active_id_allocations};

use super::super::git::{git_output, git_success};

/// Shared sanctioned worktree removal sequence. Pipeline cleanup may force
/// removal because it owns the just-created checkout; out-of-band GC must
/// always pass `force = false`.
pub(super) fn remove_worktree(
    repo_root: &Path,
    workspace_path: &Path,
    branch_name: Option<&str>,
    force: bool,
) -> Result<(), OrbitError> {
    if workspace_path.exists() {
        let shared_orbit_root = shared_orbit_root(repo_root)?;
        with_active_id_allocations(
            &shared_orbit_root.join("state/semantic.db"),
            &shared_orbit_root.join("state/.id_alloc.lock"),
            |allocations| {
                ensure_knowledge_bodies_recoverable(repo_root, workspace_path, allocations)?;
                let workspace_path = workspace_path.to_string_lossy();
                if force {
                    git_success(
                        repo_root,
                        &["worktree", "remove", "--force", workspace_path.as_ref()],
                    )?;
                } else {
                    git_success(repo_root, &["worktree", "remove", workspace_path.as_ref()])?;
                }
                Ok(())
            },
        )?;
    }
    git_success(repo_root, &["worktree", "prune"])?;
    if let Some(branch_name) = branch_name {
        git_success(repo_root, &["branch", "-D", branch_name])?;
    }
    Ok(())
}

/// F2026-07-094 / ORB-10535: removing a worktree must not turn its locally
/// readable learning or ADR into the orphan that ORB-10501 can only diagnose
/// and retire after the body is already lost.
fn ensure_knowledge_bodies_recoverable(
    repo_root: &Path,
    workspace_path: &Path,
    allocations: &[IdAllocationRecord],
) -> Result<(), OrbitError> {
    let removed_root = fs::canonicalize(workspace_path).map_err(|error| {
        OrbitError::Execution(format!(
            "failed to resolve worktree '{}' before knowledge-artifact preflight: {error}",
            workspace_path.display()
        ))
    })?;
    let durable_roots = registered_worktree_roots(repo_root)?;
    let mut blocked = Vec::new();

    for allocation in allocations {
        if allocation.kind == IdAllocationKind::Adr {
            // ADR allocations are historical rows from the retired store and
            // no longer participate in worktree body-loss protection.
            continue;
        }
        if canonical_path(&allocation.worktree_root).as_deref() != Some(removed_root.as_path()) {
            continue;
        }
        let local_bodies = readable_allocation_bodies(allocation, &removed_root)?;
        if local_bodies.is_empty() {
            // A reserved row with no body is ORB-10501's repair concern, not a
            // unique-body loss. Do not duplicate that orphan repair here.
            continue;
        }

        let mut durable_candidates = allocation_body_candidates(allocation, None)?;
        for root in &durable_roots {
            if root != &removed_root {
                durable_candidates.extend(allocation_body_candidates(allocation, Some(root))?);
            }
        }
        let body_is_durable = durable_candidates.into_iter().any(|candidate| {
            let Some(candidate_root) = canonical_path(&candidate) else {
                return false;
            };
            if candidate_root.starts_with(&removed_root) {
                return false;
            }
            let Ok(candidate_body) = fs::read(candidate_root) else {
                return false;
            };
            !candidate_body.is_empty()
                && local_bodies
                    .iter()
                    .any(|local_body| local_body == &candidate_body)
        });
        if !body_is_durable {
            blocked.push(format!("{} {}", allocation.kind.as_str(), allocation.id));
        }
    }

    if blocked.is_empty() {
        return Ok(());
    }
    Err(OrbitError::Execution(format!(
        "refusing to remove worktree '{}': it contains the only readable body for these learning allocations: {}. Reconcile each body into another registered worktree (normally the canonical checkout), verify it there with `orbit learning show <id>`, then retry cleanup",
        workspace_path.display(),
        blocked.join(", ")
    )))
}

fn readable_allocation_bodies(
    allocation: &IdAllocationRecord,
    removed_root: &Path,
) -> Result<Vec<Vec<u8>>, OrbitError> {
    let mut bodies = Vec::new();
    let mut candidates = allocation_body_candidates(allocation, None)?;
    candidates.extend(allocation_body_candidates(allocation, Some(removed_root))?);
    for candidate in candidates {
        let Some(path) = canonical_path(&candidate) else {
            continue;
        };
        if !path.starts_with(removed_root) {
            continue;
        }
        if let Ok(body) = fs::read(path)
            && !body.is_empty()
            && !bodies.contains(&body)
        {
            bodies.push(body);
        }
    }
    Ok(bodies)
}

/// Candidate with `root = None` is the allocator's recorded body path. A
/// supplied root probes the canonical artifact layout in that registered
/// checkout, including a body written just before its allocation metadata was
/// finalized.
fn allocation_body_candidates(
    allocation: &IdAllocationRecord,
    root: Option<&Path>,
) -> Result<Vec<PathBuf>, OrbitError> {
    let Some(root) = root else {
        return Ok(allocation.resolved_body_path().into_iter().collect());
    };
    match allocation.kind {
        IdAllocationKind::Learning => Ok(vec![
            root.join(".orbit/learnings")
                .join(&allocation.id)
                .join("learning.yaml"),
        ]),
        IdAllocationKind::Adr => Ok(Vec::new()),
    }
}

fn registered_worktree_roots(repo_root: &Path) -> Result<BTreeSet<PathBuf>, OrbitError> {
    let list = git_output(repo_root, &["worktree", "list", "--porcelain"])?;
    Ok(list
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .filter_map(|path| canonical_path(Path::new(path)))
        .collect())
}

fn shared_orbit_root(repo_root: &Path) -> Result<PathBuf, OrbitError> {
    let common_dir = git_output(
        repo_root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common_dir = PathBuf::from(common_dir);
    let main_root = common_dir.parent().ok_or_else(|| {
        OrbitError::Execution(format!(
            "cannot derive the shared Orbit root from Git common directory '{}'",
            common_dir.display()
        ))
    })?;
    Ok(main_root.join(".orbit"))
}

fn canonical_path(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok()
}
