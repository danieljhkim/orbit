#![allow(missing_docs)]

// Tests for commands/show.rs live here as sibling under commands/tests/ per
// docs/design-patterns/test_layout.md. Explicit imports.

use crate::KnowledgeError;
use crate::graph::{BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode};
use crate::service::GraphContextService;
use orbit_graph_extract::Selector;

use super::super::show::{DID_YOU_MEAN_LIMIT, invalid_selector_resolution_error};

#[test]
fn failed_method_on_resolvable_type_returns_did_you_mean() {
    let graph = graph_with_methods(vec![
        "load_layered",
        "default_for_data_root",
        "workflow_base_branch",
    ]);
    let selector: Selector = "symbol:src/runtime.rs#<RuntimeConfig>::load:method"
        .parse()
        .expect("valid selector");

    let error = resolution_error_for(&graph, &selector);

    assert_eq!(
        error.did_you_mean.first().map(String::as_str),
        Some("symbol:src/runtime.rs#<RuntimeConfig>::load_layered:method")
    );
}

#[test]
fn failed_type_or_file_returns_no_suggestions() {
    let graph = graph_with_methods(vec!["load_layered"]);
    let missing_type: Selector = "symbol:src/runtime.rs#<MissingConfig>::load:method"
        .parse()
        .expect("valid selector");
    let missing_file: Selector = "symbol:src/missing.rs#<RuntimeConfig>::load:method"
        .parse()
        .expect("valid selector");

    assert!(
        resolution_error_for(&graph, &missing_type)
            .did_you_mean
            .is_empty()
    );
    assert!(
        resolution_error_for(&graph, &missing_file)
            .did_you_mean
            .is_empty()
    );
}

#[test]
fn method_suggestions_are_bounded_by_cap() {
    let graph = graph_with_methods(vec![
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf",
    ]);
    let selector: Selector = "symbol:src/runtime.rs#<RuntimeConfig>::missing:method"
        .parse()
        .expect("valid selector");

    let error = resolution_error_for(&graph, &selector);

    assert_eq!(error.did_you_mean.len(), DID_YOU_MEAN_LIMIT);
}

fn resolution_error_for(graph: &CodebaseGraphV1, selector: &Selector) -> KnowledgeError {
    let service = GraphContextService::new(graph);
    let error = service
        .resolve_selector(selector)
        .expect_err("selector should be unresolved");
    invalid_selector_resolution_error(graph, selector, error)
}

fn graph_with_methods(method_names: Vec<&str>) -> CodebaseGraphV1 {
    let file_id = "file:src/runtime.rs";
    let mut leaf_children = vec!["symbol:src/runtime.rs#RuntimeConfig:struct".to_string()];
    let mut leaves = vec![leaf_node(
        "symbol:src/runtime.rs#RuntimeConfig:struct",
        "RuntimeConfig",
        "src/runtime.rs#RuntimeConfig",
        file_id,
        LeafKind::Struct,
    )];

    for method_name in method_names {
        let location = format!("src/runtime.rs#<RuntimeConfig>::{method_name}");
        let id = format!("symbol:{location}:method");
        leaf_children.push(id.clone());
        leaves.push(leaf_node(
            &id,
            method_name,
            &location,
            file_id,
            LeafKind::Method,
        ));
    }

    CodebaseGraphV1 {
        root_dir_id: "dir:.".to_string(),
        dirs: vec![DirNode {
            base: base_node("dir:.", ".", ".", None),
            dir_children: Vec::new(),
            file_children: vec![file_id.to_string()],
        }],
        files: vec![FileNode {
            base: base_node(file_id, "runtime.rs", "src/runtime.rs", Some("dir:.")),
            extension: Some("rs".to_string()),
            source_blob_hash: None,
            source: String::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            re_exports: Vec::new(),
            leaf_children,
        }],
        leaves,
    }
}

fn leaf_node(id: &str, name: &str, location: &str, parent_id: &str, kind: LeafKind) -> LeafNode {
    LeafNode {
        base: base_node(id, name, location, Some(parent_id)),
        kind,
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
    }
}

fn base_node(id: &str, name: &str, location: &str, parent_id: Option<&str>) -> BaseNodeFields {
    BaseNodeFields {
        id: id.to_string(),
        identity_key: id.to_string(),
        object_hash: None,
        name: name.to_string(),
        location: location.to_string(),
        language: "rust".to_string(),
        description: String::new(),
        parent_id: parent_id.map(str::to_string),
        is_locked: false,
        lineage_locked: false,
        lock_owner: None,
        lock_reason: String::new(),
    }
}
