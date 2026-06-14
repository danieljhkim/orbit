use crate::query::tests::support::{
    TestWorktree, insert_file, insert_symbol, open_connection, open_graph,
};
use crate::{GraphError, OverviewFormat, Selector, SyncPolicy};

/// Seed three files (two Rust under `crates/a/src`, one root markdown) with a
/// mix of symbol kinds. Returns the worktree (kept alive by the caller so its
/// temp dir and DB outlive the query) and the opened graph.
fn seeded_graph(name: &str) -> (TestWorktree, crate::Graph) {
    let worktree = TestWorktree::new(name);
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    insert_file(&conn, "crates/a/src/lib.rs", "rust", "");
    insert_file(&conn, "crates/a/src/mod.rs", "rust", "");
    insert_file(&conn, "README.md", "markdown", "");
    insert_symbol(
        &conn,
        "crates/a/src/lib.rs",
        "alpha",
        "a::alpha",
        "function",
        0,
        1,
    );
    insert_symbol(
        &conn,
        "crates/a/src/lib.rs",
        "beta",
        "a::beta",
        "function",
        2,
        3,
    );
    insert_symbol(
        &conn,
        "crates/a/src/mod.rs",
        "Widget",
        "a::Widget",
        "struct",
        0,
        1,
    );
    insert_symbol(&conn, "README.md", "Intro", "Intro", "heading", 0, 1);
    (worktree, graph)
}

#[test]
fn overview_summary_aggregates_counts_and_top_files() {
    let (_worktree, graph) = seeded_graph("overview-summary");

    let result = graph
        .overview(None, OverviewFormat::Summary)
        .expect("overview query");

    assert_eq!(result.format, OverviewFormat::Summary);
    assert_eq!(result.scope, None);
    assert_eq!(result.total_files, 3);
    assert_eq!(result.total_symbols, 4);
    assert_eq!(result.languages.get("rust"), Some(&2));
    assert_eq!(result.languages.get("markdown"), Some(&1));
    assert_eq!(result.symbol_kinds.get("function"), Some(&2));
    assert_eq!(result.symbol_kinds.get("struct"), Some(&1));
    assert_eq!(result.symbol_kinds.get("heading"), Some(&1));
    // Summary omits per-file symbol lists.
    assert!(result.files.iter().all(|file| file.symbols.is_empty()));
    // lib.rs has the most symbols, so it ranks first.
    assert_eq!(
        result.files.first().map(|file| file.path.as_str()),
        Some("crates/a/src/lib.rs")
    );
    assert_eq!(result.files.first().map(|file| file.symbol_count), Some(2));
}

#[test]
fn overview_full_scoped_to_dir_lists_symbols() {
    let (_worktree, graph) = seeded_graph("overview-full");

    let result = graph
        .overview(
            Some(&Selector::Dir {
                path: "crates/a/src".to_string(),
            }),
            OverviewFormat::Full,
        )
        .expect("overview query");

    assert_eq!(result.format, OverviewFormat::Full);
    assert_eq!(result.scope, Some("crates/a/src".to_string()));
    assert_eq!(result.total_files, 2);
    assert_eq!(result.total_symbols, 3);
    // README.md is outside the scope.
    assert!(result.files.iter().all(|file| file.path != "README.md"));

    let lib = result
        .files
        .iter()
        .find(|file| file.path == "crates/a/src/lib.rs")
        .expect("lib.rs in full overview");
    assert_eq!(lib.symbol_count, 2);
    assert_eq!(lib.symbols.len(), 2);
    assert_eq!(lib.symbols[0].name, "alpha");
    assert_eq!(lib.symbols[0].kind, "function");
}

#[test]
fn overview_rejects_symbol_selector_scope() {
    let (_worktree, graph) = seeded_graph("overview-reject");

    let error = graph
        .overview(
            Some(&Selector::Symbol {
                path: "crates/a/src/lib.rs".to_string(),
                symbol: "alpha".to_string(),
                kind: "function".to_string(),
            }),
            OverviewFormat::Summary,
        )
        .expect_err("symbol selector should be rejected");
    assert!(matches!(error, GraphError::InvalidData { .. }));
}
