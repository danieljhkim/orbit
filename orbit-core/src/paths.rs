use std::path::{Component, Path, PathBuf};

use orbit_types::OrbitError;

pub(crate) const ORBIT_ROOT_TOKEN: &str = "{{ORBIT_ROOT}}";

pub(crate) fn home_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME")
        && !home.trim().is_empty()
    {
        return Some(PathBuf::from(home));
    }
    if let Ok(profile) = std::env::var("USERPROFILE")
        && !profile.trim().is_empty()
    {
        return Some(PathBuf::from(profile));
    }
    None
}

pub(crate) fn cwd_orbit_root(cwd: &Path) -> PathBuf {
    normalize_path_components(&cwd.join(".orbit"))
}

pub(crate) fn current_dir_orbit_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    cwd_orbit_root(&cwd)
}

pub(crate) fn resolve_path_value(
    raw: &str,
    base_dir: &Path,
    field_name: &str,
) -> Result<PathBuf, OrbitError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(OrbitError::InvalidInput(format!(
            "{field_name} must not be empty"
        )));
    }
    if value == "~" || value.starts_with("~/") {
        let home = home_dir().ok_or_else(|| {
            OrbitError::InvalidInput(
                "cannot expand '~' because HOME/USERPROFILE is not set".to_string(),
            )
        })?;
        let suffix = value.trim_start_matches("~/");
        return Ok(normalize_path_components(&home.join(suffix)));
    }
    let path = PathBuf::from(value);
    if path.is_relative() {
        return Ok(normalize_path_components(&base_dir.join(path)));
    }
    Ok(normalize_path_components(&path))
}

pub(crate) fn find_git_repo_root(start: &Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        let git_path = ancestor.join(".git");
        if git_path.is_dir() {
            return Some(ancestor.to_path_buf());
        }
        if git_path.is_file() {
            // Git worktree: .git is a file pointing to the main repo's gitdir.
            // Follow the pointer so orbit tool calls from within a worktree resolve
            // to the main repo's .orbit directory rather than creating a new one.
            if let Some(main_root) = resolve_main_repo_from_worktree_gitfile(&git_path) {
                return Some(main_root);
            }
        }
    }
    None
}

/// Parses a worktree `.git` file to find the main repository root.
///
/// A worktree `.git` file contains a single line:
///   `gitdir: /path/to/main/repo/.git/worktrees/<name>`
///
/// The main repo root is three levels up from that path.
fn resolve_main_repo_from_worktree_gitfile(git_file: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(git_file).ok()?;
    let raw_path = content.strip_prefix("gitdir:")?.trim();
    let gitdir = if Path::new(raw_path).is_absolute() {
        PathBuf::from(raw_path)
    } else {
        git_file.parent()?.join(raw_path)
    };
    // gitdir = /main/repo/.git/worktrees/<name>
    // repo root = gitdir/../../../  (up past worktrees/, .git/, repo/)
    let repo_root = gitdir.parent()?.parent()?.parent()?;
    if repo_root.join(".git").is_dir() {
        Some(repo_root.to_path_buf())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_git_repo_root_returns_main_repo_from_worktree() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Set up a fake main repo with a .git directory.
        let main_repo = dir.path().join("main");
        std::fs::create_dir_all(main_repo.join(".git/worktrees/task-branch"))
            .expect("create gitdir");

        // Set up a fake worktree directory with a .git file pointing back.
        let worktree = dir.path().join("worktrees/task-branch");
        std::fs::create_dir_all(&worktree).expect("create worktree");
        let gitdir_target = main_repo.join(".git/worktrees/task-branch");
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", gitdir_target.display()),
        )
        .expect("write .git file");

        // A subdirectory inside the worktree (simulates agent CWD).
        let cwd = worktree.join("src");
        std::fs::create_dir_all(&cwd).expect("create cwd");

        let root = find_git_repo_root(&cwd);
        assert_eq!(
            root,
            Some(main_repo),
            "should resolve to main repo, not worktree"
        );
    }

    #[test]
    fn find_git_repo_root_returns_none_for_non_git_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cwd = dir.path().join("workspace");
        std::fs::create_dir_all(&cwd).expect("create cwd");
        assert_eq!(find_git_repo_root(&cwd), None);
    }
}

pub(crate) fn normalize_path_components(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::CurDir) {
            continue;
        }
        normalized.push(component.as_os_str());
    }
    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}
