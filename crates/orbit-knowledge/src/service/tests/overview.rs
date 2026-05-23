#![allow(missing_docs)]

// Tests for service/overview.rs live here as a sibling under service/tests/ (see
// docs/design-patterns/test_layout.md). Explicit imports, no blanket super::*.

use crate::graph::nodes::{BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode};

use super::super::{FileOverview, GraphContextService, GraphOverview, TopFileEntry};

#[test]
fn top_files_matches_full_sort_for_large_fixture() {
    let graph = large_overview_graph();
    assert!(graph.files.len() >= 1000);

    let service = GraphContextService::new(&graph);
    let overview = service.overview(None);
    let expected = full_sort_top_files(&overview, 10);

    assert_eq!(overview.top_files(10), expected);
    assert_eq!(expected[0].selector, "file:src/top_tie_a.rs");
    assert_eq!(expected[1].selector, "file:src/top_tie_b.rs");
    assert_eq!(expected[0].symbol_count, expected[1].symbol_count);
}

fn full_sort_top_files(overview: &GraphOverview, limit: usize) -> Vec<TopFileEntry> {
    let mut files: Vec<&FileOverview> = overview.files.iter().collect();
    files.sort_by(|left, right| {
        right
            .symbol_count
            .cmp(&left.symbol_count)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.selector.cmp(&right.selector))
            .then_with(|| left.name.cmp(&right.name))
    });
    files
        .into_iter()
        .take(limit)
        .map(FileOverview::top_file_entry)
        .collect()
}

fn large_overview_graph() -> CodebaseGraphV1 {
    let root_id = "dir:.".to_string();
    let mut fixture = vec![
        ("src/top_tie_a.rs".to_string(), 200),
        ("src/top_tie_b.rs".to_string(), 200),
        ("src/top_001.rs".to_string(), 199),
        ("src/top_002.rs".to_string(), 198),
        ("src/top_003.rs".to_string(), 197),
        ("src/top_004.rs".to_string(), 196),
        ("src/top_005.rs".to_string(), 195),
        ("src/top_006.rs".to_string(), 194),
        ("src/top_007.rs".to_string(), 193),
        ("src/top_008.rs".to_string(), 192),
    ];
    for index in 0..1000 {
        fixture.push((format!("src/bulk_{index:04}.rs"), index % 50));
    }

    let mut file_children = Vec::with_capacity(fixture.len());
    let mut files = Vec::with_capacity(fixture.len());
    let mut leaves = Vec::new();

    for (file_index, (path, symbol_count)) in fixture.into_iter().enumerate() {
        let file_id = format!("file:{path}");
        let file_name = path.rsplit('/').next().unwrap_or(path.as_str()).to_string();
        let mut leaf_children = Vec::with_capacity(symbol_count);

        for symbol_index in 0..symbol_count {
            let symbol_name = format!("symbol_{file_index}_{symbol_index}");
            let leaf_location = format!("{path}#{symbol_name}");
            let leaf_id = format!("symbol:{leaf_location}:function");
            leaf_children.push(leaf_id.clone());
            leaves.push(LeafNode {
                base: base_node(
                    &leaf_id,
                    &symbol_name,
                    &leaf_location,
                    "rust",
                    Some(&file_id),
                ),
                kind: LeafKind::Function,
                source: String::new(),
                source_blob_hash: None,
                source_hash: None,
                file_hash_at_capture: None,
                history: Vec::new(),
                input_signature: Vec::new(),
                output_signature: Vec::new(),
                start_line: None,
                end_line: None,
                children: Vec::new(),
            });
        }

        file_children.push(file_id.clone());
        files.push(FileNode {
            base: base_node(&file_id, &file_name, &path, "rust", Some(&root_id)),
            extension: Some("rs".to_string()),
            source_blob_hash: None,
            source: String::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            re_exports: Vec::new(),
            leaf_children,
        });
    }

    CodebaseGraphV1 {
        root_dir_id: root_id.clone(),
        dirs: vec![DirNode {
            base: base_node(&root_id, ".", ".", "", None),
            dir_children: Vec::new(),
            file_children,
        }],
        files,
        leaves,
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
