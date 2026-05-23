// Migrated from sqlite/id_allocator.rs per ORB-00231
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

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
fn allocates_dense_adr_and_learning_ids() {
    let temp = TempDir::new().expect("tempdir");
    let allocator = IdAllocator::open(IdAllocatorConfig::new(
        temp.path().join("state/semantic.db"),
        temp.path().join("state/.id_alloc.lock"),
        temp.path().join(".orbit"),
        temp.path().to_path_buf(),
        temp.path().join(".orbit/adrs"),
        temp.path().join(".orbit/learnings"),
    ))
    .expect("allocator");

    assert_eq!(allocator.allocate_adr().expect("adr").id, "ADR-0001");
    assert_eq!(allocator.allocate_adr().expect("adr").id, "ADR-0002");
    assert_eq!(
        allocator.allocate_learning().expect("learning").id,
        "L-0001"
    );
    assert_eq!(
        allocator.allocate_learning().expect("learning").id,
        "L-0002"
    );
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

#[test]
fn backfills_existing_adrs_idempotently_and_allocates_after_max() {
    let temp = TempDir::new().expect("tempdir");
    let shared_root = temp.path().join(".orbit");
    let adr_root = shared_root.join("adrs");
    let adr_dir = adr_root.join("accepted/ADR-0007");
    std::fs::create_dir_all(&adr_dir).expect("adr dir");
    std::fs::write(
        adr_dir.join("adr.yaml"),
        "schema_version: 1\nid: ADR-0007\ncreated_at: 2026-05-17T00:00:00Z\n",
    )
    .expect("adr yaml");
    let config = allocator_config(temp.path());

    let allocator = IdAllocator::open(config.clone()).expect("allocator");
    assert_eq!(allocation_count(&config.semantic_db_path), 1);
    drop(allocator);

    let allocator = IdAllocator::open(config.clone()).expect("allocator reopen");
    assert_eq!(allocation_count(&config.semantic_db_path), 1);
    assert_eq!(allocator.allocate_adr().expect("allocate").id, "ADR-0008");
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

#[test]
fn multi_process_allocation_is_dense_for_adrs_and_learnings() {
    for kind in [IdAllocationKind::Adr, IdAllocationKind::Learning] {
        assert_multi_process_dense(kind);
    }
}

#[test]
fn allocate_ids_child() {
    let Ok(kind) = std::env::var("ORBIT_ID_ALLOCATOR_CHILD_KIND") else {
        return;
    };
    let root = std::env::var("ORBIT_ID_ALLOCATOR_CHILD_ROOT").expect("root env");
    let output = std::env::var("ORBIT_ID_ALLOCATOR_CHILD_OUTPUT").expect("output env");
    let count: usize = std::env::var("ORBIT_ID_ALLOCATOR_CHILD_COUNT")
        .expect("count env")
        .parse()
        .expect("count parse");
    let kind = match kind.as_str() {
        "adr" => IdAllocationKind::Adr,
        "learning" => IdAllocationKind::Learning,
        other => panic!("unknown kind {other}"),
    };
    let allocator = IdAllocator::open(allocator_config(Path::new(&root))).expect("allocator");
    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        let allocation = match kind {
            IdAllocationKind::Adr => allocator.allocate_adr(),
            IdAllocationKind::Learning => allocator.allocate_learning(),
        }
        .expect("allocate");
        ids.push(allocation.id);
    }
    std::fs::write(output, ids.join("\n")).expect("write ids");
}

fn assert_multi_process_dense(kind: IdAllocationKind) {
    let temp = TempDir::new().expect("tempdir");
    let exe = std::env::current_exe().expect("current exe");
    let count = 50usize;
    let output_a = temp.path().join("ids-a.txt");
    let output_b = temp.path().join("ids-b.txt");
    let kind_name = kind.as_str();

    let mut child_a = Command::new(&exe)
        .args([
            "--exact",
            "sqlite::id_allocator::tests::id_allocator::allocate_ids_child",
            "--nocapture",
        ])
        .env("ORBIT_ID_ALLOCATOR_CHILD_KIND", kind_name)
        .env("ORBIT_ID_ALLOCATOR_CHILD_ROOT", temp.path())
        .env("ORBIT_ID_ALLOCATOR_CHILD_OUTPUT", &output_a)
        .env("ORBIT_ID_ALLOCATOR_CHILD_COUNT", count.to_string())
        .spawn()
        .expect("spawn child a");
    let mut child_b = Command::new(&exe)
        .args([
            "--exact",
            "sqlite::id_allocator::tests::id_allocator::allocate_ids_child",
            "--nocapture",
        ])
        .env("ORBIT_ID_ALLOCATOR_CHILD_KIND", kind_name)
        .env("ORBIT_ID_ALLOCATOR_CHILD_ROOT", temp.path())
        .env("ORBIT_ID_ALLOCATOR_CHILD_OUTPUT", &output_b)
        .env("ORBIT_ID_ALLOCATOR_CHILD_COUNT", count.to_string())
        .spawn()
        .expect("spawn child b");

    assert!(child_a.wait().expect("wait a").success());
    assert!(child_b.wait().expect("wait b").success());

    let mut ids = read_id_file(&output_a);
    ids.extend(read_id_file(&output_b));
    let unique: BTreeSet<_> = ids.iter().cloned().collect();
    assert_eq!(unique.len(), count * 2, "ids collided: {ids:?}");
    let sequences: Vec<_> = unique
        .iter()
        .map(|id| match kind {
            IdAllocationKind::Adr => parse_adr_sequence(id).expect("adr seq"),
            IdAllocationKind::Learning => parse_learning_sequence(id).expect("learning seq"),
        })
        .collect();
    assert_eq!(sequences, (1..=(count as u32 * 2)).collect::<Vec<_>>());
}

fn allocator_config(root: &Path) -> IdAllocatorConfig {
    IdAllocatorConfig::new(
        root.join(".orbit/state/semantic.db"),
        root.join(".orbit/state/.id_alloc.lock"),
        root.join(".orbit"),
        root.to_path_buf(),
        root.join(".orbit/adrs"),
        root.join(".orbit/learnings"),
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

fn read_id_file(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .expect("read ids")
        .lines()
        .map(str::to_string)
        .filter(|line| !line.is_empty())
        .collect()
}
