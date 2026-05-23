//! Tests for `require_relative_file_paths` and git path handling.

use super::super::*;
use std::fs;

use serde_json::json;
use tempfile::TempDir;

#[test]
fn relative_file_paths_accept_scalar_string() {
    let repo = repo_with_file("src/lib.rs");
    let repo_root = repo.path().canonicalize().unwrap();
    let paths = require_relative_file_paths(&json!({"files":"src/lib.rs"}), &repo_root)
        .expect("scalar file path is accepted");

    assert_eq!(paths, vec!["src/lib.rs"]);
}

#[test]
fn relative_file_paths_keep_array_behavior() {
    let repo = repo_with_file("src/lib.rs");
    fs::write(repo.path().join("README.md"), "hello\n").unwrap();
    let repo_root = repo.path().canonicalize().unwrap();
    let paths =
        require_relative_file_paths(&json!({"files":["src/lib.rs", "README.md"]}), &repo_root)
            .expect("array file paths are accepted");

    assert_eq!(paths, vec!["src/lib.rs", "README.md"]);
}

fn repo_with_file(rel: &str) -> TempDir {
    let repo = TempDir::new().unwrap();
    fs::create_dir_all(repo.path().join(".git")).unwrap();
    let path = repo.path().join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, "content\n").unwrap();
    repo
}
