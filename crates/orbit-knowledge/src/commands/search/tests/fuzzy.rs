#![allow(missing_docs)]

use super::super::*;
use super::*;

#[test]
fn fuzzy_pass_returns_orbit_error_for_orbit_erorr_query() {
    let graph = graph_with_named_leaves(&[("OrbitError", LeafKind::Struct)]);
    let (_temp_dir, context) = context_for_graph(&graph);

    let result = run(search_input(context, "OrbitErorr", true, 5)).expect("fuzzy search result");

    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].name, "OrbitError");
    assert_eq!(result.hits[0].match_kind.as_deref(), Some("fuzzy"));
    let score = result.hits[0].score.expect("fuzzy score");
    assert!(score > 0.0);
    assert!(score <= 1.0);
}

#[test]
fn exact_match_suppresses_fuzzy_candidates() {
    let graph = graph_with_named_leaves(&[
        ("OrbitError", LeafKind::Struct),
        ("OrbitErrorKind", LeafKind::Enum),
    ]);
    let (_temp_dir, context) = context_for_graph(&graph);

    let result = run(search_input(context, "OrbitError", true, 5)).expect("exact search result");

    assert!(!result.hits.is_empty());
    assert!(result.hits.iter().all(|hit| hit.match_kind.is_none()));
}

#[test]
fn allow_fuzzy_false_preserves_zero_result_behavior() {
    let graph = graph_with_named_leaves(&[("OrbitError", LeafKind::Struct)]);
    let (_temp_dir, context) = context_for_graph(&graph);

    let result =
        run(search_input(context, "OrbitErorr", false, 5)).expect("non-fuzzy search result");

    assert!(result.hits.is_empty());
}

#[test]
fn fuzzy_pass_returns_empty_for_no_plausible_match() {
    let graph = graph_with_named_leaves(&[
        ("Widget", LeafKind::Struct),
        ("Adapter", LeafKind::Struct),
        ("Runtime", LeafKind::Struct),
    ]);
    let (_temp_dir, context) = context_for_graph(&graph);

    let result = run(search_input(context, "zzzzzzzzzz", true, 5)).expect("fuzzy search result");

    assert!(result.hits.is_empty());
}

#[test]
fn fuzzy_results_break_score_ties_alphabetically() {
    let graph = graph_with_named_leaves(&[("Baz", LeafKind::Struct), ("Bar", LeafKind::Struct)]);
    let (_temp_dir, context) = context_for_graph(&graph);

    let result = run(search_input(context, "Bax", true, 5)).expect("fuzzy search result");

    assert_eq!(result.hits.len(), 2);
    assert_eq!(result.hits[0].score, result.hits[1].score);
    assert!(result.hits[0].selector < result.hits[1].selector);
    assert_eq!(result.hits[0].name, "Bar");
    assert_eq!(result.hits[1].name, "Baz");
}
