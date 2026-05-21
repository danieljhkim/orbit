//! Tests for write_graph (including sqlite index creation, idempotency, replacement for different refs).

use super::super::*;
use crate::graph::nodes::{BaseNodeFields, CodebaseGraphV1, DirNode, FileNode, LeafKind, LeafNode};
use rusqlite::Connection;
use std::collections::BTreeMap;

#[test]
fn write_graph_creates_sqlite_index_schema_and_rows() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let store = GraphObjectStore::new(temp_dir.path());
    let graph = fixture_graph();

    let current_ref = store.write_graph(&graph).expect("write graph");
    let index_path = store.graph_sqlite_index_path();
    assert!(index_path.is_file());

    let conn = Connection::open(&index_path).expect("open sqlite index");
    let tables = sqlite_master_names(&conn, "table");
    assert_eq!(tables, vec!["child", "file_summary", "meta", "node"]);
    let indexes = sqlite_master_names(&conn, "index");
    assert!(indexes.contains(&"idx_child_parent_ordinal".to_string()));
    assert!(indexes.contains(&"idx_file_symbol_count".to_string()));
    assert!(indexes.contains(&"idx_node_location_lower".to_string()));
    assert!(indexes.contains(&"idx_node_name_lower".to_string()));
    assert!(indexes.contains(&"idx_node_parent".to_string()));
    assert!(indexes.contains(&"idx_node_parent_ordinal".to_string()));
    assert!(indexes.contains(&"idx_node_selector".to_string()));

    let meta = sqlite_meta(&conn);
    assert_eq!(meta.get("schema_version").map(String::as_str), Some("6"));
    assert_eq!(
        meta.get("graph_ref").map(String::as_str),
        Some(current_ref.root_graph_hash.as_str())
    );

    let expected_node_count = graph.dirs.len() + graph.files.len() + graph.leaves.len();
    let node_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM node", [], |row| row.get(0))
        .expect("count nodes");
    assert_eq!(node_count, expected_node_count as i64);

    let selector_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM node WHERE selector IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("count selectors");
    assert_eq!(selector_count, expected_node_count as i64);

    let (name_lower, language, location_lower, selector, scan_order): (
        String,
        String,
        String,
        String,
        i64,
    ) = conn
        .query_row(
            "SELECT name_lower, language, location_lower, selector, scan_order FROM node WHERE id = 'leaf-greet'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("leaf row");
    assert_eq!(name_lower, "greet");
    assert_eq!(language, "rust");
    assert_eq!(location_lower, "src/lib.rs#greet");
    assert_eq!(selector, "symbol:src/Lib.rs#Greet:function");
    assert_eq!(scan_order, 2);

    let (symbol_count, path): (i64, String) = conn
        .query_row(
            "SELECT symbol_count, path FROM file_summary WHERE file_id = 'file-lib'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("file summary");
    assert_eq!(symbol_count, 1);
    assert_eq!(path, "src/Lib.rs");
}

#[test]
fn write_graph_rebuilds_sqlite_index_idempotently_for_same_ref() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let store = GraphObjectStore::new(temp_dir.path());
    let graph = fixture_graph();

    let first_ref = store.write_graph(&graph).expect("write graph first");
    let first_conn = Connection::open(store.graph_sqlite_index_path()).expect("open first");
    let first_meta = sqlite_meta(&first_conn);
    let first_node_count: i64 = first_conn
        .query_row("SELECT COUNT(*) FROM node", [], |row| row.get(0))
        .expect("first node count");
    drop(first_conn);

    let second_ref = store.write_graph(&graph).expect("write graph second");
    let second_conn = Connection::open(store.graph_sqlite_index_path()).expect("open second");
    let second_meta = sqlite_meta(&second_conn);
    let second_node_count: i64 = second_conn
        .query_row("SELECT COUNT(*) FROM node", [], |row| row.get(0))
        .expect("second node count");

    assert_eq!(first_ref.root_graph_hash, second_ref.root_graph_hash);
    assert_eq!(first_meta, second_meta);
    assert_eq!(first_node_count, second_node_count);
}

#[test]
fn write_graph_replaces_sqlite_index_for_different_ref() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let store = GraphObjectStore::new(temp_dir.path());

    let first_ref = store
        .write_graph(&fixture_graph())
        .expect("write first graph");
    let second_graph = replacement_graph();
    let second_ref = store
        .write_graph(&second_graph)
        .expect("write second graph");
    assert_ne!(first_ref.root_graph_hash, second_ref.root_graph_hash);

    let conn = Connection::open(store.graph_sqlite_index_path()).expect("open sqlite index");
    let meta = sqlite_meta(&conn);
    assert_eq!(
        meta.get("graph_ref").map(String::as_str),
        Some(second_ref.root_graph_hash.as_str())
    );

    let node_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM node", [], |row| row.get(0))
        .expect("node count");
    assert_eq!(
        node_count,
        (second_graph.dirs.len() + second_graph.files.len() + second_graph.leaves.len()) as i64
    );
    let old_leaf_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM node WHERE id = 'leaf-greet'",
            [],
            |row| row.get(0),
        )
        .expect("old leaf count");
    assert_eq!(old_leaf_count, 0);
}

// --- test helpers (local to this concern) ---

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

fn replacement_graph() -> CodebaseGraphV1 {
    let mut graph = fixture_graph();
    graph.files[0].leaf_children = vec!["leaf-helper".to_string()];
    graph.leaves = vec![LeafNode {
        base: base_node(
            "leaf-helper",
            "Helper",
            "src/Lib.rs#Helper",
            Some("file-lib"),
        ),
        kind: LeafKind::Struct,
        source: "pub struct Helper;\n".to_string(),
        source_blob_hash: None,
        source_hash: Some("replacement-source-hash".to_string()),
        file_hash_at_capture: Some("replacement-file-hash".to_string()),
        history: Vec::new(),
        input_signature: Vec::new(),
        output_signature: Vec::new(),
        start_line: Some(3),
        end_line: Some(3),
        children: Vec::new(),
    }];
    graph
}

fn sqlite_meta(conn: &Connection) -> BTreeMap<String, String> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM meta ORDER BY key")
        .expect("prepare meta query");
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query meta")
        .map(|row| row.expect("meta row"))
        .collect()
}

fn sqlite_master_names(conn: &Connection, kind: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = ?1 ORDER BY name")
        .expect("prepare sqlite_master query");
    stmt.query_map([kind], |row| row.get(0))
        .expect("query sqlite_master")
        .map(|row| row.expect("sqlite_master row"))
        .collect()
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
