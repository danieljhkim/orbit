//! SQL/navigator equivalence harness (T20260510-1, T20260510-2).
//!
//! Holds the SQL fast-path query primitives accountable to the navigator
//! semantics they claim to mirror. The two production bugs that motivated
//! this module both shipped because no test asserted equivalence between
//! `GraphIndexReader::*` and `GraphNavigator::*` for a non-trivial fixture:
//!
//! - `search_substring` (was `search_exact_name` + `search_location_prefix`)
//!   must match `node_candidate_matches` substring-on-name-or-location.
//! - `children_of` (now backed by the `child` edge table) must match
//!   `GraphNodeRef::child_ids()` — the forward pointer — even when the
//!   graph's `parent_id` field is uninformative for nested leaf relationships.
//!
//! New SQL query primitives or new graph data shapes must extend this harness
//! with a corresponding assertion before they can be considered correct.

use std::collections::BTreeMap;
use std::path::Path;

use crate::graph::nodes::{BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode};
use crate::graph::object_store::{GraphObjectStore, RefName};
use crate::graph::{GraphIndexReader, navigator::GraphNavigator};
use crate::pipeline;
use crate::pipeline::context::BuildConfig;

const PYTHON_DUP_METHODS: &str = r#"
class Alpha:
    def save(self):
        return "alpha"

    class Inner:
        def save(self):
            return "inner"

class Beta:
    def save(self):
        return "beta"
"#;

const RUST_MULTI_IMPL: &str = r#"
trait Runner {
    fn run(&self);
}

struct Foo;

impl Foo {
    fn run(&self) {}
}

impl Runner for Foo {
    fn run(&self) {}
}
"#;

const JAVA_OVERLOADS: &str = r#"
class Client {
    void connect(int port) {}
    void connect(int port, String host) {}
}
"#;

const TYPESCRIPT_OVERLOADS: &str = r#"
function pick(value: string): string;
function pick(value: number): number;
function pick(value: string | number) {
    return value;
}

class Store {
    get(value: string): string;
    get(value: number): number;
    get(value: string | number) {
        return String(value);
    }
}
"#;

/// Fixture mirroring the django QuerySet shape from the T-2 reproduction:
/// a class leaf with method leaves nested as forward children, where
/// every leaf's `parent_id` field still points at the file (per build.rs).
/// This is the exact data shape that broke the parent-id reverse lookup.
fn nested_class_fixture() -> CodebaseGraphV1 {
    let dir_id = "dir-root";
    let file_id = "file-query";
    let class_id = "leaf-QuerySet";
    let method_a = "leaf-QuerySet-init";
    let method_b = "leaf-QuerySet-filter";
    let method_c = "leaf-QuerySet-exclude";
    let top_level_fn = "leaf-async-generator";

    CodebaseGraphV1 {
        root_dir_id: dir_id.to_string(),
        dirs: vec![DirNode {
            base: harness_base(dir_id, ".", "./", None),
            dir_children: Vec::new(),
            file_children: vec![file_id.to_string()],
        }],
        files: vec![FileNode {
            base: harness_base(
                file_id,
                "query.py",
                "django/db/models/query.py",
                Some(dir_id),
            ),
            extension: Some("py".to_string()),
            source_blob_hash: None,
            source: String::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            re_exports: Vec::new(),
            // Top-level fn precedes the class — mirrors the bug-report ordering.
            leaf_children: vec![
                top_level_fn.to_string(),
                class_id.to_string(),
                method_a.to_string(),
                method_b.to_string(),
                method_c.to_string(),
            ],
        }],
        leaves: vec![
            harness_leaf(
                top_level_fn,
                "_async_generator",
                "django/db/models/query.py#_async_generator",
                Some(file_id),
                LeafKind::Function,
                Vec::new(),
            ),
            // Class leaf names its methods via the forward children pointer.
            harness_leaf(
                class_id,
                "QuerySet",
                "django/db/models/query.py#QuerySet",
                Some(file_id),
                LeafKind::Class,
                vec![
                    method_a.to_string(),
                    method_b.to_string(),
                    method_c.to_string(),
                ],
            ),
            // Method leaves all have parent_id = file_id (the bug-shaped data).
            harness_leaf(
                method_a,
                "__init__",
                "django/db/models/query.py#QuerySet.__init__",
                Some(file_id),
                LeafKind::Method,
                Vec::new(),
            ),
            harness_leaf(
                method_b,
                "filter",
                "django/db/models/query.py#QuerySet.filter",
                Some(file_id),
                LeafKind::Method,
                Vec::new(),
            ),
            harness_leaf(
                method_c,
                "exclude",
                "django/db/models/query.py#QuerySet.exclude",
                Some(file_id),
                LeafKind::Method,
                Vec::new(),
            ),
        ],
    }
}

fn open_reader(graph: &CodebaseGraphV1) -> (tempfile::TempDir, GraphIndexReader) {
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

/// SQL `children_of` must enumerate the forward child pointers of `id` in
/// the same set and order as `GraphNavigator::get_children`.
#[test]
fn children_of_matches_navigator_for_class_with_nested_methods() {
    let graph = nested_class_fixture();
    let (_tmp, reader) = open_reader(&graph);
    let nav = GraphNavigator::new(&graph);

    for node_id in [
        "dir-root",
        "file-query",
        "leaf-QuerySet",      // <-- the case that broke pre-fix
        "leaf-QuerySet-init", // leaf with no children
        "leaf-async-generator",
    ] {
        let nav_children: Vec<String> = nav
            .get_children(node_id)
            .expect("nav children")
            .into_iter()
            .map(|node| node.id().to_string())
            .collect();
        let sql_children: Vec<String> = reader
            .children_of(node_id, usize::MAX / 2)
            .expect("sql children")
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(
            sql_children, nav_children,
            "children divergence for `{node_id}` (sql={sql_children:?}, nav={nav_children:?})"
        );
    }
}

/// SQL siblings — derived as `children_of(parent_id) minus self` in
/// `show.rs` — must match `GraphNavigator::get_siblings`.
#[test]
fn siblings_via_children_of_match_navigator() {
    let graph = nested_class_fixture();
    let (_tmp, reader) = open_reader(&graph);
    let nav = GraphNavigator::new(&graph);

    for node_id in [
        "leaf-QuerySet",
        "leaf-QuerySet-init",
        "leaf-async-generator",
        "file-query",
    ] {
        let nav_siblings: Vec<String> = nav
            .get_siblings(node_id)
            .expect("nav siblings")
            .into_iter()
            .map(|node| node.id().to_string())
            .collect();
        let sql_siblings: Vec<String> = match reader
            .node_by_id(node_id)
            .expect("sql node row")
            .and_then(|row| row.parent_id)
        {
            Some(parent_id) => reader
                .children_of(&parent_id, usize::MAX / 2)
                .expect("sql children for siblings")
                .into_iter()
                .filter(|row| row.id != node_id)
                .map(|row| row.id)
                .collect(),
            None => Vec::new(),
        };
        assert_eq!(
            sql_siblings, nav_siblings,
            "sibling divergence for `{node_id}`"
        );
    }
}

/// SQL `lineage_for(parent_id, depth)` must match
/// `GraphNavigator::get_lineage(id, false)` truncated to depth.
#[test]
fn lineage_for_matches_navigator_for_all_nodes() {
    let graph = nested_class_fixture();
    let (_tmp, reader) = open_reader(&graph);
    let nav = GraphNavigator::new(&graph);

    let depth = 8usize;
    for node_id in [
        "dir-root",
        "file-query",
        "leaf-QuerySet",
        "leaf-QuerySet-init",
    ] {
        let nav_lineage: Vec<String> = nav
            .get_lineage(node_id, false)
            .expect("nav lineage")
            .into_iter()
            .map(|node| node.id().to_string())
            .collect();
        let nav_lineage = if nav_lineage.len() > depth {
            nav_lineage[nav_lineage.len() - depth..].to_vec()
        } else {
            nav_lineage
        };
        let row = reader
            .node_by_id(node_id)
            .expect("sql node row")
            .expect("row present");
        let sql_lineage: Vec<String> = reader
            .lineage_for(row.parent_id.as_deref(), depth)
            .expect("sql lineage")
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(
            sql_lineage, nav_lineage,
            "lineage divergence for `{node_id}`"
        );
    }
}

/// SQL `search_substring` must produce the same hit set (in scan order)
/// as the navigator's substring-on-name-or-location matching.
#[test]
fn search_substring_matches_navigator_for_diverse_query_shapes() {
    let graph = nested_class_fixture();
    let (_tmp, reader) = open_reader(&graph);

    for query in [
        "QuerySet",  // exact symbol name
        "queryset",  // case-insensitive substring
        "filter",    // matches one nested method
        "django/db", // location prefix
        "models",    // location substring
        "ery",       // partial substring of QuerySet
        "_async",    // top-level fn match
        "%",         // LIKE metacharacter as literal
        "_",         // LIKE metacharacter as literal
        "",          // browse mode (empty query)
    ] {
        let sql_ids = sql_substring_ids(&reader, query);
        let nav_ids = navigator_substring_ids(&graph, query);
        assert_eq!(
            sql_ids, nav_ids,
            "search divergence for `{query}` (sql={sql_ids:?}, nav={nav_ids:?})"
        );
    }
}

/// `find_node_by_selector` must resolve to the same node id as the
/// navigator-equivalent selector lookup.
#[test]
fn find_node_by_selector_matches_in_memory_resolution() {
    let graph = nested_class_fixture();
    let (_tmp, reader) = open_reader(&graph);

    let cases = [
        ("dir:.", "dir-root"),
        ("file:django/db/models/query.py", "file-query"),
        (
            "symbol:django/db/models/query.py#QuerySet:class",
            "leaf-QuerySet",
        ),
        (
            "symbol:django/db/models/query.py#QuerySet.filter:method",
            "leaf-QuerySet-filter",
        ),
    ];
    for (selector, expected_id) in cases {
        let row = reader
            .find_node_by_selector(selector)
            .expect("sql selector lookup")
            .unwrap_or_else(|| panic!("selector `{selector}` not found in sql index"));
        assert_eq!(row.id, expected_id, "selector `{selector}` mapped wrong");
    }
}

#[test]
fn sql_leaves_equal_fallback_leaves_python_dup_methods() {
    assert_sql_leaves_equal_fallback_leaves("src/fixture.py", PYTHON_DUP_METHODS);
}

#[test]
fn sql_leaves_equal_fallback_leaves_rust_multi_impl() {
    assert_sql_leaves_equal_fallback_leaves("src/fixture.rs", RUST_MULTI_IMPL);
}

#[test]
fn sql_leaves_equal_fallback_leaves_java_overloads() {
    assert_sql_leaves_equal_fallback_leaves("src/fixture.java", JAVA_OVERLOADS);
}

#[test]
fn sql_leaves_equal_fallback_leaves_ts_overloads() {
    assert_sql_leaves_equal_fallback_leaves("src/fixture.ts", TYPESCRIPT_OVERLOADS);
}

fn assert_sql_leaves_equal_fallback_leaves(path: &str, source: &str) {
    let graph = fixture_graph_from_source(path, source);
    let (_tmp, reader) = open_reader(&graph);
    let expected_node_count = graph.dirs.len() + graph.files.len() + graph.leaves.len();
    assert_eq!(
        reader.count_nodes().expect("sql node count"),
        expected_node_count as u64,
        "SQL node table dropped rows for fixture `{path}`"
    );

    assert_eq!(
        sql_leaf_multiset(&reader, expected_node_count),
        fallback_leaf_multiset(&graph),
        "SQL leaves diverged from fallback graph leaves for fixture `{path}`"
    );
}

type LeafKey = (String, String, String, String);

fn sql_leaf_multiset(reader: &GraphIndexReader, limit: usize) -> BTreeMap<LeafKey, usize> {
    let mut leaves = BTreeMap::new();
    for row in reader
        .search_substring("", limit)
        .expect("sql browse rows")
        .into_iter()
        .filter(|row| row.node_type == "leaf")
    {
        let key = (
            row.id,
            row.name,
            row.location,
            row.kind.expect("leaf kind present"),
        );
        *leaves.entry(key).or_default() += 1;
    }
    leaves
}

fn fallback_leaf_multiset(graph: &CodebaseGraphV1) -> BTreeMap<LeafKey, usize> {
    let mut leaves = BTreeMap::new();
    for leaf in &graph.leaves {
        let key = (
            leaf.base.id.clone(),
            leaf.base.name.clone(),
            leaf.base.location.clone(),
            leaf.kind.to_string(),
        );
        *leaves.entry(key).or_default() += 1;
    }
    leaves
}

fn fixture_graph_from_source(path: &str, source: &str) -> CodebaseGraphV1 {
    let repo_dir = tempfile::tempdir().expect("repo tempdir");
    let repo = repo_dir.path();
    write_file(repo, path, source);

    let knowledge_dir = tempfile::tempdir().expect("knowledge tempdir");
    let ctx = pipeline::run_build(BuildConfig {
        repo_path: repo.to_path_buf(),
        output_dir: knowledge_dir.path().join("knowledge"),
        incremental: false,
        ref_name: Some(RefName::new("main").expect("valid ref")),
    })
    .expect("pipeline build succeeds");
    ctx.graph
}

fn write_file(repo: &Path, rel: &str, content: &str) {
    let path = repo.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create fixture parent");
    }
    std::fs::write(path, content).expect("write fixture file");
}

fn sql_substring_ids(reader: &GraphIndexReader, query: &str) -> Vec<String> {
    reader
        .search_substring(&query.to_lowercase(), 1024)
        .expect("sql substring")
        .into_iter()
        .map(|row| row.id)
        .collect()
}

/// Mirrors `GraphContextService::node_candidate_matches` for browse +
/// substring on `name_lower` OR `location_lower`, walking dirs → files →
/// leaves to match SQL `ORDER BY scan_order`.
fn navigator_substring_ids(graph: &CodebaseGraphV1, query: &str) -> Vec<String> {
    let q = query.to_lowercase();
    let browse = q.is_empty();
    let mut hits = Vec::new();
    for dir in &graph.dirs {
        if browse
            || dir.base.name.to_lowercase().contains(&q)
            || dir.base.location.to_lowercase().contains(&q)
        {
            hits.push(dir.base.id.clone());
        }
    }
    for file in &graph.files {
        if browse
            || file.base.name.to_lowercase().contains(&q)
            || file.base.location.to_lowercase().contains(&q)
        {
            hits.push(file.base.id.clone());
        }
    }
    for leaf in &graph.leaves {
        if browse
            || leaf.base.name.to_lowercase().contains(&q)
            || leaf.base.location.to_lowercase().contains(&q)
        {
            hits.push(leaf.base.id.clone());
        }
    }
    hits
}

fn harness_base(id: &str, name: &str, location: &str, parent_id: Option<&str>) -> BaseNodeFields {
    BaseNodeFields {
        id: id.to_string(),
        identity_key: id.to_string(),
        object_hash: None,
        name: name.to_string(),
        location: location.to_string(),
        language: "python".to_string(),
        description: String::new(),
        parent_id: parent_id.map(str::to_string),
        is_locked: false,
        lineage_locked: false,
        lock_owner: None,
        lock_reason: String::new(),
    }
}

fn harness_leaf(
    id: &str,
    name: &str,
    location: &str,
    parent_id: Option<&str>,
    kind: LeafKind,
    children: Vec<String>,
) -> LeafNode {
    LeafNode {
        base: harness_base(id, name, location, parent_id),
        kind,
        source: String::new(),
        source_blob_hash: None,
        source_hash: None,
        file_hash_at_capture: None,
        history: Vec::new(),
        input_signature: Vec::new(),
        output_signature: Vec::new(),
        start_line: Some(1),
        end_line: Some(1),
        children,
    }
}
