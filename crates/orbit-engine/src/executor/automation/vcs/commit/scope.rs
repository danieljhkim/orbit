use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use orbit_common::types::{OrbitError, Task};
use orbit_common::utility::selector::anchor_path;

use super::super::git::git_output_paths;

pub(super) fn changed_files_for_task(
    workspace_path: &Path,
    task: &Task,
) -> Result<Vec<String>, OrbitError> {
    let changed_files = collect_worktree_changes(workspace_path)?;
    Ok(filter_changed_files_for_task(
        &changed_files,
        workspace_path,
        task,
    ))
}

pub(super) fn filter_changed_files_for_task(
    changed_files: &BTreeSet<String>,
    workspace_path: &Path,
    task: &Task,
) -> Vec<String> {
    let scopes = task_scopes(task, workspace_path);
    if scopes.is_empty() {
        return Vec::new();
    }

    changed_files
        .iter()
        .filter(|file| scopes.iter().any(|scope| path_matches_scope(file, scope)))
        .cloned()
        .collect()
}

pub(super) fn collect_worktree_changes(
    workspace_path: &Path,
) -> Result<BTreeSet<String>, OrbitError> {
    let mut files = BTreeSet::new();
    for path in git_output_paths(
        workspace_path,
        &["diff", "--name-only", "-z", "--relative", "HEAD", "--"],
    )? {
        files.insert(path);
    }
    for path in git_output_paths(
        workspace_path,
        &["ls-files", "--others", "--exclude-standard", "-z", "--"],
    )? {
        files.insert(path);
    }
    Ok(files)
}

fn task_scopes(task: &Task, workspace_path: &Path) -> Vec<String> {
    task.context_files
        .iter()
        .filter_map(|raw| normalize_task_scope(raw, workspace_path))
        .collect()
}

pub(super) fn normalize_task_scope(raw: &str, workspace_path: &Path) -> Option<String> {
    let anchor = anchor_path(raw).ok()?;
    let relative = if anchor.is_absolute() {
        anchor.strip_prefix(workspace_path).ok()?.to_path_buf()
    } else {
        anchor
    };
    normalize_relative_path(&relative)
}

fn normalize_relative_path(path: &Path) -> Option<String> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    let value = normalized.to_string_lossy().replace('\\', "/");
    (!value.is_empty()).then_some(value)
}

pub(super) fn path_matches_scope(path: &str, scope: &str) -> bool {
    path == scope
        || scope == "."
        || path
            .strip_prefix(scope)
            .is_some_and(|suffix| suffix.starts_with('/'))
}
