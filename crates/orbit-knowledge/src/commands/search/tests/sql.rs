#![allow(missing_docs)]

use super::*;

#[test]
fn sql_substring_path_matches_fallback_for_diverse_query_shapes() {
    let graph = graph_for_sql_search_tests();
    let (_temp_dir, reader) = index_reader_for_graph(&graph);

    for query in [
        "UniqueSymbol",
        "nique",
        "Unique Symbol",
        "^UniqueSymbol$",
        "src/core/",
        "src/special%_dir/",
    ] {
        let rows = sql_substring_selectors(&reader, query, false, 20);
        assert_eq!(
            rows,
            fallback_selectors(&graph, query, false, 20),
            "sql/fallback divergence for `{query}`"
        );
    }
}

#[test]
fn sql_substring_path_honors_user_limit() {
    let graph = graph_with_repeated_name_leaves(25, "CommonSymbol");
    let (_temp_dir, reader) = index_reader_for_graph(&graph);

    let rows = sql_substring_selectors(&reader, "CommonSymbol", false, 10);
    assert_eq!(rows.len(), 10);
    assert_eq!(rows, fallback_selectors(&graph, "CommonSymbol", false, 10));
}

#[test]
fn missing_or_stale_index_yields_no_sql_outcome() {
    let graph = graph_for_sql_search_tests();
    let (temp_dir, _reader) = index_reader_for_graph(&graph);
    let store = GraphObjectStore::new(temp_dir.path().join("graph"));

    assert!(
        GraphIndexReader::open_current(store.graph_sqlite_index_path(), "stale-root")
            .expect("open stale index")
            .is_none()
    );
    std::fs::remove_file(store.graph_sqlite_index_path()).expect("delete sqlite index");
    assert!(
        GraphIndexReader::open_current(store.graph_sqlite_index_path(), "missing-root")
            .expect("open missing index")
            .is_none()
    );
}
