//! Tests for the private `dir_depth` helper (visible via module subtree).
//!
//! These ensure depth-descending sort in write_graph works correctly for root and nested locations.

use super::super::*;

#[test]
fn dir_depth_root_location_is_zero() {
    // Regression for T20260421-0652: root location produced by
    // `build_graph_dirs` is `"./"`, which must normalize to depth 0 so the
    // depth-descending sort in `write_graph` writes root after its
    // children.
    assert_eq!(dir_depth("./"), 0);
    assert_eq!(dir_depth("."), 0);
    assert_eq!(dir_depth(""), 0);
    assert_eq!(dir_depth("/"), 0);
}

#[test]
fn dir_depth_counts_segments_not_slashes() {
    assert_eq!(dir_depth("src/"), 1);
    assert_eq!(dir_depth("src"), 1);
    assert_eq!(dir_depth("src/foo/"), 2);
    assert_eq!(dir_depth("src/foo/bar/"), 3);
}

#[test]
fn dir_depth_ignores_current_dir_segments() {
    // Paths like "./src/" should count "src" only — the leading "." is a
    // relative-path marker, not a depth segment.
    assert_eq!(dir_depth("./src/"), 1);
    assert_eq!(dir_depth("./src/foo/"), 2);
}

#[test]
fn dir_depth_is_strict_weak_order_root_first_by_descending_depth() {
    // Depth-descending sort must place nested dirs before root.
    let mut locations = vec!["./", "src/", "src/foo/"];
    locations.sort_by_key(|location| std::cmp::Reverse(dir_depth(location)));
    assert_eq!(locations, vec!["src/foo/", "src/", "./"]);
}
