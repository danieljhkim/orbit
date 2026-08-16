//! Walk / git-ignore batching tests migrated for ORB-00250.

use std::fs;

use tempfile::tempdir;

use super::super::walk::{
    git_check_ignore_invocations, reset_git_check_ignore_invocations, walk_docs_roots,
};

#[test]
fn walker_skips_dot_orbit_even_when_root_points_above_it() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("docs")).expect("docs dir");
    fs::write(
        root.join("docs/good.md"),
        "---\ntype: context\nsummary: Good doc\n---\nbody\n",
    )
    .expect("write good");
    fs::create_dir_all(root.join(".orbit/adrs/ADR-0001")).expect("adr dir");
    fs::write(root.join(".orbit/adrs/ADR-0001/body.md"), "# ADR\n").expect("write adr");

    let records = walk_docs_roots(root, &[".".to_string()]).expect("walk docs");
    assert_eq!(
        records
            .iter()
            .map(|record| record.path.as_str())
            .collect::<Vec<_>>(),
        vec!["docs/good.md"]
    );
}

#[test]
fn walker_batches_git_ignore_once_per_walk() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("docs/nested")).expect("docs dir");
    fs::write(
        root.join("docs/one.md"),
        "---\ntype: context\nsummary: One doc\n---\nbody\n",
    )
    .expect("write one");
    fs::write(
        root.join("docs/nested/two.md"),
        "---\ntype: context\nsummary: Two doc\n---\nbody\n",
    )
    .expect("write two");

    reset_git_check_ignore_invocations();
    let records = walk_docs_roots(root, &["docs/".to_string()]).expect("walk docs");

    assert_eq!(git_check_ignore_invocations(), 1);
    assert_eq!(records.len(), 2);
}
