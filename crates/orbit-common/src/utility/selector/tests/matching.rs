use std::path::PathBuf;

use tempfile::tempdir;

use super::super::*;

#[test]
fn anchor_path_extracts_symbol_file_path() {
    assert_eq!(
        anchor_path("symbol:src/lib.rs#run:function").unwrap(),
        PathBuf::from("src/lib.rs")
    );
}

#[test]
fn exists_in_workspace_uses_anchor_paths() {
    let temp = tempdir().unwrap();
    let workspace = temp.path();
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::write(workspace.join("src/lib.rs"), "pub fn ok() {}\n").unwrap();

    assert!(exists_in_workspace(
        "symbol:src/lib.rs#run:function",
        workspace
    ));
    assert!(!exists_in_workspace(
        "symbol:src/missing.rs#run:function",
        workspace
    ));
}

#[test]
fn overlaps_uses_anchor_semantics() {
    assert!(overlaps("symbol:f.rs#a:method", "symbol:f.rs#b:method"));
    assert!(overlaps("dir:src", "file:src/lib.rs"));
    assert!(overlaps("src", "file:src/lib.rs"));
    assert!(!overlaps("file:f.rs", "file:g.rs"));
    assert!(!overlaps("dir:src", "file:lib/y.rs"));
}

#[test]
fn shared_anchor_prefix_depth_ignores_selector_metadata() {
    assert_eq!(
        shared_anchor_prefix_depth("symbol:src/lib.rs#alpha:function", "file:src/nested/mod.rs"),
        1
    );
}
