use super::super::shared_prefix_depth;

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
