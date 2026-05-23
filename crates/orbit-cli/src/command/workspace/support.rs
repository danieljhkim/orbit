use std::path::Path;

use orbit_core::OrbitError;

/// Remove all symlinks in a directory (non-recursive).
pub(super) fn remove_symlinks_in(dir: &Path) -> Result<(), OrbitError> {
    let entries = std::fs::read_dir(dir).map_err(|e| OrbitError::Io(e.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|e| OrbitError::Io(e.to_string()))?;
        let meta =
            std::fs::symlink_metadata(entry.path()).map_err(|e| OrbitError::Io(e.to_string()))?;
        if meta.file_type().is_symlink() {
            std::fs::remove_file(entry.path()).map_err(|e| OrbitError::Io(e.to_string()))?;
        }
    }
    Ok(())
}

/// Check if a directory is empty.
pub(super) fn is_dir_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false)
}

pub(super) fn dir_name_or_fallback(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("default")
        .to_string()
}

pub(super) fn detect_git_remote(cwd: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

pub(super) fn ensure_orbit_gitignore_entry(
    workspace_root: &Path,
    orbit_dir: &Path,
) -> Result<(), OrbitError> {
    let Some(gitignore_root) = orbit_gitignore_root(workspace_root, orbit_dir) else {
        return Ok(());
    };
    let gitignore_path = gitignore_root.join(".gitignore");
    write_orbit_gitignore_entry(&gitignore_path)
}

fn orbit_gitignore_root<'a>(workspace_root: &'a Path, orbit_dir: &'a Path) -> Option<&'a Path> {
    // Legacy: walking up from a subdir, orbit_dir is `<repo>/.orbit` whose
    // parent is a git repo root.
    if orbit_dir.file_name().and_then(|name| name.to_str()) == Some(".orbit")
        && let Some(repo_root) = orbit_dir.parent()
        && is_git_repo_root(repo_root)
    {
        return Some(repo_root);
    }

    // Default: orbit_dir lives directly inside workspace_root as `.orbit`.
    // If the user passed `--root` to relocate Orbit data outside the workspace
    // (or to a non-`.orbit` basename), skip the gitignore write — there is no
    // `<workspace>/.orbit` directory to ignore.
    if is_git_repo_root(workspace_root) && orbit_dir == workspace_root.join(".orbit") {
        return Some(workspace_root);
    }

    None
}

fn is_git_repo_root(path: &Path) -> bool {
    path.join(".git").exists()
}

fn write_orbit_gitignore_entry(gitignore_path: &Path) -> Result<(), OrbitError> {
    let content = match std::fs::read_to_string(gitignore_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(OrbitError::Io(error.to_string())),
    };

    if gitignore_has_orbit_entry(&content) {
        return Ok(());
    }

    let mut next = content;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(".orbit\n");
    std::fs::write(gitignore_path, next).map_err(|error| OrbitError::Io(error.to_string()))
}

fn gitignore_has_orbit_entry(content: &str) -> bool {
    content.lines().any(|line| {
        let line = line.trim();
        matches!(line, ".orbit" | ".orbit/" | "/.orbit" | "/.orbit/")
    })
}
