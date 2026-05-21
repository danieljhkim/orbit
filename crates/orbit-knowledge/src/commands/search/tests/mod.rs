#![allow(missing_docs)]

use crate::commands::{GraphCommandContext, TaskGraphScope};
use crate::graph::GraphIndexReader;
use crate::graph::nodes::{BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode};
use crate::graph::object_store::{GraphObjectStore, RefName};

use super::*;

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

mod fuzzy;
mod ranking;
mod sql;
