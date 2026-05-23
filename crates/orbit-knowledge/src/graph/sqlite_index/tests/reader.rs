//! Tests for GraphIndexReader::open / open_current (stale/missing/valid/readonly cases).
//! Fixtures build minimal graphs via GraphObjectStore.

use super::super::super::nodes::{
    BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode,
};
use super::super::super::object_store::GraphObjectStore;
use super::super::*; // items from sqlite_index (e.g. GRAPH_SQLITE_INDEX_SCHEMA_VERSION) + siblings
use rusqlite::Connection;

#[test]
fn open_missing_index_returns_none() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("missing.sqlite");

    let reader = GraphIndexReader::open(&path, "graph-a").expect("open missing");

    assert!(reader.is_none());
}

#[test]
fn open_stale_ref_returns_none() {
    let (_temp_dir, store, current_ref) = write_fixture_index();

    let reader = GraphIndexReader::open(store.graph_sqlite_index_path(), "different-ref")
        .expect("open stale ref");

    assert!(reader.is_none());
    assert_ne!(current_ref.root_graph_hash, "different-ref");
}

#[test]
fn open_stale_schema_returns_none() {
    let (_temp_dir, store, current_ref) = write_fixture_index();
    let index_path = store.graph_sqlite_index_path();
    let conn = Connection::open(&index_path).expect("open sqlite index for schema update");
    conn.execute(
        "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
        [GRAPH_SQLITE_INDEX_SCHEMA_VERSION
            .saturating_sub(1)
            .to_string()],
    )
    .expect("update schema version");
    drop(conn);

    let reader =
        GraphIndexReader::open(&index_path, &current_ref.root_graph_hash).expect("open index");

    assert!(reader.is_none());
}

#[test]
fn open_valid_index_counts_nodes_like_node_table() {
    let (_temp_dir, store, current_ref) = write_fixture_index();
    let index_path = store.graph_sqlite_index_path();
    let conn = Connection::open(&index_path).expect("open sqlite index");
    let expected_count: u64 = conn
        .query_row("SELECT COUNT(*) FROM node", [], |row| row.get::<_, i64>(0))
        .expect("count nodes")
        .try_into()
        .expect("node count is non-negative");
    drop(conn);

    let reader = GraphIndexReader::open(&index_path, &current_ref.root_graph_hash)
        .expect("open index")
        .expect("valid current index");

    assert_eq!(
        reader.count_nodes().expect("count reader nodes"),
        expected_count
    );
}

#[test]
fn open_valid_index_uses_read_only_connection() {
    let (_temp_dir, store, current_ref) = write_fixture_index();
    let reader = GraphIndexReader::open(
        store.graph_sqlite_index_path(),
        &current_ref.root_graph_hash,
    )
    .expect("open index")
    .expect("valid current index");

    let write_result = reader.conn.execute(
        "INSERT INTO meta (key, value) VALUES ('read_only_test', 'should_fail')",
        [],
    );

    assert!(write_result.is_err());
}

fn write_fixture_index() -> (
    tempfile::TempDir,
    GraphObjectStore,
    super::super::super::object_store::CurrentRef,
) {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let store = GraphObjectStore::new(temp_dir.path());
    let current_ref = store.write_graph(&fixture_graph()).expect("write graph");
    (temp_dir, store, current_ref)
}

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
