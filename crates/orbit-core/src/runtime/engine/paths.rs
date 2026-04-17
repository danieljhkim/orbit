use orbit_types::{OrbitError, WorkspacePaths};

use crate::OrbitRuntime;

pub(super) fn current_repo_root(runtime: &OrbitRuntime) -> Result<String, OrbitError> {
    Ok(runtime
        .context
        .paths()
        .repo_root
        .to_string_lossy()
        .to_string())
}

pub(super) fn codex_workspace_write_writable_dirs(paths: &WorkspacePaths) -> Vec<String> {
    let mut dirs = Vec::new();
    for dir in [&paths.orbit_dir, &paths.global_dir] {
        let dir = dir.to_string_lossy().into_owned();
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}
