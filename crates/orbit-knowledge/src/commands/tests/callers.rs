#![allow(missing_docs)]

use crate::commands::{GraphCommandContext, TaskGraphScope};
use crate::graph::object_store::{GraphObjectStore, RefName};
use crate::graph::{BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode};

use super::super::callers::{CallersInput, run};

#[test]
fn callers_uses_sql_index_without_object_hydration_or_reparse_on_read() {
    let (temp_dir, context) = context_for_graph(&callers_graph());
    std::fs::remove_dir_all(temp_dir.path().join("graph/objects")).expect("remove objects");
    std::fs::remove_dir_all(temp_dir.path().join("graph/blobs")).expect("remove blobs");

    let result = run(CallersInput {
        context,
        selector: "symbol:src/lib.rs#target:function".to_string(),
        requested_depth: Some(2),
    })
    .expect("callers via sqlite index");

    let hits = result
        .callers
        .into_iter()
        .map(|hit| (hit.selector, hit.distance, hit.via))
        .collect::<Vec<_>>();
    assert_eq!(
        hits,
        vec![
            (
                "symbol:src/lib.rs#direct:function".to_string(),
                1,
                "target".to_string()
            ),
            (
                "symbol:src/lib.rs#indirect:function".to_string(),
                2,
                "direct".to_string()
            ),
        ]
    );
}

fn context_for_graph(graph: &CodebaseGraphV1) -> (tempfile::TempDir, GraphCommandContext) {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let store = GraphObjectStore::new(temp_dir.path().join("graph"));
    let current_ref = store.write_graph(graph).expect("write graph");
    let ref_name = RefName::new("callers-test").expect("valid ref name");
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

fn callers_graph() -> CodebaseGraphV1 {
    let dir_id = "dir:.";
    let file_id = "file:src/lib.rs";
    let target_id = "symbol:src/lib.rs#target:function";
    let direct_id = "symbol:src/lib.rs#direct:function";
    let indirect_id = "symbol:src/lib.rs#indirect:function";
    CodebaseGraphV1 {
        root_dir_id: dir_id.to_string(),
        dirs: vec![DirNode {
            base: base_node(dir_id, ".", ".", None),
            dir_children: Vec::new(),
            file_children: vec![file_id.to_string()],
        }],
        files: vec![FileNode {
            base: base_node(file_id, "lib.rs", "src/lib.rs", Some(dir_id)),
            extension: Some("rs".to_string()),
            source_blob_hash: None,
            source: String::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            re_exports: Vec::new(),
            leaf_children: vec![
                target_id.to_string(),
                direct_id.to_string(),
                indirect_id.to_string(),
            ],
        }],
        leaves: vec![
            leaf_node(
                target_id,
                "target",
                "src/lib.rs#target",
                file_id,
                "fn target() {}",
            ),
            leaf_node(
                direct_id,
                "direct",
                "src/lib.rs#direct",
                file_id,
                "fn direct() { target(); }",
            ),
            leaf_node(
                indirect_id,
                "indirect",
                "src/lib.rs#indirect",
                file_id,
                "fn indirect() { direct(); }",
            ),
        ],
    }
}

fn leaf_node(id: &str, name: &str, location: &str, parent_id: &str, source: &str) -> LeafNode {
    LeafNode {
        base: base_node(id, name, location, Some(parent_id)),
        kind: LeafKind::Function,
        source: source.to_string(),
        source_blob_hash: None,
        source_hash: None,
        file_hash_at_capture: None,
        history: Vec::new(),
        input_signature: Vec::new(),
        output_signature: Vec::new(),
        start_line: Some(1),
        end_line: Some(1),
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
