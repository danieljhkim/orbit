use rusqlite::{Connection, params};

use crate::query::tests::support::{TestWorktree, insert_file, open_connection, open_graph};
use crate::{RefKind, Selector, SyncPolicy};

fn insert_relation(
    conn: &Connection,
    def_file: &str,
    from_qualified: &str,
    to_qualified: &str,
    kind: &str,
) {
    conn.execute(
        "INSERT INTO relations (
            from_qualified, to_qualified, kind, def_file, def_span_start, def_span_end, confidence
         ) VALUES (?1, ?2, ?3, ?4, 0, 1, 'exact')",
        params![from_qualified, to_qualified, kind, def_file],
    )
    .expect("insert relation");
}

fn trait_selector(file: &str, name: &str) -> Selector {
    Selector::Symbol {
        path: file.to_string(),
        symbol: name.to_string(),
        kind: "trait".to_string(),
    }
}

#[test]
fn implementors_lists_types_implementing_a_trait() {
    let worktree = TestWorktree::new("impl-basic");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    insert_file(&conn, "crates/x/src/audit.rs", "rust", "");
    insert_relation(
        &conn,
        "crates/x/src/audit.rs",
        "NullSink",
        "AuditSink",
        "impl",
    );
    insert_relation(
        &conn,
        "crates/x/src/audit.rs",
        "InMemorySink",
        "AuditSink",
        "impl",
    );

    let result = graph
        .implementors(&trait_selector("crates/x/src/audit.rs", "AuditSink"))
        .expect("implementors query");

    assert_eq!(result.trait_name, "AuditSink");
    let names: Vec<&str> = result
        .implementors
        .iter()
        .map(|item| item.type_qualified.as_str())
        .collect();
    // ORDER BY def_file, from_qualified: InMemorySink < NullSink.
    assert_eq!(names, vec!["InMemorySink", "NullSink"]);
    assert!(
        result
            .implementors
            .iter()
            .all(|item| item.kind == RefKind::Impl)
    );
    assert!(
        result
            .implementors
            .iter()
            .all(|item| item.trait_matched == "AuditSink")
    );
}

#[test]
fn implementors_matches_trailing_trait_segment() {
    let worktree = TestWorktree::new("impl-trailing");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    insert_file(&conn, "crates/x/src/error.rs", "rust", "");
    insert_relation(
        &conn,
        "crates/x/src/error.rs",
        "AgentLoopError",
        "std::fmt::Display",
        "impl",
    );

    let result = graph
        .implementors(&trait_selector("crates/x/src/error.rs", "Display"))
        .expect("implementors query");

    assert_eq!(result.trait_name, "Display");
    assert_eq!(result.implementors.len(), 1);
    assert_eq!(result.implementors[0].type_qualified, "AgentLoopError");
    assert_eq!(result.implementors[0].trait_matched, "std::fmt::Display");
}

#[test]
fn implementors_empty_for_non_trait_selector() {
    let worktree = TestWorktree::new("impl-nontrait");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    insert_file(&conn, "crates/x/src/audit.rs", "rust", "");
    insert_relation(
        &conn,
        "crates/x/src/audit.rs",
        "NullSink",
        "AuditSink",
        "impl",
    );

    let result = graph
        .implementors(&Selector::File {
            path: "crates/x/src/audit.rs".to_string(),
        })
        .expect("implementors query");

    assert!(result.implementors.is_empty());
}
