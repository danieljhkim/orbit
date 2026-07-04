//! Tests for `check_workspace_boundary` (defined in `fs/mod.rs`): the
//! resolve-then-compare workspace containment gate every fs tool runs before
//! policy evaluation. [ORB-00418]

use orbit_common::types::OrbitError;

use crate::ToolContext;

use super::super::check_workspace_boundary;

fn ctx_with_root(root: &std::path::Path) -> ToolContext {
    ToolContext {
        workspace_root: Some(root.to_path_buf()),
        ..ToolContext::default()
    }
}

#[cfg(unix)]
#[test]
fn boundary_denies_symlink_escaping_workspace() {
    use std::os::unix::fs::symlink;

    let outside = tempfile::tempdir().expect("outside");
    std::fs::write(outside.path().join("target.txt"), b"x").expect("outside file");
    let workspace = tempfile::tempdir().expect("workspace");
    symlink(
        outside.path().join("target.txt"),
        workspace.path().join("link"),
    )
    .expect("symlink");

    let error = check_workspace_boundary(
        &ctx_with_root(workspace.path()),
        &workspace.path().join("link"),
    )
    .expect_err("symlink pointing outside the workspace must be denied");
    assert!(matches!(error, OrbitError::PolicyDenied(_)), "{error:?}");
}

#[cfg(unix)]
#[test]
fn boundary_denies_dangling_symlink_escaping_workspace() {
    use std::os::unix::fs::symlink;

    let outside = tempfile::tempdir().expect("outside");
    let workspace = tempfile::tempdir().expect("workspace");
    // Dangling link: target does not exist, but an O_CREAT open through the
    // link (fs::write / File::create) would create it *outside* the workspace.
    symlink(
        outside.path().join("planted.txt"),
        workspace.path().join("link"),
    )
    .expect("symlink");

    let error = check_workspace_boundary(
        &ctx_with_root(workspace.path()),
        &workspace.path().join("link"),
    )
    .expect_err("dangling symlink pointing outside the workspace must be denied");
    assert!(matches!(error, OrbitError::PolicyDenied(_)), "{error:?}");
}

#[test]
fn boundary_allows_missing_tail_inside_workspace() {
    let workspace = tempfile::tempdir().expect("workspace");

    let canonical = check_workspace_boundary(
        &ctx_with_root(workspace.path()),
        &workspace.path().join("new_dir/new_file.txt"),
    )
    .expect("a not-yet-existing in-workspace target must pass");
    assert!(canonical.ends_with("new_dir/new_file.txt"));
}
