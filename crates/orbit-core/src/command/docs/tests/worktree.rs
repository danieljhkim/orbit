//! Worktree-local docs source-root regression coverage.

use std::fs;

use tempfile::tempdir;

use super::super::search::doc_embedding_sources;
use crate::OrbitRuntime;

#[test]
fn docs_use_the_active_worktree_when_metadata_is_shared() {
    let root = tempdir().expect("tempdir");
    let global_root = root.path().join("global");
    let primary = root.path().join("primary");
    let worktree = root.path().join("worktree");
    let shared_root = primary.join(".orbit");
    let local_root = worktree.join(".orbit");

    fs::create_dir_all(&global_root).expect("global root");
    fs::create_dir_all(&shared_root).expect("shared root");
    fs::create_dir_all(&local_root).expect("local root");
    fs::create_dir_all(primary.join("docs")).expect("primary docs");
    fs::create_dir_all(worktree.join("docs")).expect("worktree docs");
    fs::write(
        shared_root.join("config.toml"),
        "[docs]\nroots = [\"docs/\"]\n",
    )
    .expect("docs config");
    fs::write(
        primary.join("docs/guide.md"),
        "---\ntype: context\nsummary: Guide\n---\nPrimary body\n",
    )
    .expect("primary doc");
    fs::write(
        worktree.join("docs/guide.md"),
        "---\ntype: context\nsummary: Guide\n---\nWorktree body\n",
    )
    .expect("worktree doc");

    let runtime = OrbitRuntime::from_resolved_roots(&global_root, &shared_root, &local_root)
        .expect("build runtime");

    let shown = runtime
        .show_doc("docs/guide.md")
        .expect("show worktree doc");
    assert_eq!(shown.body, "Worktree body\n");
    assert_eq!(runtime.docs_source_root(), worktree);

    let sources = doc_embedding_sources(
        &runtime.docs_source_root(),
        &runtime.docs_roots().expect("docs roots"),
    )
    .expect("read worktree embedding sources");
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].body, "Worktree body\n");
}
