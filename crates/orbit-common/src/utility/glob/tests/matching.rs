use super::super::*;
use crate::types::OrbitError;

#[test]
fn double_star_matches_nested_paths() {
    let path = normalize_glob_path("crates/orbit-engine/perf_runner.rs").expect("normalize");
    assert!(match_glob("**/perf*.rs", &path).expect("match glob"));
}

#[test]
fn double_star_rejects_non_matching_filename() {
    let path = normalize_glob_path("crates/orbit-engine/runner.rs").expect("normalize");
    assert!(!match_glob("**/perf*.rs", &path).expect("match glob"));
}

#[test]
fn normalize_strips_leading_dot_slash_and_backslashes() {
    let path = normalize_glob_path("./crates\\orbit-engine/perf.rs").expect("normalize");
    assert_eq!(path, "crates/orbit-engine/perf.rs");
}

#[test]
fn normalize_rejects_traversal() {
    assert!(matches!(
        normalize_glob_path("../escape"),
        Err(OrbitError::InvalidInput(_))
    ));
}

#[test]
fn trailing_double_star_matches_subtree_and_anchor() {
    let path = normalize_glob_path("foo/bar/baz.rs").expect("normalize");
    assert!(match_glob("foo/**", &path).expect("match"));

    let exact = normalize_glob_path("foo").expect("normalize");
    assert!(match_glob("foo/**", &exact).expect("match"));
}

#[test]
fn single_star_does_not_cross_separator() {
    let path = normalize_glob_path("foo/bar/baz.rs").expect("normalize");
    assert!(!match_glob("foo/*.rs", &path).expect("match"));
}
