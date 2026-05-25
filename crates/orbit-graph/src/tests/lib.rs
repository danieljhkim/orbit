use std::path::Path;

use crate::resolve_db_path;

#[test]
fn db_path_sanitizes_branch_slashes_and_preserves_raw_branch() {
    let worktree_root = Path::new("/tmp/orbit-worktree");

    let feat = resolve_db_path(worktree_root, "feat/foo", 1);
    assert_eq!(
        feat.path(),
        Path::new("/tmp/orbit-worktree/.orbit/graph/feat_foo.1.db")
    );
    assert_eq!(
        feat.path().file_name().and_then(|name| name.to_str()),
        Some("feat_foo.1.db")
    );
    assert_eq!(feat.branch(), "feat/foo");
    assert_eq!(feat.extractor_version(), 1);

    let main = resolve_db_path(worktree_root, "main", 42);
    assert_eq!(
        main.path(),
        Path::new("/tmp/orbit-worktree/.orbit/graph/main.42.db")
    );
    assert_eq!(
        main.path().file_name().and_then(|name| name.to_str()),
        Some("main.42.db")
    );
    assert_eq!(main.branch(), "main");
    assert_eq!(main.extractor_version(), 42);
}
