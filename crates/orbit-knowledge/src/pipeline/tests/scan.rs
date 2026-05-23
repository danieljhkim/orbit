use std::fs;
use std::path::Path;

use tempfile::tempdir;

use super::super::scan::OrbitIgnoreMatcher;

#[test]
fn orbitignore_matches_literal_filename() {
    let repo = tempdir().unwrap();
    fs::write(repo.path().join(".orbitignore"), "foo.rs\n").unwrap();

    let matcher = OrbitIgnoreMatcher::load(repo.path()).unwrap();
    assert!(matcher.is_ignored(Path::new("foo.rs"), false));
    assert!(!matcher.is_ignored(Path::new("bar.rs"), false));
}

#[test]
fn orbitignore_matches_recursive_glob() {
    let repo = tempdir().unwrap();
    fs::write(repo.path().join(".orbitignore"), "**/generated.rs\n").unwrap();

    let matcher = OrbitIgnoreMatcher::load(repo.path()).unwrap();
    assert!(matcher.is_ignored(Path::new("src/generated.rs"), false));
    assert!(matcher.is_ignored(Path::new("deep/nested/generated.rs"), false));
}

#[test]
fn orbitignore_negation_reincludes_prior_exclusion() {
    let repo = tempdir().unwrap();
    fs::write(
        repo.path().join(".orbitignore"),
        "generated/**\n!generated/keep.rs\n",
    )
    .unwrap();

    let matcher = OrbitIgnoreMatcher::load(repo.path()).unwrap();
    assert!(matcher.is_ignored(Path::new("generated/drop.rs"), false));
    assert!(!matcher.is_ignored(Path::new("generated/keep.rs"), false));
}

#[test]
fn orbitignore_directory_only_patterns_match_dirs_but_not_files() {
    let repo = tempdir().unwrap();
    fs::write(repo.path().join(".orbitignore"), "foo/\n").unwrap();

    let matcher = OrbitIgnoreMatcher::load(repo.path()).unwrap();
    assert!(matcher.is_ignored(Path::new("foo"), true));
    assert!(!matcher.is_ignored(Path::new("foo"), false));
}

#[test]
fn orbitignore_ignores_comment_lines() {
    let repo = tempdir().unwrap();
    fs::write(repo.path().join(".orbitignore"), "# comment\nbar.rs\n").unwrap();

    let matcher = OrbitIgnoreMatcher::load(repo.path()).unwrap();
    assert!(matcher.is_ignored(Path::new("bar.rs"), false));
    assert!(!matcher.is_ignored(Path::new("comment"), false));
}
