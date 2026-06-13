use std::collections::BTreeMap;
use std::str::FromStr;
use std::time::Instant;

use rusqlite::{Connection, params};

use crate::query::tests::support::{
    TestWorktree, graph_db_path, insert_file, insert_symbol, open_connection, open_graph,
};
use crate::sync::sync_leader_count;
use crate::{
    IMPACT_NODE_CAP, ImpactEntry, ImpactResult, RefConfidence, RefKind, Selector, SyncPolicy,
};

#[test]
fn impact_result_shape_matches_golden_fixture() {
    let result = ImpactResult {
        touched: vec![
            ImpactEntry {
                qualified_name: "crate::a".to_string(),
                distance: 1,
                edge_kind: RefKind::Call,
            },
            ImpactEntry {
                qualified_name: "crate::Impl".to_string(),
                distance: 2,
                edge_kind: RefKind::Impl,
            },
        ],
        truncated: false,
        visited_nodes: 2,
    };

    crate::query::tests::support::assert_json_matches_fixture(
        &result,
        include_str!("impact.golden.json"),
    );
}

#[test]
fn five_node_tree_at_depth_two_returns_all_five_touched_symbols() {
    let worktree = TestWorktree::new("impact-tree");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);

    seed_symbol(&conn, "src/root.rs", "root", "crate::root", 0, 100);
    seed_symbol(&conn, "src/a.rs", "a", "crate::a", 0, 50);
    seed_symbol(&conn, "src/b.rs", "b", "crate::b", 0, 50);
    seed_symbol(&conn, "src/c.rs", "c", "crate::c", 0, 50);
    seed_symbol(&conn, "src/d.rs", "d", "crate::d", 0, 50);
    seed_symbol(&conn, "src/e.rs", "e", "crate::e", 0, 50);
    seed_symbol(&conn, "src/fuzzy.rs", "fuzzy", "crate::fuzzy", 0, 50);

    insert_call_ref(&conn, "src/root.rs", 10, 11, "a", "crate::a", "exact");
    insert_call_ref(
        &conn,
        "src/root.rs",
        12,
        13,
        "fuzzy",
        "crate::fuzzy",
        "fuzzy_name",
    );
    insert_call_ref(&conn, "src/a.rs", 10, 11, "d", "crate::d", "exact");
    insert_call_ref(&conn, "src/b.rs", 10, 11, "root", "crate::root", "exact");
    insert_call_ref(&conn, "src/e.rs", 10, 11, "b", "crate::b", "exact");
    insert_relation(
        &conn,
        "src/c.rs",
        "crate::c",
        "crate::root",
        "impl",
        "exact",
    );

    let result = graph
        .impact(
            &symbol_selector("src/root.rs", "root"),
            2,
            RefConfidence::SameModule,
        )
        .expect("query impact tree");

    assert_eq!(result.visited_nodes, 5);
    assert_eq!(result.touched.len(), 5);
    assert!(!result.truncated);
    assert_by_distance(&result.touched);

    let by_name: BTreeMap<_, _> = result
        .touched
        .iter()
        .map(|entry| {
            (
                entry.qualified_name.as_str(),
                (entry.distance, entry.edge_kind),
            )
        })
        .collect();
    assert_eq!(by_name["crate::a"], (1, RefKind::Call));
    assert_eq!(by_name["crate::b"], (1, RefKind::Call));
    assert_eq!(by_name["crate::c"], (1, RefKind::Impl));
    assert_eq!(by_name["crate::d"], (2, RefKind::Call));
    assert_eq!(by_name["crate::e"], (2, RefKind::Call));
    assert!(!by_name.contains_key("crate::fuzzy"));
}

#[test]
fn circular_reference_does_not_loop_forever() {
    let worktree = TestWorktree::new("impact-cycle");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);

    seed_symbol(&conn, "src/root.rs", "root", "crate::root", 0, 100);
    seed_symbol(&conn, "src/a.rs", "a", "crate::a", 0, 100);
    insert_call_ref(&conn, "src/root.rs", 10, 11, "a", "crate::a", "exact");
    insert_call_ref(&conn, "src/a.rs", 10, 11, "root", "crate::root", "exact");

    let result = graph
        .impact(
            &symbol_selector("src/root.rs", "root"),
            10,
            RefConfidence::SameModule,
        )
        .expect("query impact cycle");

    assert_eq!(result.visited_nodes, 1);
    assert_eq!(result.touched[0].qualified_name, "crate::a");
    assert_eq!(result.touched[0].distance, 1);
    assert!(!result.truncated);
}

#[test]
fn synthetic_300_node_graph_caps_at_200_and_reports_truncation() {
    let worktree = TestWorktree::new("impact-cap");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    seed_wide_callee_graph(&conn, 300);

    let result = graph
        .impact(
            &symbol_selector("src/root.rs", "root"),
            10,
            RefConfidence::SameModule,
        )
        .expect("query impact cap");

    assert_eq!(result.visited_nodes, IMPACT_NODE_CAP);
    assert_eq!(result.touched.len(), IMPACT_NODE_CAP);
    assert!(result.truncated);
    assert!(result.touched.iter().all(|entry| entry.distance == 1));
}

#[test]
fn impact_calls_ensure_synced_at_entry() {
    let worktree = TestWorktree::new("impact-ensure-synced");
    worktree.write("src/lib.rs", "pub fn synced_root() {}\n");
    let graph = open_graph(&worktree, SyncPolicy::OnRead);
    let db_path = graph_db_path(&worktree);
    let selector =
        Selector::from_str("symbol:src/lib.rs#synced_root:function").expect("parse selector");

    let result = graph
        .impact(&selector, 1, RefConfidence::SameModule)
        .expect("impact triggers sync");

    assert_eq!(sync_leader_count(db_path.as_path()), 1);
    assert!(result.touched.is_empty());
}

#[test]
fn impact_200_node_cap_performance_smoke_prints_elapsed_ms() {
    let worktree = TestWorktree::new("impact-perf-cap");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    seed_wide_callee_graph(&conn, 300);

    let started = Instant::now();
    let result = graph
        .impact(
            &symbol_selector("src/root.rs", "root"),
            10,
            RefConfidence::SameModule,
        )
        .expect("query impact perf cap");
    let elapsed = started.elapsed();

    #[allow(clippy::print_stdout)]
    {
        println!("impact_200_node_cap_ms={}", elapsed.as_millis());
    }
    assert_eq!(result.visited_nodes, IMPACT_NODE_CAP);
    assert!(result.truncated);
}

#[test]
fn confidence_floor_filters_and_prevents_below_floor_expansion() {
    let worktree = TestWorktree::new("impact-confidence-floor");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);

    seed_symbol(&conn, "src/root.rs", "root", "crate::root", 0, 100);
    seed_symbol(&conn, "src/exact.rs", "exact", "crate::exact", 0, 50);
    seed_symbol(&conn, "src/same.rs", "same", "crate::same_module", 0, 50);
    seed_symbol(&conn, "src/leaf.rs", "leaf", "crate::leaf", 0, 50);
    seed_symbol(&conn, "src/fuzzy.rs", "fuzzy", "crate::fuzzy", 0, 50);

    insert_call_ref(
        &conn,
        "src/root.rs",
        10,
        11,
        "exact",
        "crate::exact",
        "exact",
    );
    insert_call_ref(
        &conn,
        "src/root.rs",
        12,
        13,
        "same",
        "crate::same_module",
        "same_module",
    );
    insert_call_ref(
        &conn,
        "src/root.rs",
        14,
        15,
        "fuzzy",
        "crate::fuzzy",
        "fuzzy_name",
    );
    insert_call_ref(&conn, "src/same.rs", 10, 11, "leaf", "crate::leaf", "exact");

    let default_floor = graph
        .impact(
            &symbol_selector("src/root.rs", "root"),
            2,
            RefConfidence::SameModule,
        )
        .expect("query impact default confidence floor");
    let default_names: Vec<_> = default_floor
        .touched
        .iter()
        .map(|entry| entry.qualified_name.as_str())
        .collect();
    assert_eq!(
        default_names,
        vec!["crate::exact", "crate::same_module", "crate::leaf"]
    );

    let exact_floor = graph
        .impact(
            &symbol_selector("src/root.rs", "root"),
            2,
            RefConfidence::Exact,
        )
        .expect("query impact exact confidence floor");
    let exact_names: Vec<_> = exact_floor
        .touched
        .iter()
        .map(|entry| entry.qualified_name.as_str())
        .collect();
    assert_eq!(exact_names, vec!["crate::exact"]);
}

#[test]
fn inbound_ref_outside_any_symbol_span_is_attributed_to_source_file() {
    // Regression for ORB-00381: a recorded inbound call site that does not fall
    // within any indexed symbol span must not be silently dropped. `refs` surfaces
    // such an edge (it only needs the call-site file/offset); `impact` previously
    // resolved the source symbol via span containment and dropped the edge when that
    // subquery returned NULL. The edge is now attributed to the source file node.
    let worktree = TestWorktree::new("impact-null-source-span");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);

    seed_symbol(&conn, "src/target.rs", "target", "crate::target", 0, 50);
    // The caller file has a symbol, but the call site at offset 5..6 sits *before*
    // that symbol's span (100..200), so no enclosing symbol exists for the edge.
    seed_symbol(&conn, "src/caller.rs", "caller", "crate::caller", 100, 200);
    insert_call_ref(
        &conn,
        "src/caller.rs",
        5,
        6,
        "target",
        "crate::target",
        "exact",
    );

    let result = graph
        .impact(
            &symbol_selector("src/target.rs", "target"),
            2,
            RefConfidence::Exact,
        )
        .expect("query impact with unanchored inbound ref");

    assert_eq!(result.visited_nodes, 1, "edge must not be dropped");
    assert_eq!(result.touched.len(), 1);
    let entry = &result.touched[0];
    assert_eq!(entry.qualified_name, "src/caller.rs");
    assert_eq!(entry.distance, 1);
    assert_eq!(entry.edge_kind, RefKind::Call);
    assert!(!result.truncated);
}

#[test]
fn fuzzy_name_inbound_ref_with_null_qualified_is_visible_at_fuzzy_floor() {
    // Regression for ORB-00381: `fuzzy_name` edges store a NULL `target_qualified`
    // and are matchable only by `target_name`. Keying impact's inbound query solely
    // on `target_qualified` made every fuzzy edge invisible — so a symbol that `refs
    // --confidence fuzzy` reports as referenced returned an empty blast radius.
    let worktree = TestWorktree::new("impact-fuzzy-null-qualified");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);

    seed_symbol(
        &conn,
        "src/target.rs",
        "did_you_mean",
        "crate::did_you_mean",
        0,
        50,
    );
    seed_symbol(&conn, "src/caller.rs", "caller", "crate::caller", 0, 200);
    insert_fuzzy_call_ref(&conn, "src/caller.rs", 10, 11, "did_you_mean");

    // Below the fuzzy floor the edge stays excluded, matching `refs` semantics.
    let same_module = graph
        .impact(
            &symbol_selector("src/target.rs", "did_you_mean"),
            2,
            RefConfidence::SameModule,
        )
        .expect("query impact below fuzzy floor");
    assert!(same_module.touched.is_empty());

    // At the fuzzy floor the caller is surfaced.
    let fuzzy = graph
        .impact(
            &symbol_selector("src/target.rs", "did_you_mean"),
            2,
            RefConfidence::FuzzyName,
        )
        .expect("query impact at fuzzy floor");
    assert_eq!(fuzzy.visited_nodes, 1);
    let names: Vec<_> = fuzzy
        .touched
        .iter()
        .map(|entry| entry.qualified_name.as_str())
        .collect();
    assert_eq!(names, vec!["crate::caller"]);
    assert_eq!(fuzzy.touched[0].edge_kind, RefKind::Call);
}

fn seed_symbol(
    conn: &Connection,
    file_path: &str,
    name: &str,
    qualified: &str,
    span_start: usize,
    span_end: usize,
) {
    let content = " ".repeat(span_end.max(1));
    insert_file(conn, file_path, "rust", content.as_str());
    insert_symbol(
        conn, file_path, name, qualified, "function", span_start, span_end,
    );
}

fn seed_wide_callee_graph(conn: &Connection, count: usize) {
    let root_len = count * 10 + 20;
    let root_content = " ".repeat(root_len);
    insert_file(conn, "src/root.rs", "rust", root_content.as_str());
    insert_symbol(
        conn,
        "src/root.rs",
        "root",
        "crate::root",
        "function",
        0,
        root_len,
    );

    let target_content = " ".repeat(root_len);
    insert_file(conn, "src/targets.rs", "rust", target_content.as_str());
    for index in 0..count {
        let name = format!("target_{index:03}");
        let qualified = format!("crate::{name}");
        let span_start = index * 10;
        insert_symbol(
            conn,
            "src/targets.rs",
            name.as_str(),
            qualified.as_str(),
            "function",
            span_start,
            span_start + 5,
        );
        insert_call_ref(
            conn,
            "src/root.rs",
            span_start + 1,
            span_start + 2,
            name.as_str(),
            qualified.as_str(),
            "exact",
        );
    }
}

fn insert_call_ref(
    conn: &Connection,
    from_file: &str,
    span_start: usize,
    span_end: usize,
    target_name: &str,
    target_qualified: &str,
    confidence: &str,
) {
    conn.execute(
        "INSERT INTO refs (
            from_file, from_span_start, from_span_end, target_name, target_qualified,
            target_symbol_hint, kind, confidence
         ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, 'call', ?6)",
        params![
            from_file,
            i64::try_from(span_start).expect("span start fits"),
            i64::try_from(span_end).expect("span end fits"),
            target_name,
            target_qualified,
            confidence
        ],
    )
    .expect("insert call ref");
}

/// Insert a `fuzzy_name` call ref as the extractor records it: NULL `target_qualified`,
/// matchable only by `target_name`.
fn insert_fuzzy_call_ref(
    conn: &Connection,
    from_file: &str,
    span_start: usize,
    span_end: usize,
    target_name: &str,
) {
    conn.execute(
        "INSERT INTO refs (
            from_file, from_span_start, from_span_end, target_name, target_qualified,
            target_symbol_hint, kind, confidence
         ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, 'call', 'fuzzy_name')",
        params![
            from_file,
            i64::try_from(span_start).expect("span start fits"),
            i64::try_from(span_end).expect("span end fits"),
            target_name,
        ],
    )
    .expect("insert fuzzy call ref");
}

fn insert_relation(
    conn: &Connection,
    def_file: &str,
    from_qualified: &str,
    to_qualified: &str,
    kind: &str,
    confidence: &str,
) {
    conn.execute(
        "INSERT INTO relations (
            from_qualified, to_qualified, kind, def_file, def_span_start, def_span_end, confidence
         ) VALUES (?1, ?2, ?3, ?4, 0, 1, ?5)",
        params![from_qualified, to_qualified, kind, def_file, confidence],
    )
    .expect("insert relation");
}

fn symbol_selector(file_path: &str, symbol: &str) -> Selector {
    Selector::Symbol {
        path: file_path.to_string(),
        symbol: symbol.to_string(),
        kind: "function".to_string(),
    }
}

fn assert_by_distance(entries: &[ImpactEntry]) {
    assert!(
        entries
            .windows(2)
            .all(|window| window[0].distance <= window[1].distance)
    );
}
