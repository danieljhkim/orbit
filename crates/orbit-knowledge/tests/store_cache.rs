use std::fs;
use std::path::{Path, PathBuf};

use orbit_knowledge::KnowledgeStore;
use orbit_knowledge::graph::nodes::{
    BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode,
};
use orbit_knowledge::graph::object_store::{GraphObjectStore, RefName};
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn pack_reuses_cached_object_and_blob_when_backing_files_are_removed()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TempDir::new()?;
    let knowledge_dir = temp.path().join("knowledge");
    write_manifest(&knowledge_dir)?;

    let graph_store = GraphObjectStore::new(knowledge_dir.join("graph"));
    let graph = sample_graph("cached_fn");
    let graph_ref = RefName::new("cache-test")?;
    let current_ref = graph_store.write_graph(&graph)?;
    graph_store.write_ref_atomic(&graph_ref, &current_ref)?;

    let index_path = knowledge_dir.join(&current_ref.index);
    let index_json: Value = serde_json::from_str(&fs::read_to_string(index_path)?)?;
    let nodes = index_json
        .get("nodes")
        .and_then(Value::as_object)
        .expect("index has nodes");
    assert!(nodes.len() >= 2);

    let leaf_id = "symbol:src/lib.rs#cached_fn:function";
    let object_hash = nodes
        .get(leaf_id)
        .and_then(|entry| entry.get("object_hash"))
        .and_then(Value::as_str)
        .expect("leaf object hash");
    let object_path = graph_object_path(&knowledge_dir, object_hash);
    let object_json: Value = serde_json::from_str(&fs::read_to_string(&object_path)?)?;
    let blob_hash = object_json
        .get("node")
        .and_then(|node| node.get("source_blob_hash"))
        .and_then(Value::as_str)
        .expect("leaf source blob hash");
    let blob_path = graph_blob_path(&knowledge_dir, blob_hash);

    let store = KnowledgeStore::open(&knowledge_dir, &graph_ref, None, None)?;
    let selector_text = "symbol:src/lib.rs#cached_fn:function";
    let first = store.pack(&[selector_text.parse()?])?;
    assert_eq!(first.total_nodes, 1);
    assert_eq!(
        first.entries[0].source.as_deref(),
        Some("pub fn cached_fn() -> &'static str {\n    \"cached\"\n}\n")
    );

    fs::remove_file(&object_path)?;
    fs::remove_file(&blob_path)?;

    let second = store.pack(&[selector_text.parse()?])?;
    assert_eq!(second.total_nodes, 1);
    assert_eq!(second.entries[0].source, first.entries[0].source);
    Ok(())
}

fn sample_graph(function_name: &str) -> CodebaseGraphV1 {
    let root_id = "dir:.".to_string();
    let file_id = "file:src/lib.rs".to_string();
    let leaf_id = format!("symbol:src/lib.rs#{function_name}:function");

    CodebaseGraphV1 {
        root_dir_id: root_id.clone(),
        dirs: vec![DirNode {
            base: base_node(&root_id, ".", ".", None),
            dir_children: Vec::new(),
            file_children: vec![file_id.clone()],
        }],
        files: vec![FileNode {
            base: base_node(&file_id, "lib.rs", "src/lib.rs", Some(&root_id)),
            extension: Some("rs".to_string()),
            source_blob_hash: None,
            source: String::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            re_exports: Vec::new(),
            leaf_children: vec![leaf_id.clone()],
        }],
        leaves: vec![LeafNode {
            base: base_node(
                &leaf_id,
                function_name,
                &format!("src/lib.rs#{function_name}"),
                Some(&file_id),
            ),
            kind: LeafKind::Function,
            source: "pub fn cached_fn() -> &'static str {\n    \"cached\"\n}\n".to_string(),
            source_blob_hash: None,
            source_hash: None,
            file_hash_at_capture: None,
            history: Vec::new(),
            input_signature: Vec::new(),
            output_signature: Vec::new(),
            start_line: Some(1),
            end_line: Some(3),
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
        parent_id: parent_id.map(ToOwned::to_owned),
        is_locked: false,
        lineage_locked: false,
        lock_owner: None,
        lock_reason: String::new(),
        task_ids: Vec::new(),
        structural_conflict: false,
    }
}

fn write_manifest(knowledge_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(knowledge_dir)?;
    fs::write(
        knowledge_dir.join("manifest.json"),
        "{\n  \"generated_at\": \"2026-04-26T00:00:00Z\"\n}\n",
    )?;
    Ok(())
}

fn graph_object_path(knowledge_dir: &Path, object_hash: &str) -> PathBuf {
    knowledge_dir
        .join("graph/objects")
        .join(&object_hash[..2])
        .join(format!("{object_hash}.json"))
}

fn graph_blob_path(knowledge_dir: &Path, blob_hash: &str) -> PathBuf {
    knowledge_dir
        .join("graph/blobs")
        .join(&blob_hash[..2])
        .join(format!("{blob_hash}.txt"))
}
