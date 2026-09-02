//! Unit tests for `linux_sandbox` rule expansion — sibling layout under
//! src/tests/. The filesystem walk is platform-neutral, so these run
//! everywhere; the bwrap spawn itself is covered by the Linux-only
//! integration tests.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use super::{expand_rule, expand_rules, walk_paths};

fn tree() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    for dir in ["a", "a/deep", "b", "target/debug"] {
        fs::create_dir_all(root.join(dir)).expect("create dir");
    }
    for file in [
        ".env",
        "a/.env",
        "a/deep/.env.local",
        "b/settings.env",
        "b/env.txt",
        "target/debug/build.env.bak",
    ] {
        fs::write(root.join(file), b"x").expect("write file");
    }
    temp
}

fn canonical(root: &std::path::Path, rel: &str) -> PathBuf {
    root.join(rel).canonicalize().expect("canonical")
}

/// Every rule sharing a search root is matched from one walk, and the union
/// equals what the rules would have matched one at a time.
#[test]
fn rules_sharing_a_root_expand_from_one_walk_to_the_same_set() {
    let temp = tree();
    let root = temp.path().canonicalize().expect("canonical root");
    let prefix = root.to_string_lossy().replace('\\', "/");
    let rules: Vec<String> = ["**/.env", "**/.env.*", "**/*.env", "**/*.env.*"]
        .iter()
        .map(|glob| format!("{prefix}/{glob}"))
        .collect();

    let together = expand_rules(&rules).expect("expand together");
    let mut one_at_a_time = BTreeSet::new();
    for rule in &rules {
        one_at_a_time.extend(expand_rule(rule).expect("expand one"));
    }
    assert_eq!(together, one_at_a_time);

    let expected: BTreeSet<PathBuf> = [
        ".env",
        "a/.env",
        "a/deep/.env.local",
        "b/settings.env",
        "target/debug/build.env.bak",
    ]
    .iter()
    .map(|rel| canonical(&root, rel))
    .collect();
    assert_eq!(together, expected);
}

/// The walk lists each path exactly once: directories used to be pushed on
/// entry and again from their parent's listing.
#[test]
fn walk_lists_every_path_once() {
    let temp = tree();
    let root = temp.path().canonicalize().expect("canonical root");
    let mut paths = Vec::new();
    walk_paths(&root, &mut paths).expect("walk");
    let unique: BTreeSet<&PathBuf> = paths.iter().collect();
    assert_eq!(unique.len(), paths.len(), "duplicates in {paths:?}");
    // 1 root + 4 dirs (a, a/deep, b, target, target/debug = 5) + 6 files.
    assert_eq!(paths.len(), 1 + 5 + 6);
    assert_eq!(paths[0], root);
}
