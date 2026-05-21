use super::super::*;

#[test]
fn normalize_learning_tags_trims_lowercases_and_dedupes() {
    let tags = normalize_learning_tags(vec![
        "  Perf ".to_string(),
        "BENCH".to_string(),
        "perf".to_string(),
        "   ".to_string(),
    ]);

    assert_eq!(tags, vec!["perf", "bench"]);
}

#[test]
fn normalize_learning_paths_trims_and_dedupes_preserving_case() {
    let paths = normalize_learning_paths(vec![
        "  crates/Foo/**  ".to_string(),
        "crates/Foo/**".to_string(),
        "crates/Bar/*.rs".to_string(),
        "   ".to_string(),
    ]);

    assert_eq!(paths, vec!["crates/Foo/**", "crates/Bar/*.rs"]);
}
