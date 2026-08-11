// Migrated from sqlite/id_allocator.rs per ORB-00231
use std::path::Path;

use rusqlite::Connection;
use tempfile::TempDir;

use super::super::*;

#[test]
fn schema_is_idempotent_for_preexisting_semantic_db() {
    let conn = Connection::open_in_memory().expect("open db");
    conn.execute_batch("CREATE TABLE embeddings(source_id TEXT);")
        .expect("legacy semantic table");

    ensure_id_allocation_schema(&conn).expect("schema");
    ensure_id_allocation_schema(&conn).expect("schema again");

    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='id_allocations'",
            [],
            |row| row.get(0),
        )
        .expect("table exists");
    assert_eq!(exists, 1);
    assert!(id_allocations_has_column(&conn, "body_path"));
}

#[test]
fn schema_adds_body_path_to_existing_id_allocations_table() {
    let conn = Connection::open_in_memory().expect("open db");
    conn.execute_batch(
        "CREATE TABLE id_allocations (
                kind TEXT NOT NULL,
                id TEXT NOT NULL,
                allocated_at INTEGER NOT NULL,
                worktree_root TEXT NOT NULL,
                branch TEXT,
                status TEXT NOT NULL,
                PRIMARY KEY (kind, id)
            );",
    )
    .expect("legacy allocation table");

    ensure_id_allocation_schema(&conn).expect("schema");

    assert!(id_allocations_has_column(&conn, "body_path"));
}

#[test]
fn open_creates_schema_in_preexisting_semantic_db_file() {
    let temp = TempDir::new().expect("tempdir");
    let config = allocator_config(temp.path());
    if let Some(parent) = config.semantic_db_path.parent() {
        std::fs::create_dir_all(parent).expect("state dir");
    }
    {
        let conn = Connection::open(&config.semantic_db_path).expect("open db");
        conn.execute_batch("CREATE TABLE embeddings(source_id TEXT);")
            .expect("legacy semantic table");
    }

    let _allocator = IdAllocator::open(config.clone()).expect("allocator");
    let conn = Connection::open(&config.semantic_db_path).expect("reopen db");
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='id_allocations'",
            [],
            |row| row.get(0),
        )
        .expect("table exists");
    assert_eq!(exists, 1);
}

#[test]
fn abandoned_learning_allocation_advances_sequence_but_is_hidden() {
    let temp = TempDir::new().expect("tempdir");
    let allocator = IdAllocator::open(allocator_config(temp.path())).expect("allocator");

    let first = allocator.allocate_learning().expect("first");
    allocator
        .abandon_learning(&first.id)
        .expect("abandon first");
    let second = allocator.allocate_learning().expect("second");

    assert_eq!(first.id, "L-0001");
    assert_eq!(second.id, "L-0002");
    assert!(
        allocator
            .learning_allocation(&first.id)
            .expect("first allocation")
            .is_none()
    );
    let visible: Vec<_> = allocator
        .learning_allocations()
        .expect("allocations")
        .into_iter()
        .map(|record| record.id)
        .collect();
    assert_eq!(visible, vec!["L-0002"]);
}

/// [ORB-10501] A worktree reaped after allocation leaves rows nothing can ever
/// resolve. The guarded abandon retires both a `reserved` row and a `merged`
/// row whose recorded `body_path` died with the worktree, and the ids stay
/// consumed so no future allocation reuses them.
/// [ORB-10501] The guard is what separates a live sibling worktree from a
/// reaped one: an allocation whose worktree still exists is refused, so a
/// caller working from a stale scan cannot retire a recoverable id.
#[test]
fn refuses_to_abandon_an_allocation_whose_worktree_still_exists() {
    let temp = TempDir::new().expect("tempdir");
    let allocator = IdAllocator::open(allocator_config(temp.path())).expect("allocator");
    let learning = allocator.allocate_learning().expect("learning");

    let error = allocator
        .abandon_orphaned_learning(&learning.id)
        .expect_err("a live worktree must be refused");

    assert!(error.to_string().contains("still exists"), "{error}");
    assert!(
        allocator
            .learning_allocation(&learning.id)
            .expect("learning row")
            .is_some(),
        "a refused repair must leave the row untouched"
    );
}

#[test]
fn learning_id_format_migration_renames_and_is_idempotent() {
    let temp = TempDir::new().expect("tempdir");
    let learning_root = temp.path().join(".orbit/learnings");
    write_legacy_learning(&learning_root, "L20260518-2", "2026-05-18T00:00:00Z", None);
    write_legacy_learning(
        &learning_root,
        "L20260517-1",
        "2026-05-17T00:00:00Z",
        Some("L20260518-2"),
    );

    let config = allocator_config(temp.path());
    let allocator = IdAllocator::open(config.clone()).expect("allocator");
    let report = allocator.migrate_learning_ids().expect("migrate");
    assert_eq!(
        report.renames,
        vec![
            LearningIdRename {
                old_id: "L20260517-1".to_string(),
                new_id: "L-0001".to_string(),
            },
            LearningIdRename {
                old_id: "L20260518-2".to_string(),
                new_id: "L-0002".to_string(),
            },
        ]
    );

    let first =
        std::fs::read_to_string(learning_root.join("L-0001/learning.yaml")).expect("first yaml");
    assert!(first.contains("id: L-0001"));
    assert!(first.contains("- L20260517-1"));
    assert!(first.contains("supersedes: L-0002"));
    assert!(!learning_root.join("L20260517-1").exists());
    assert_eq!(allocation_count(&config.semantic_db_path), 2);

    let second_report = allocator.migrate_learning_ids().expect("migrate again");
    assert!(second_report.is_empty());
    assert_eq!(allocation_count(&config.semantic_db_path), 2);
}

fn allocator_config(root: &Path) -> IdAllocatorConfig {
    allocator_config_for_worktree(root, root)
}

fn allocator_config_for_worktree(root: &Path, worktree: &Path) -> IdAllocatorConfig {
    IdAllocatorConfig::new(
        root.join(".orbit/state/semantic.db"),
        root.join(".orbit/state/.id_alloc.lock"),
        root.join(".orbit"),
        worktree.to_path_buf(),
        worktree.join(".orbit/learnings"),
    )
}

fn allocation_count(db_path: &Path) -> i64 {
    let conn = Connection::open(db_path).expect("open db");
    conn.query_row("SELECT COUNT(*) FROM id_allocations", [], |row| row.get(0))
        .expect("count")
}

fn id_allocations_has_column(conn: &Connection, column: &str) -> bool {
    let mut stmt = conn
        .prepare("PRAGMA table_info(id_allocations)")
        .expect("table info");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query columns");
    rows.into_iter()
        .map(|row| row.expect("column"))
        .any(|name| name == column)
}

fn write_legacy_learning(
    learning_root: &Path,
    id: &str,
    created_at: &str,
    supersedes: Option<&str>,
) {
    let dir = learning_root.join(id);
    std::fs::create_dir_all(&dir).expect("learning dir");
    let supersedes_line = supersedes
        .map(|value| format!("supersedes: {value}\n"))
        .unwrap_or_default();
    std::fs::write(
            dir.join("learning.yaml"),
            format!(
                "schema_version: 1\nid: {id}\nstatus: active\nscope:\n  paths: []\n  tags: []\nsummary: Test\nbody: ''\nevidence: []\n{supersedes_line}created_at: {created_at}\nupdated_at: {created_at}\n"
            ),
        )
        .expect("learning yaml");
}
