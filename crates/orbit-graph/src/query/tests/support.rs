use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

use crate::{EXTRACTOR_VERSION, Graph, SyncPolicy, resolve_db_path};

pub(super) struct TestWorktree {
    path: PathBuf,
}

impl TestWorktree {
    pub(super) fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        path.push(format!(
            "orbit-graph-query-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test worktree");
        Self { path }
    }

    pub(super) fn path(&self) -> &Path {
        self.path.as_path()
    }

    pub(super) fn write(&self, rel: &str, content: &str) {
        let path = self.path.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        fs::write(path, content).expect("write source file");
    }
}

impl Drop for TestWorktree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(super) fn open_graph(worktree: &TestWorktree, policy: SyncPolicy) -> Graph {
    Graph::open(worktree.path(), policy).expect("open graph")
}

pub(super) fn open_connection(worktree: &TestWorktree) -> Connection {
    let conn = Connection::open(graph_db_path(worktree).as_path()).expect("open graph database");
    conn.pragma_update(None, "foreign_keys", "ON")
        .expect("enable foreign keys");
    conn
}

pub(super) fn graph_db_path(worktree: &TestWorktree) -> PathBuf {
    resolve_db_path(worktree.path(), "HEAD", EXTRACTOR_VERSION)
        .path()
        .to_path_buf()
}

pub(super) fn insert_file(conn: &Connection, path: &str, lang: &str, content: &str) {
    conn.execute(
        "INSERT INTO files (path, content_hash, mtime_ns, lang, byte_len, extracted_at)
         VALUES (?1, x'00', 1, ?2, ?3, 2)",
        params![
            path,
            lang,
            i64::try_from(content.len()).expect("content length fits")
        ],
    )
    .expect("insert file row");
}

pub(super) fn insert_symbol(
    conn: &Connection,
    file_path: &str,
    name: &str,
    qualified: &str,
    kind: &str,
    span_start: usize,
    span_end: usize,
) -> i64 {
    conn.execute(
        "INSERT INTO symbols (
            file_path, name, qualified, kind, span_start, span_end, signature, parent_symbol
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
        params![
            file_path,
            name,
            qualified,
            kind,
            i64::try_from(span_start).expect("span start fits"),
            i64::try_from(span_end).expect("span end fits"),
            format!("fn {name}()")
        ],
    )
    .expect("insert symbol row");
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO symbols_fts (rowid, name, qualified, signature)
         VALUES (?1, ?2, ?3, ?4)",
        params![id, name, qualified, format!("fn {name}()")],
    )
    .expect("insert symbol fts row");
    id
}
