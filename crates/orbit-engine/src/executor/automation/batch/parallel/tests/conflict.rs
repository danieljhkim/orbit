use super::super::tasks_conflict;

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
