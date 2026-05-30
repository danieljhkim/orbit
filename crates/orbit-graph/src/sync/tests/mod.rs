#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;

use crate::sync::{SyncLeaderGate, set_sync_after_scan_gate};
use crate::{EXTRACTOR_VERSION, Graph, SyncMode, SyncPolicy, resolve_db_path};

#[cfg(unix)]
#[test]
fn concurrent_syncs_hold_flock_across_scan_and_writes() {
    let worktree = TestWorktree::new("whole-sync-flock");
    worktree.write("src/lib.rs", "pub fn original() {}\n");
    let first_graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open first graph");
    first_graph
        .sync(SyncMode::Full)
        .expect("seed initial graph rows");
    let db_path = first_graph.db_path().path().to_path_buf();

    // L-0055: the symlink path bypasses in-process coalescing while sharing the same lock file.
    let link = symlink_to(worktree.path(), "whole-sync-flock-link");
    let second_graph = Graph::open(link.as_path(), SyncPolicy::Manual).expect("open second graph");

    fs::remove_file(worktree.path().join("src/lib.rs")).expect("remove indexed file");
    let gate = Arc::new(SyncLeaderGate::new());
    set_sync_after_scan_gate(db_path.clone(), Some(Arc::clone(&gate)));

    let first = thread::spawn(move || first_graph.sync(SyncMode::Auto));
    assert!(gate.wait_started(Duration::from_secs(2)));

    worktree.write("src/lib.rs", "pub fn recreated() {}\n");
    let second = thread::spawn(move || second_graph.sync(SyncMode::Auto));
    let _second_finished_before_release = wait_until_finished(&second, Duration::from_secs(1));
    gate.release();

    first
        .join()
        .expect("join first sync")
        .expect("first sync succeeds");
    second
        .join()
        .expect("join second sync")
        .expect("second sync succeeds");
    set_sync_after_scan_gate(db_path, None);
    fs::remove_file(link).expect("remove symlink");

    let conn = open_test_connection(worktree.path());
    assert_eq!(row_count(&conn, "files"), 1);
    assert_eq!(duplicate_file_row_groups(&conn), 0);
}

#[cfg(unix)]
fn symlink_to(target: &Path, name: &str) -> PathBuf {
    let mut link = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos();
    link.push(format!("orbit-graph-{name}-{}-{stamp}", std::process::id()));
    std::os::unix::fs::symlink(target, link.as_path()).expect("create symlink");
    link
}

fn wait_until_finished<T>(handle: &thread::JoinHandle<T>, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if handle.is_finished() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    handle.is_finished()
}

fn open_test_connection(worktree: &Path) -> Connection {
    Connection::open(graph_db_path(worktree)).expect("open graph database")
}

fn graph_db_path(worktree: &Path) -> PathBuf {
    resolve_db_path(worktree, "HEAD", EXTRACTOR_VERSION)
        .path()
        .to_path_buf()
}

fn row_count(conn: &Connection, table: &str) -> i64 {
    let sql = format!("SELECT count(*) FROM {table}");
    conn.query_row(&sql, [], |row| row.get(0))
        .expect("count rows")
}

fn duplicate_file_row_groups(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM (
            SELECT path FROM files GROUP BY path HAVING count(*) > 1
         )",
        [],
        |row| row.get(0),
    )
    .expect("count duplicate file row groups")
}

struct TestWorktree {
    path: PathBuf,
}

impl TestWorktree {
    fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        path.push(format!(
            "orbit-graph-sync-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test worktree");
        Self { path }
    }

    fn path(&self) -> &Path {
        self.path.as_path()
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.path.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write file");
    }
}

impl Drop for TestWorktree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
