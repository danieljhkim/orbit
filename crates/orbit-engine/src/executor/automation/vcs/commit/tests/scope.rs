use tempfile::tempdir;

use super::super::scope::{normalize_task_scope, path_matches_scope};

#[test]
fn normalize_task_scope_uses_selector_anchor_paths() {
    let temp = tempdir().unwrap();
    let workspace = temp.path();
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::write(workspace.join("src/lib.rs"), "pub fn run() {}\n").unwrap();

    assert_eq!(
        normalize_task_scope("symbol:src/lib.rs#run:function", workspace).as_deref(),
        Some("src/lib.rs")
    );
    assert_eq!(
        normalize_task_scope("dir:src", workspace).as_deref(),
        Some("src")
    );
    assert_eq!(
        normalize_task_scope(&workspace.join("src/lib.rs").to_string_lossy(), workspace).as_deref(),
        Some("src/lib.rs")
    );
}

#[test]
fn path_matches_scope_handles_directory_scopes() {
    assert!(path_matches_scope("src/lib.rs", "src"));
    assert!(path_matches_scope("src/lib.rs", "src/lib.rs"));
    assert!(!path_matches_scope("tests/lib.rs", "src"));
}
