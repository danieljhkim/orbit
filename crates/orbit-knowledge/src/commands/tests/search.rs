#![allow(missing_docs)]

// Merged tests for commands/search.rs (was split into mod/fuzzy/ranking/sql under
// nested search/tests/ anti-pattern). Now single sibling file per source, per
// docs/design-patterns/test_layout.md. Explicit imports from super::super::search.

use crate::commands::{GraphCommandContext, TaskGraphScope};
use crate::graph::GraphIndexReader;
use crate::graph::navigator::GraphNodeRef;
use crate::graph::nodes::{BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode};
use crate::graph::object_store::{GraphObjectStore, RefName};
use crate::service::GraphContextService;

use crate::service::SearchHit;

use super::super::search::{
    DEFAULT_RANKING_HARD_CAP, DEFAULT_RANKING_HEADROOM, DefaultSearchInput, SearchInput,
    default_ranking_search_limit, default_search, rank_exact_symbol_definition_hits,
    rank_sql_default_search_results, run, selector_for_search_row,
};

// --------------------------------------------------------------------------
// Shared test helpers (private to tests module + descendants)
// --------------------------------------------------------------------------

fn search_input(
    context: GraphCommandContext,
    query: &str,
    allow_fuzzy: bool,
    limit: usize,
) -> SearchInput {
    SearchInput {
        context,
        query: query.to_string(),
        node_type: None,
        kind_filter: None,
        prefix: None,
        source_regex: None,
        include_non_code: false,
        allow_fuzzy,
        limit,
    }
}

fn context_for_graph(graph: &CodebaseGraphV1) -> (tempfile::TempDir, GraphCommandContext) {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let store = GraphObjectStore::new(temp_dir.path().join("graph"));
    let current_ref = store.write_graph(graph).expect("write graph");
    let ref_name = RefName::new("search-test").expect("valid ref name");
    store
        .write_ref_atomic(&ref_name, &current_ref)
        .expect("write graph ref");
    let context = GraphCommandContext {
        knowledge_dir: temp_dir.path().to_path_buf(),
        workspace_root: None,
        explicit_ref: Some(ref_name.as_str().to_string()),
        explicit_knowledge_dir: true,
        task_scope: TaskGraphScope::default(),
    };
    (temp_dir, context)
}

fn graph_with_named_leaves(named_leaves: &[(&str, LeafKind)]) -> CodebaseGraphV1 {
    let root_id = "dir:.".to_string();
    let mut file_ids = Vec::with_capacity(named_leaves.len());
    let mut files = Vec::with_capacity(named_leaves.len());
    let mut leaves = Vec::with_capacity(named_leaves.len());

    for (index, (name, kind)) in named_leaves.iter().enumerate() {
        let stem = name.to_ascii_lowercase();
        let file_location = format!("src/{stem}.rs");
        let file_id = format!("file:{file_location}");
        let leaf_location = format!("{file_location}#{name}");
        let leaf_id = format!("symbol:{leaf_location}:{kind}");
        file_ids.push(file_id.clone());
        files.push(file_node(
            &file_id,
            &format!("{stem}.rs"),
            &file_location,
            Some(&root_id),
            vec![leaf_id.clone()],
        ));
        leaves.push(leaf_node_with_kind(
            &leaf_id,
            name,
            &leaf_location,
            Some(&file_id),
            (index + 1) as u32,
            kind.clone(),
        ));
    }

    CodebaseGraphV1 {
        root_dir_id: root_id.clone(),
        dirs: vec![DirNode {
            base: base_node(&root_id, ".", ".", "", None),
            dir_children: Vec::new(),
            file_children: file_ids,
        }],
        files,
        leaves,
    }
}

fn graph_for_default_ranking_snapshot() -> CodebaseGraphV1 {
    let root_id = "dir:.".to_string();
    let code_file_id = "file:src/fixture.rs".to_string();
    let doc_file_id = "file:README.md".to_string();
    let code_leaf_id = "symbol:src/fixture.rs#fixture_fn:function".to_string();
    let doc_leaf_id = "symbol:README.md#fixture_doc:section".to_string();

    CodebaseGraphV1 {
        root_dir_id: root_id.clone(),
        dirs: vec![DirNode {
            base: base_node(&root_id, ".", ".", "", None),
            dir_children: Vec::new(),
            file_children: vec![code_file_id.clone(), doc_file_id.clone()],
        }],
        files: vec![
            FileNode {
                base: base_node(
                    &code_file_id,
                    "fixture.rs",
                    "src/fixture.rs",
                    "rust",
                    Some(&root_id),
                ),
                extension: Some("rs".to_string()),
                source_blob_hash: None,
                source: String::new(),
                imports: Vec::new(),
                exports: Vec::new(),
                re_exports: Vec::new(),
                leaf_children: vec![code_leaf_id.clone()],
            },
            FileNode {
                base: base_node(
                    &doc_file_id,
                    "README.md",
                    "README.md",
                    "markdown",
                    Some(&root_id),
                ),
                extension: Some("md".to_string()),
                source_blob_hash: None,
                source: String::new(),
                imports: Vec::new(),
                exports: Vec::new(),
                re_exports: Vec::new(),
                leaf_children: vec![doc_leaf_id.clone()],
            },
        ],
        leaves: vec![
            LeafNode {
                base: base_node(
                    &code_leaf_id,
                    "fixture_fn",
                    "src/fixture.rs#fixture_fn",
                    "rust",
                    Some(&code_file_id),
                ),
                kind: LeafKind::Function,
                source: String::new(),
                source_blob_hash: None,
                source_hash: None,
                file_hash_at_capture: None,
                history: Vec::new(),
                input_signature: Vec::new(),
                output_signature: Vec::new(),
                start_line: Some(1),
                end_line: Some(1),
                children: Vec::new(),
            },
            LeafNode {
                base: base_node(
                    &doc_leaf_id,
                    "fixture_doc",
                    "README.md#fixture_doc",
                    "markdown",
                    Some(&doc_file_id),
                ),
                kind: LeafKind::Section { depth: 1 },
                source: String::new(),
                source_blob_hash: None,
                source_hash: None,
                file_hash_at_capture: None,
                history: Vec::new(),
                input_signature: Vec::new(),
                output_signature: Vec::new(),
                start_line: Some(1),
                end_line: Some(1),
                children: Vec::new(),
            },
        ],
    }
}

fn graph_with_matching_leaves(leaf_count: usize) -> CodebaseGraphV1 {
    let root_id = "dir:.".to_string();
    let file_id = "file:src/fixture.rs".to_string();
    let mut leaf_ids = Vec::with_capacity(leaf_count);
    let mut leaves = Vec::with_capacity(leaf_count);

    for index in 0..leaf_count {
        let name = format!("fixture_{index}");
        let leaf_id = format!("symbol:src/fixture.rs#{name}:function");
        leaf_ids.push(leaf_id.clone());
        leaves.push(leaf_node(
            &leaf_id,
            &name,
            &format!("src/fixture.rs#{name}"),
            Some(&file_id),
            (index + 1) as u32,
        ));
    }

    CodebaseGraphV1 {
        root_dir_id: root_id.clone(),
        dirs: vec![DirNode {
            base: base_node(&root_id, ".", ".", "", None),
            dir_children: Vec::new(),
            file_children: vec![file_id.clone()],
        }],
        files: vec![FileNode {
            base: base_node(
                &file_id,
                "fixture.rs",
                "src/fixture.rs",
                "rust",
                Some(&root_id),
            ),
            extension: Some("rs".to_string()),
            source_blob_hash: None,
            source: String::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            re_exports: Vec::new(),
            leaf_children: leaf_ids,
        }],
        leaves,
    }
}

fn graph_for_sql_search_tests() -> CodebaseGraphV1 {
    let root_id = "dir:.".to_string();
    let src_dir_id = "dir:src".to_string();
    let core_dir_id = "dir:src/core".to_string();
    let special_dir_id = "dir:src/special%_dir".to_string();
    let lookalike_dir_id = "dir:src/specialABdir".to_string();
    let core_file_id = "file:src/core/main.rs".to_string();
    let special_file_id = "file:src/special%_dir/mod.rs".to_string();
    let lookalike_file_id = "file:src/specialABdir/mod.rs".to_string();
    let unique_leaf_id = "symbol:src/core/main.rs#UniqueSymbol:function".to_string();
    let escaped_leaf_id = "symbol:src/special%_dir/mod.rs#EscapedSymbol:function".to_string();
    let lookalike_leaf_id = "symbol:src/specialABdir/mod.rs#LookalikeSymbol:function".to_string();

    CodebaseGraphV1 {
        root_dir_id: root_id.clone(),
        dirs: vec![
            DirNode {
                base: base_node(&root_id, ".", ".", "", None),
                dir_children: vec![src_dir_id.clone()],
                file_children: Vec::new(),
            },
            DirNode {
                base: base_node(&src_dir_id, "src", "src/", "", Some(&root_id)),
                dir_children: vec![
                    core_dir_id.clone(),
                    special_dir_id.clone(),
                    lookalike_dir_id.clone(),
                ],
                file_children: Vec::new(),
            },
            DirNode {
                base: base_node(&core_dir_id, "core", "src/core/", "", Some(&src_dir_id)),
                dir_children: Vec::new(),
                file_children: vec![core_file_id.clone()],
            },
            DirNode {
                base: base_node(
                    &special_dir_id,
                    "special%_dir",
                    "src/special%_dir/",
                    "",
                    Some(&src_dir_id),
                ),
                dir_children: Vec::new(),
                file_children: vec![special_file_id.clone()],
            },
            DirNode {
                base: base_node(
                    &lookalike_dir_id,
                    "specialABdir",
                    "src/specialABdir/",
                    "",
                    Some(&src_dir_id),
                ),
                dir_children: Vec::new(),
                file_children: vec![lookalike_file_id.clone()],
            },
        ],
        files: vec![
            file_node(
                &core_file_id,
                "main.rs",
                "src/core/main.rs",
                Some(&core_dir_id),
                vec![unique_leaf_id.clone()],
            ),
            file_node(
                &special_file_id,
                "mod.rs",
                "src/special%_dir/mod.rs",
                Some(&special_dir_id),
                vec![escaped_leaf_id.clone()],
            ),
            file_node(
                &lookalike_file_id,
                "mod.rs",
                "src/specialABdir/mod.rs",
                Some(&lookalike_dir_id),
                vec![lookalike_leaf_id.clone()],
            ),
        ],
        leaves: vec![
            leaf_node(
                &unique_leaf_id,
                "UniqueSymbol",
                "src/core/main.rs#UniqueSymbol",
                Some(&core_file_id),
                1,
            ),
            leaf_node(
                &escaped_leaf_id,
                "EscapedSymbol",
                "src/special%_dir/mod.rs#EscapedSymbol",
                Some(&special_file_id),
                2,
            ),
            leaf_node(
                &lookalike_leaf_id,
                "LookalikeSymbol",
                "src/specialABdir/mod.rs#LookalikeSymbol",
                Some(&lookalike_file_id),
                3,
            ),
        ],
    }
}

fn graph_with_repeated_name_leaves(leaf_count: usize, name: &str) -> CodebaseGraphV1 {
    let root_id = "dir:.".to_string();
    let dir_id = "dir:src/limit".to_string();
    let mut file_ids = Vec::with_capacity(leaf_count);
    let mut files = Vec::with_capacity(leaf_count);
    let mut leaves = Vec::with_capacity(leaf_count);

    for index in 0..leaf_count {
        let file_id = format!("file:src/limit/file_{index:05}.rs");
        let location = format!("src/limit/file_{index:05}.rs");
        let leaf_id = format!("symbol:{location}#{name}:function");
        file_ids.push(file_id.clone());
        files.push(file_node(
            &file_id,
            &format!("file_{index:05}.rs"),
            &location,
            Some(&dir_id),
            vec![leaf_id.clone()],
        ));
        leaves.push(leaf_node(
            &leaf_id,
            name,
            &format!("{location}#{name}"),
            Some(&file_id),
            (index + 1) as u32,
        ));
    }

    CodebaseGraphV1 {
        root_dir_id: root_id.clone(),
        dirs: vec![
            DirNode {
                base: base_node(&root_id, ".", ".", "", None),
                dir_children: vec![dir_id.clone()],
                file_children: Vec::new(),
            },
            DirNode {
                base: base_node(&dir_id, "limit", "src/limit/", "", Some(&root_id)),
                dir_children: Vec::new(),
                file_children: file_ids,
            },
        ],
        files,
        leaves,
    }
}

fn index_reader_for_graph(graph: &CodebaseGraphV1) -> (tempfile::TempDir, GraphIndexReader) {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let store = GraphObjectStore::new(temp_dir.path().join("graph"));
    let current_ref = store.write_graph(graph).expect("write graph");
    let reader = GraphIndexReader::open_current(
        store.graph_sqlite_index_path(),
        &current_ref.root_graph_hash,
    )
    .expect("open sqlite index")
    .expect("current sqlite index");
    (temp_dir, reader)
}

fn file_node(
    id: &str,
    name: &str,
    location: &str,
    parent_id: Option<&str>,
    leaf_children: Vec<String>,
) -> FileNode {
    FileNode {
        base: base_node(id, name, location, "rust", parent_id),
        extension: Some("rs".to_string()),
        source_blob_hash: None,
        source: String::new(),
        imports: Vec::new(),
        exports: Vec::new(),
        re_exports: Vec::new(),
        leaf_children,
    }
}

fn search_hits_for_leaves(leaves: &[LeafNode]) -> Vec<SearchHit<'_>> {
    leaves
        .iter()
        .map(|leaf| SearchHit {
            node: GraphNodeRef::Leaf(leaf),
            matched_lines: Vec::new(),
        })
        .collect()
}

fn hit_ids(hits: &[SearchHit<'_>]) -> Vec<String> {
    hits.iter().map(|hit| hit.node.id().to_string()).collect()
}

fn leaf_node(id: &str, name: &str, location: &str, parent_id: Option<&str>, line: u32) -> LeafNode {
    leaf_node_with_kind(id, name, location, parent_id, line, LeafKind::Function)
}

fn leaf_node_with_kind(
    id: &str,
    name: &str,
    location: &str,
    parent_id: Option<&str>,
    line: u32,
    kind: LeafKind,
) -> LeafNode {
    LeafNode {
        base: base_node(id, name, location, "rust", parent_id),
        kind,
        source: String::new(),
        source_blob_hash: None,
        source_hash: None,
        file_hash_at_capture: None,
        history: Vec::new(),
        input_signature: Vec::new(),
        output_signature: Vec::new(),
        start_line: Some(line),
        end_line: Some(line),
        children: Vec::new(),
    }
}

fn base_node(
    id: &str,
    name: &str,
    location: &str,
    language: &str,
    parent_id: Option<&str>,
) -> BaseNodeFields {
    BaseNodeFields {
        id: id.to_string(),
        identity_key: id.to_string(),
        object_hash: None,
        name: name.to_string(),
        location: location.to_string(),
        language: language.to_string(),
        description: String::new(),
        parent_id: parent_id.map(str::to_string),
        is_locked: false,
        lineage_locked: false,
        lock_owner: None,
        lock_reason: String::new(),
    }
}

fn sql_substring_selectors(
    reader: &GraphIndexReader,
    query: &str,
    include_non_code: bool,
    limit: usize,
) -> Vec<String> {
    let scan_cap = default_ranking_search_limit(limit);
    let rows = reader
        .search_substring(&query.trim().to_lowercase(), scan_cap)
        .expect("sql substring search");
    let ranked = rank_sql_default_search_results(rows, include_non_code, query);
    ranked
        .into_iter()
        .take(limit)
        .map(|row| selector_for_search_row(&row))
        .collect()
}

fn fallback_selectors(
    graph: &CodebaseGraphV1,
    query: &str,
    include_non_code: bool,
    limit: usize,
) -> Vec<String> {
    default_search(DefaultSearchInput {
        graph,
        query,
        include_non_code,
        limit,
    })
    .expect("default search")
    .hits
    .into_iter()
    .map(|hit| hit.selector)
    .collect()
}

// --- fuzzy tests ---

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

// --- ranking tests ---

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

// --- sql tests ---

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
