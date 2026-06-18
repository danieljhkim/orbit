use rusqlite::{Connection, params};

use crate::query::tests::support::{TestWorktree, insert_file, open_connection, open_graph};
use crate::{DepEdge, GraphError, Selector, SyncPolicy};

fn insert_import(
    conn: &Connection,
    from_file: &str,
    target_path: &str,
    target_symbol: Option<&str>,
) {
    conn.execute(
        "INSERT INTO imports (from_file, target_path, target_symbol) VALUES (?1, ?2, ?3)",
        params![from_file, target_path, target_symbol],
    )
    .expect("insert import");
}

#[test]
fn deps_lists_outbound_imports_for_a_file() {
    let worktree = TestWorktree::new("deps-file");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    insert_file(&conn, "crates/a/src/lib.rs", "rust", "");
    insert_file(&conn, "crates/a/src/other.rs", "rust", "");
    insert_import(
        &conn,
        "crates/a/src/lib.rs",
        "orbit_core::scheduler",
        Some("Scheduler"),
    );
    insert_import(&conn, "crates/a/src/lib.rs", "std::fmt", None);
    insert_import(&conn, "crates/a/src/other.rs", "serde", Some("Serialize"));

    let result = graph
        .deps(&Selector::File {
            path: "crates/a/src/lib.rs".to_string(),
        })
        .expect("deps query");

    assert_eq!(result.scope, "file:crates/a/src/lib.rs");
    assert_eq!(
        result.imports,
        vec![
            DepEdge {
                from_file: "crates/a/src/lib.rs".to_string(),
                target_path: "orbit_core::scheduler".to_string(),
                target_symbol: Some("Scheduler".to_string()),
            },
            DepEdge {
                from_file: "crates/a/src/lib.rs".to_string(),
                target_path: "std::fmt".to_string(),
                target_symbol: None,
            },
        ]
    );
}

#[test]
fn deps_aggregates_imports_under_a_directory() {
    let worktree = TestWorktree::new("deps-dir");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    insert_file(&conn, "crates/a/src/lib.rs", "rust", "");
    insert_file(&conn, "crates/a/src/other.rs", "rust", "");
    insert_file(&conn, "crates/b/src/lib.rs", "rust", "");
    insert_import(
        &conn,
        "crates/a/src/lib.rs",
        "orbit_core",
        Some("Scheduler"),
    );
    insert_import(&conn, "crates/a/src/other.rs", "serde", Some("Serialize"));
    insert_import(&conn, "crates/b/src/lib.rs", "tokio", None);

    let result = graph
        .deps(&Selector::Dir {
            path: "crates/a".to_string(),
        })
        .expect("deps query");

    assert_eq!(result.scope, "dir:crates/a");
    let froms: Vec<&str> = result
        .imports
        .iter()
        .map(|edge| edge.from_file.as_str())
        .collect();
    assert_eq!(froms, vec!["crates/a/src/lib.rs", "crates/a/src/other.rs"]);
}

#[test]
fn deps_rejects_non_path_selector() {
    let worktree = TestWorktree::new("deps-reject");
    let graph = open_graph(&worktree, SyncPolicy::Manual);

    let error = graph
        .deps(&Selector::Module {
            qualified: "orbit_core::scheduler".to_string(),
        })
        .expect_err("module selector should be rejected");
    assert!(matches!(error, GraphError::InvalidData { .. }));
}
