use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use orbit_common::types::{OrbitError, StoredTool};
use rusqlite::TransactionBehavior;

use crate::Store;

fn tool(name: &str) -> StoredTool {
    StoredTool {
        name: name.to_string(),
        path: format!("/tools/{name}"),
        description: String::new(),
        parameters: Vec::new(),
        enabled: true,
        builtin: false,
    }
}

fn file_backed_store(dir: &tempfile::TempDir) -> Store {
    Store::open(&dir.path().join("store.db")).expect("open store")
}

#[test]
fn pooled_reader_is_query_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = file_backed_store(&dir);

    let conn = store.read().expect("read conn");
    let result = conn.execute("INSERT INTO tools(name, path) VALUES ('x', '/x')", []);
    assert!(
        result.is_err(),
        "write through a pooled reader must fail (query_only=ON)"
    );
}

#[test]
fn readers_are_returned_to_the_pool_and_reused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = file_backed_store(&dir);

    // Guards dropped one at a time: each checkout after the first drop must
    // reuse the idle connection instead of growing the pool.
    drop(store.read().expect("first read"));
    drop(store.read().expect("second read"));
    drop(store.read().expect("third read"));

    let pool = store.reader_pool_for_test().expect("file store has pool");
    assert_eq!(pool.idle_len(), 1, "sequential reads reuse one connection");

    let a = store.read().expect("read a");
    let b = store.read().expect("read b");
    assert_eq!(pool.idle_len(), 0);
    drop(a);
    drop(b);
    assert_eq!(pool.idle_len(), 2, "both readers checked back in");
}

#[test]
fn reads_see_writes_committed_by_the_writer_connection() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = file_backed_store(&dir);

    store
        .with_transaction(|tx| tx.insert_tool(&tool("hammer")))
        .expect("insert");

    let tools = store.list_tools().expect("list");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "hammer");
}

#[test]
fn in_memory_store_reads_fall_back_to_writer_connection() {
    let store = Store::open_in_memory().expect("open in-memory");
    store
        .with_transaction(|tx| tx.insert_tool(&tool("wrench")))
        .expect("insert");

    let tools = store.list_tools().expect("list");
    assert_eq!(tools.len(), 1);
}

/// P1.3 core guarantee: a read completes while the writer holds an open
/// IMMEDIATE transaction. With the old single-mutex Store this deadlocks
/// (the read would queue behind the writer until the transaction ends);
/// with the read pool the read proceeds on its own WAL snapshot.
#[test]
fn reads_do_not_queue_behind_an_open_writer_transaction() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = file_backed_store(&dir);
    store
        .with_transaction(|tx| tx.insert_tool(&tool("seed")))
        .expect("seed insert");

    let (tx_open_send, tx_open_recv) = mpsc::channel::<()>();
    let (read_done_send, read_done_recv) = mpsc::channel::<usize>();

    let writer_store = store.clone();
    let writer = thread::spawn(move || {
        writer_store.with_transaction_behavior(TransactionBehavior::Immediate, |tx| {
            tx.insert_tool(&tool("in-flight"))?;
            tx_open_send
                .send(())
                .map_err(|e| OrbitError::Store(e.to_string()))?;
            // Hold the write transaction open until the reader has finished
            // (or prove the reader deadlocked by timing out).
            read_done_recv
                .recv_timeout(Duration::from_secs(10))
                .map_err(|_| {
                    OrbitError::Store("reader did not complete while writer tx open".to_string())
                })?;
            Ok(())
        })
    });

    tx_open_recv
        .recv_timeout(Duration::from_secs(10))
        .expect("writer opened transaction");

    // Read while the write transaction is open: must not block or error,
    // and must see the pre-transaction snapshot.
    let tools = store.list_tools().expect("concurrent read");
    read_done_send.send(tools.len()).expect("send read result");

    writer
        .join()
        .expect("writer thread")
        .expect("writer transaction commits");
    assert_eq!(
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        vec!["seed"],
        "reader sees committed state only, not the open transaction"
    );
}

/// Concurrency smoke test: parallel readers while a writer commits — no
/// `database is locked` errors surface with WAL + pooled readers.
#[test]
fn concurrent_readers_and_writer_produce_no_locked_errors() {
    const READER_THREADS: usize = 4;
    const READS_PER_THREAD: usize = 50;
    const WRITES: usize = 50;

    let dir = tempfile::tempdir().expect("tempdir");
    let store = file_backed_store(&dir);

    let mut readers = Vec::new();
    for _ in 0..READER_THREADS {
        let store = store.clone();
        readers.push(thread::spawn(move || -> Result<(), OrbitError> {
            for _ in 0..READS_PER_THREAD {
                store.list_tools()?;
                store.schema_version()?;
            }
            Ok(())
        }));
    }

    for i in 0..WRITES {
        store
            .with_transaction(|tx| tx.insert_tool(&tool(&format!("tool-{i}"))))
            .expect("concurrent write");
    }

    for reader in readers {
        reader
            .join()
            .expect("reader thread")
            .expect("no read errors under concurrent writes");
    }

    assert_eq!(store.list_tools().expect("final list").len(), WRITES);
}

/// Heavier contention benchmark; run manually with `--ignored`.
#[test]
#[ignore = "contention benchmark; slow under CI"]
fn stress_many_readers_under_sustained_writes() {
    const READER_THREADS: usize = 8;
    const READS_PER_THREAD: usize = 500;
    const WRITES: usize = 500;

    let dir = tempfile::tempdir().expect("tempdir");
    let store = file_backed_store(&dir);

    let mut readers = Vec::new();
    for _ in 0..READER_THREADS {
        let store = store.clone();
        readers.push(thread::spawn(move || -> Result<(), OrbitError> {
            for _ in 0..READS_PER_THREAD {
                store.list_tools()?;
            }
            Ok(())
        }));
    }

    for i in 0..WRITES {
        store
            .with_transaction(|tx| tx.insert_tool(&tool(&format!("stress-{i}"))))
            .expect("stress write");
    }

    for reader in readers {
        reader
            .join()
            .expect("reader thread")
            .expect("no locked errors under stress");
    }
}
