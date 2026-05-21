//! Tests for GraphReadOptions gating of blob source hydration.

use super::super::*;
use crate::graph::nodes::{BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode};

#[test]
fn graph_read_options_gate_blob_source_hydration() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let store = GraphObjectStore::new(temp_dir.path());
    let graph = fixture_graph();
    let current_ref = store.write_graph(&graph).expect("write graph");
    let ref_name = RefName::new("main").expect("valid ref");
    store
        .write_ref_atomic(&ref_name, &current_ref)
        .expect("write ref");

    let default_graph = store
        .read_graph(
            &ref_name,
            None,
            Some(&ref_name),
            GraphReadOptions::default(),
        )
        .expect("read graph without hydration");
    assert_eq!(default_graph.files[0].source, "");
    assert!(default_graph.files[0].source_blob_hash.is_some());
    assert_eq!(default_graph.leaves[0].source, "");
    assert!(default_graph.leaves[0].source_blob_hash.is_some());

    let hydrated_graph = store
        .read_graph(
            &ref_name,
            None,
            Some(&ref_name),
            GraphReadOptions {
                hydrate_file_source: true,
                hydrate_leaf_source: true,
            },
        )
        .expect("read graph with hydration");
    assert_eq!(hydrated_graph.files[0].source, graph.files[0].source);
    assert_eq!(hydrated_graph.leaves[0].source, graph.leaves[0].source);
}

// --- test helpers (duplicated from write_graph concern for isolation; small fixtures) ---

fn fixture_graph() -> CodebaseGraphV1 {
    CodebaseGraphV1 {
        root_dir_id: "dir-root".to_string(),
        dirs: vec![DirNode {
            base: base_node("dir-root", ".", "./", None),
            dir_children: Vec::new(),
            file_children: vec!["file-lib".to_string()],
        }],
        files: vec![FileNode {
            base: base_node("file-lib", "Lib.rs", "src/Lib.rs", Some("dir-root")),
            extension: Some("rs".to_string()),
            source_blob_hash: None,
            source: "pub fn greet() { helper(); }\n".to_string(),
            imports: Vec::new(),
            exports: vec!["greet".to_string()],
            re_exports: Vec::new(),
            leaf_children: vec!["leaf-greet".to_string()],
        }],
        leaves: vec![LeafNode {
            base: base_node("leaf-greet", "Greet", "src/Lib.rs#Greet", Some("file-lib")),
            kind: LeafKind::Function,
            source: "pub fn greet() { helper(); }\n".to_string(),
            source_blob_hash: None,
            source_hash: Some("source-hash".to_string()),
            file_hash_at_capture: Some("file-hash".to_string()),
            history: Vec::new(),
            input_signature: Vec::new(),
            output_signature: Vec::new(),
            start_line: Some(1),
            end_line: Some(1),
            children: Vec::new(),
        }],
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
