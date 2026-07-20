#![allow(missing_docs)]

use super::super::dispatch::shared_prefix_depth;
use super::super::support::tasks_conflict;

#[test]
fn tasks_conflict_uses_selector_anchor_overlap() {
    assert!(tasks_conflict(
        &["symbol:f.rs#a:method".to_string()],
        &["symbol:f.rs#b:method".to_string()]
    ));
    assert!(tasks_conflict(
        &["dir:src".to_string()],
        &["file:src/lib.rs".to_string()]
    ));
    assert!(!tasks_conflict(
        &["file:f.rs".to_string()],
        &["file:g.rs".to_string()]
    ));
}

#[test]
fn shared_prefix_depth_uses_selector_anchors() {
    assert_eq!(
        shared_prefix_depth("symbol:src/lib.rs#run:function", "dir:src"),
        1
    );
    assert_eq!(
        shared_prefix_depth("file:src/a.rs", "file:src/nested/b.rs"),
        1
    );
    assert_eq!(shared_prefix_depth("file:src/a.rs", "file:tests/a.rs"), 0);
}
