use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

use crate::sync::{fail_next_sync_after_scan, sync_leader_count};
use crate::{CalleeEdge, EXTRACTOR_VERSION, Graph, GraphError, Selector, SyncPolicy, resolve_db_path};

#[test]
fn db_path_sanitizes_branch_slashes_and_preserves_raw_branch() {
    let worktree_root = Path::new("/tmp/orbit-worktree");

    let feat = resolve_db_path(worktree_root, "feat/foo", 1);
    assert_eq!(
        feat.path(),
        Path::new("/tmp/orbit-worktree/.orbit/graph/feat_foo.1.db")
    );
    assert_eq!(
        feat.path().file_name().and_then(|name| name.to_str()),
        Some("feat_foo.1.db")
    );
    assert_eq!(feat.branch(), "feat/foo");
    assert_eq!(feat.extractor_version(), 1);

    let main = resolve_db_path(worktree_root, "main", 42);
    assert_eq!(
        main.path(),
        Path::new("/tmp/orbit-worktree/.orbit/graph/main.42.db")
    );
    assert_eq!(
        main.path().file_name().and_then(|name| name.to_str()),
        Some("main.42.db")
    );
    assert_eq!(main.branch(), "main");
    assert_eq!(main.extractor_version(), 42);
}

#[test]
fn manual_policy_ensure_synced_is_noop() {
    let worktree = TestWorktree::new("manual-noop");
    worktree.write("src/lib.rs", "pub fn manual() {}\n");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let db_path = graph_db_path(worktree.path());

    graph.ensure_synced().expect("manual ensure");

    let conn = open_test_connection(worktree.path());
    assert_eq!(sync_leader_count(db_path.as_path()), 0);
    assert_eq!(row_count(&conn, "files"), 0);
    assert_eq!(meta_value(&conn, "last_incremental_at"), 0);
}

#[test]
fn on_read_policy_ensure_synced_syncs_on_every_call() {
    let worktree = TestWorktree::new("on-read");
    worktree.write("src/lib.rs", "pub fn on_read() {}\n");
    let graph = Graph::open(worktree.path(), SyncPolicy::OnRead).expect("open graph");
    let db_path = graph_db_path(worktree.path());

    graph.ensure_synced().expect("first on-read ensure");
    graph.ensure_synced().expect("second on-read ensure");

    let conn = open_test_connection(worktree.path());
    assert_eq!(sync_leader_count(db_path.as_path()), 2);
    assert_eq!(row_count(&conn, "files"), 1);
    assert!(meta_value(&conn, "last_incremental_at") > 0);
}

#[test]
fn windowed_policy_respects_recent_and_expired_sync_windows() {
    let worktree = TestWorktree::new("windowed");
    worktree.write("src/lib.rs", "pub fn windowed() {}\n");
    let graph = Graph::open(
        worktree.path(),
        SyncPolicy::Windowed {
            window: Duration::from_millis(500),
        },
    )
    .expect("open graph");
    let db_path = graph_db_path(worktree.path());

    graph.ensure_synced().expect("initial windowed ensure");
    graph.ensure_synced().expect("recent windowed ensure");
    assert_eq!(sync_leader_count(db_path.as_path()), 1);

    thread::sleep(Duration::from_millis(600));
    graph.ensure_synced().expect("expired windowed ensure");

    assert_eq!(sync_leader_count(db_path.as_path()), 2);
}

#[test]
fn windowed_policy_reads_last_incremental_at_from_db_each_check() {
    let worktree = TestWorktree::new("windowed-out-of-band");
    worktree.write("src/lib.rs", "pub fn stale_meta() {}\n");
    let graph = Graph::open(
        worktree.path(),
        SyncPolicy::Windowed {
            window: Duration::from_millis(500),
        },
    )
    .expect("open graph");
    let db_path = graph_db_path(worktree.path());

    graph.ensure_synced().expect("initial windowed ensure");
    set_meta_value(worktree.path(), "last_incremental_at", 1);
    graph
        .ensure_synced()
        .expect("windowed ensure after out-of-band metadata update");

    assert_eq!(sync_leader_count(db_path.as_path()), 2);
}

#[test]
fn windowed_policy_retries_after_sync_failure_without_advancing_timestamp() {
    let worktree = TestWorktree::new("windowed-retry");
    worktree.write("src/lib.rs", "pub fn retry_after_failure() {}\n");
    let graph = Graph::open(
        worktree.path(),
        SyncPolicy::Windowed {
            window: Duration::from_millis(500),
        },
    )
    .expect("open graph");
    let db_path = graph_db_path(worktree.path());

    fail_next_sync_after_scan(db_path.as_path());
    let result = graph.ensure_synced();

    assert!(matches!(
        result,
        Err(GraphError::InvalidData {
            operation: "run graph sync",
            ..
        })
    ));
    assert_eq!(sync_leader_count(db_path.as_path()), 1);
    assert_eq!(
        meta_value(
            &open_test_connection(worktree.path()),
            "last_incremental_at"
        ),
        0
    );

    graph.ensure_synced().expect("retry windowed ensure");

    assert_eq!(sync_leader_count(db_path.as_path()), 2);
    assert!(
        meta_value(
            &open_test_connection(worktree.path()),
            "last_incremental_at"
        ) > 0
    );
}

fn open_test_connection(worktree: &Path) -> Connection {
    let conn = Connection::open(graph_db_path(worktree)).expect("open graph database");
    conn.pragma_update(None, "foreign_keys", "ON")
        .expect("enable foreign keys");
    conn
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

fn meta_value(conn: &Connection, key: &str) -> i64 {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .expect("read meta value")
    .parse()
    .expect("meta value is integer")
}

fn set_meta_value(worktree: &Path, key: &str, value: i64) {
    open_test_connection(worktree)
        .execute(
            "UPDATE meta SET value = ?1 WHERE key = ?2",
            params![value.to_string(), key],
        )
        .expect("update meta value");
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
            "orbit-graph-policy-{name}-{}-{stamp}",
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

fn populate_callees_fixture(conn: &Connection, file_path: &str) {
            conn.execute(
                "INSERT INTO files (path, content_hash, mtime_ns, lang, byte_len, extracted_at)
                 VALUES (?1, x'00', 1, 'rust', 200, 2)",
                [file_path],
            )
            .expect("insert file");

            // Outer function span: 0..100
            conn.execute(
                "INSERT INTO symbols (id, file_path, name, qualified, kind, span_start, span_end, signature, parent_symbol)
                 VALUES (1, ?1, 'outer', 'crate::outer', 'function', 0, 100, 'fn outer()', NULL)",
                [file_path],
            )
            .expect("insert outer symbol");

            // Nested inner function span: 20..50 (contained in outer)
            conn.execute(
                "INSERT INTO symbols (id, file_path, name, qualified, kind, span_start, span_end, signature, parent_symbol)
                 VALUES (2, ?1, 'inner', 'crate::outer::inner', 'function', 20, 50, 'fn inner()', 1)",
                [file_path],
            )
            .expect("insert inner symbol");

            // 4 direct calls inside outer but outside inner (spans 10-15, 55-60, 70-75, 80-85)
            let calls = [
                (10, 15, "foo", Some("crate::foo"), "exact"),
                (55, 60, "bar", None, "fuzzy_name"),
                (70, 75, "baz", Some("crate::baz"), "same_module"),
                (80, 85, "quux", Some("other::quux"), "import_resolved"),
            ];
            for (i, (start, end, name, qual, conf)) in calls.iter().enumerate() {
                conn.execute(
                    "INSERT INTO refs (id, from_file, from_span_start, from_span_end, target_name, target_qualified, target_symbol_hint, kind, confidence)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 'call', ?7)",
                    params![i as i64 + 10, file_path, *start, *end, name, qual, conf],
                )
                .expect("insert direct call ref");
            }

            // 1 nested call inside inner (span 30-35, contained in both)
            conn.execute(
                "INSERT INTO refs (id, from_file, from_span_start, from_span_end, target_name, target_qualified, target_symbol_hint, kind, confidence)
                 VALUES (99, ?1, 30, 35, 'nested_call', 'crate::nested_call', NULL, 'call', 'exact')",
                [file_path],
            )
            .expect("insert nested call ref");
}

#[test]
fn query_callees_function_with_five_call_sites_returns_five_edges_including_nested_via_span_containment() {
    let worktree = TestWorktree::new("callees-five");
    let file_path = "src/example.rs";
    worktree.write(file_path, "fn outer() { /* calls */ fn inner(){} }\n");

    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let conn = open_test_connection(worktree.path());
    populate_callees_fixture(&conn, file_path);

    let sel = Selector::Symbol {
        path: file_path.to_string(),
        symbol: "outer".to_string(),
        kind: "function".to_string(),
    };
    let edges: Vec<CalleeEdge> = graph.callees(&sel).expect("callees query");

    assert_eq!(edges.len(), 5, "expected 5 callees (4 direct + 1 nested via containment)");

    let names: Vec<_> = edges.iter().map(|e| e.target_name.as_str()).collect();
    assert!(names.contains(&"foo"));
    assert!(names.contains(&"nested_call")); // attributed to outer via span containment
    let nested = edges.iter().find(|e| e.target_name == "nested_call").unwrap();
    assert_eq!(nested.confidence, "exact");
    assert_eq!(nested.from_span, 30);
}

#[test]
fn query_callees_unknown_symbol_selector_returns_empty_vec_not_error() {
    let worktree = TestWorktree::new("callees-miss");
    worktree.write("src/lib.rs", "pub fn present() {}\n");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");

    let sel = Selector::Symbol {
        path: "src/lib.rs".to_string(),
        symbol: "absent".to_string(),
        kind: "function".to_string(),
    };
    let edges: Vec<CalleeEdge> = graph.callees(&sel).expect("callees on missing symbol");
    assert!(edges.is_empty());
}

#[test]
fn query_callees_non_symbol_selector_returns_empty_vec() {
    let worktree = TestWorktree::new("callees-non-sym");
    worktree.write("src/lib.rs", "// empty\n");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");

    let file_sel = Selector::File { path: "src/lib.rs".to_string() };
    let edges: Vec<CalleeEdge> = graph.callees(&file_sel).expect("callees on file sel");
    assert!(edges.is_empty());

    let dir_sel = Selector::Dir { path: "src".to_string() };
    let edges2: Vec<CalleeEdge> = graph.callees(&dir_sel).expect("callees on dir sel");
    assert!(edges2.is_empty());
}
