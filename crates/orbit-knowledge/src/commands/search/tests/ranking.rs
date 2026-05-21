#![allow(missing_docs)]

use super::super::*;
use super::*;

#[test]
fn default_search_ranking_matches_snapshot() {
    let graph = graph_for_default_ranking_snapshot();
    let result = default_search(DefaultSearchInput {
        graph: &graph,
        query: "fixture",
        limit: 10,
        include_non_code: true,
    })
    .expect("default search");

    let snapshot: Vec<_> = result
        .hits
        .iter()
        .map(|hit| {
            (
                hit.selector.as_str(),
                hit.kind.as_str(),
                hit.file.as_deref().unwrap_or(""),
            )
        })
        .collect();
    assert_eq!(
        snapshot,
        vec![
            (
                "symbol:src/fixture.rs#fixture_fn:function",
                "function",
                "src/fixture.rs"
            ),
            ("file:src/fixture.rs", "file", "src/fixture.rs"),
            (
                "symbol:README.md#fixture_doc:section",
                "section",
                "README.md"
            ),
        ]
    );
}

#[test]
fn exact_trait_definition_outranks_impl_methods_for_same_trait_name() {
    let leaves = vec![
        leaf_node_with_kind(
            "symbol:src/runtime.rs#<OrbitRuntime as V2RuntimeHost>::start:method",
            "start",
            "src/runtime.rs#<OrbitRuntime as V2RuntimeHost>::start",
            Some("file:src/runtime.rs"),
            1,
            LeafKind::Method,
        ),
        leaf_node_with_kind(
            "symbol:src/runtime.rs#<OrbitRuntime as V2RuntimeHost>::stop:method",
            "stop",
            "src/runtime.rs#<OrbitRuntime as V2RuntimeHost>::stop",
            Some("file:src/runtime.rs"),
            2,
            LeafKind::Method,
        ),
        leaf_node_with_kind(
            "symbol:src/dispatcher.rs#V2RuntimeHost:trait",
            "V2RuntimeHost",
            "src/dispatcher.rs#V2RuntimeHost",
            Some("file:src/dispatcher.rs"),
            3,
            LeafKind::Trait,
        ),
    ];

    let ranked =
        rank_exact_symbol_definition_hits(search_hits_for_leaves(&leaves), "V2RuntimeHost");

    assert_eq!(
        hit_ids(&ranked),
        vec![
            "symbol:src/dispatcher.rs#V2RuntimeHost:trait",
            "symbol:src/runtime.rs#<OrbitRuntime as V2RuntimeHost>::start:method",
            "symbol:src/runtime.rs#<OrbitRuntime as V2RuntimeHost>::stop:method",
        ]
    );
}

#[test]
fn exact_struct_definition_outranks_methods_on_that_struct() {
    let leaves = vec![
        leaf_node_with_kind(
            "symbol:src/widget.rs#Widget::new:method",
            "new",
            "src/widget.rs#Widget::new",
            Some("file:src/widget.rs"),
            1,
            LeafKind::Method,
        ),
        leaf_node_with_kind(
            "symbol:src/widget.rs#Widget::render:method",
            "render",
            "src/widget.rs#Widget::render",
            Some("file:src/widget.rs"),
            2,
            LeafKind::Method,
        ),
        leaf_node_with_kind(
            "symbol:src/widget.rs#Widget:struct",
            "Widget",
            "src/widget.rs#Widget",
            Some("file:src/widget.rs"),
            3,
            LeafKind::Struct,
        ),
    ];

    let ranked = rank_exact_symbol_definition_hits(search_hits_for_leaves(&leaves), "Widget");

    assert_eq!(
        hit_ids(&ranked),
        vec![
            "symbol:src/widget.rs#Widget:struct",
            "symbol:src/widget.rs#Widget::new:method",
            "symbol:src/widget.rs#Widget::render:method",
        ]
    );
}

#[test]
fn substring_only_symbol_matches_retain_scan_order() {
    let leaves = vec![
        leaf_node_with_kind(
            "symbol:src/widget.rs#Widget::new:method",
            "new",
            "src/widget.rs#Widget::new",
            Some("file:src/widget.rs"),
            1,
            LeafKind::Method,
        ),
        leaf_node_with_kind(
            "symbol:src/adapter.rs#<Adapter as Widget>::run:method",
            "run",
            "src/adapter.rs#<Adapter as Widget>::run",
            Some("file:src/adapter.rs"),
            2,
            LeafKind::Method,
        ),
        leaf_node_with_kind(
            "symbol:src/adapter.rs#impl Widget for Adapter:impl",
            "impl Widget for Adapter",
            "src/adapter.rs#impl Widget for Adapter",
            Some("file:src/adapter.rs"),
            3,
            LeafKind::Impl,
        ),
    ];
    let hits = search_hits_for_leaves(&leaves);
    let original_ids = hit_ids(&hits);

    let ranked = rank_exact_symbol_definition_hits(hits, "Widget");

    assert_eq!(hit_ids(&ranked), original_ids);
}

#[test]
fn default_ranking_search_uses_finite_capped_bound_for_large_graph() {
    const REQUESTED_LIMIT: usize = 10;
    const FIXTURE_LEAF_COUNT: usize = 1_000;

    let graph = graph_with_matching_leaves(FIXTURE_LEAF_COUNT);
    let service = GraphContextService::new(&graph);
    let search_limit = default_ranking_search_limit(REQUESTED_LIMIT);

    assert!(search_limit <= DEFAULT_RANKING_HARD_CAP);
    assert_eq!(search_limit, REQUESTED_LIMIT * DEFAULT_RANKING_HEADROOM);

    let (_total, hits) =
        service.search_hits_with_total("fixture", None, None, None, None, search_limit);

    assert_eq!(hits.len(), search_limit);
}

#[test]
fn default_ranking_search_limit_saturates_at_hard_cap() {
    let over_cap_limit = DEFAULT_RANKING_HARD_CAP / DEFAULT_RANKING_HEADROOM + 1;

    assert_eq!(
        default_ranking_search_limit(over_cap_limit),
        DEFAULT_RANKING_HARD_CAP
    );
}
