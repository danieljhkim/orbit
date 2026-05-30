use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use orbit_graph_extract::RawRef;
use rusqlite::{Connection, params};

use crate::sync::pass1::ExtractedFileRefs;
use crate::{EXTRACTOR_VERSION, Graph, SyncMode, SyncPolicy, resolve_db_path};

#[test]
fn exact_resolution_prefers_unambiguous_same_file_symbol() {
    let worktree = TestWorktree::new("exact");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    worktree.write(
        "src/lib.rs",
        r#"
fn helper() {}

fn caller() {
    helper();
}
"#,
    );

    graph.sync(SyncMode::Full).expect("sync graph");

    let conn = open_test_connection(worktree.path());
    let row = call_ref(&conn, "src/lib.rs", "helper");
    assert_eq!(row.target_qualified.as_deref(), Some("helper"));
    assert_eq!(row.confidence, super::CONFIDENCE_EXACT);
    assert!(row.target_symbol_hint.is_some());
}

#[test]
fn import_resolved_resolution_uses_explicit_import() {
    let worktree = TestWorktree::new("import");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    worktree.write(
        "src/caller.rs",
        r#"
use imported::target;

mod caller {
    fn run() {
        target();
    }
}
"#,
    );
    // L-0050: file paths do not currently contribute Rust module-qualified symbol names.
    worktree.write(
        "src/imported.rs",
        r#"
mod imported {
    pub fn target() {}
}
"#,
    );

    graph.sync(SyncMode::Full).expect("sync graph");

    let conn = open_test_connection(worktree.path());
    let row = call_ref(&conn, "src/caller.rs", "target");
    assert_eq!(row.target_qualified.as_deref(), Some("imported::target"));
    assert_eq!(row.confidence, super::CONFIDENCE_IMPORT_RESOLVED);
    assert!(row.target_symbol_hint.is_some());
}

#[test]
fn same_module_resolution_uses_unique_cross_file_module_match() {
    let worktree = TestWorktree::new("same-module");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    worktree.write(
        "src/caller.rs",
        r#"
mod shared {
    fn run() {
        target();
    }
}
"#,
    );
    worktree.write(
        "src/target.rs",
        r#"
mod shared {
    pub fn target() {}
}
"#,
    );

    graph.sync(SyncMode::Full).expect("sync graph");

    let conn = open_test_connection(worktree.path());
    let row = call_ref(&conn, "src/caller.rs", "target");
    assert_eq!(row.target_qualified.as_deref(), Some("shared::target"));
    assert_eq!(row.confidence, super::CONFIDENCE_SAME_MODULE);
    assert!(row.target_symbol_hint.is_some());
}

#[test]
fn fuzzy_name_resolution_leaves_target_qualified_and_hint_null() {
    let worktree = TestWorktree::new("fuzzy");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    worktree.write(
        "src/caller.rs",
        r#"
fn run() {
    duplicate();
}
"#,
    );
    worktree.write("src/left.rs", "pub fn duplicate() {}\n");
    worktree.write("src/right.rs", "pub fn duplicate() {}\n");

    graph.sync(SyncMode::Full).expect("sync graph");

    let conn = open_test_connection(worktree.path());
    let row = call_ref(&conn, "src/caller.rs", "duplicate");
    assert!(row.target_qualified.is_none());
    assert!(row.target_symbol_hint.is_none());
    assert_eq!(row.confidence, super::CONFIDENCE_FUZZY_NAME);
}

#[test]
fn import_resolved_outranks_same_module_candidate() {
    let worktree = TestWorktree::new("import-outranks-same-module");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    worktree.write(
        "src/caller.rs",
        r#"
use chosen::run;
use shared::*;

mod shared {
    fn caller() {
        run();
    }
}
"#,
    );
    worktree.write(
        "src/same_module.rs",
        r#"
mod shared {
    pub fn run() {}
}
"#,
    );
    worktree.write(
        "src/imported.rs",
        r#"
mod chosen {
    pub fn run() {}
}
"#,
    );

    graph.sync(SyncMode::Full).expect("sync graph");

    let conn = open_test_connection(worktree.path());
    let row = call_ref(&conn, "src/caller.rs", "run");
    assert_eq!(row.target_qualified.as_deref(), Some("chosen::run"));
    assert_eq!(row.confidence, super::CONFIDENCE_IMPORT_RESOLVED);
}

#[test]
fn fuzzy_refs_are_null_and_non_fuzzy_refs_are_populated_in_three_file_sync() {
    let worktree = TestWorktree::new("three-file-tiers");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    worktree.write(
        "src/caller.rs",
        r#"
use imported::imported_call;

fn local_call() {}

mod shared {
    fn caller() {
        local_call();
        imported_call();
        module_call();
        ambiguous_call();
    }
}
"#,
    );
    worktree.write(
        "src/defs_one.rs",
        r#"
mod imported {
    pub fn imported_call() {}
}

mod shared {
    pub fn module_call() {}
    pub fn ambiguous_call() {}
}
"#,
    );
    worktree.write(
        "src/defs_two.rs",
        r#"
mod shared {
    pub fn ambiguous_call() {}
}
"#,
    );

    graph.sync(SyncMode::Full).expect("sync graph");

    let conn = open_test_connection(worktree.path());
    let rows = call_refs_by_name(&conn, "src/caller.rs");
    assert_ref(
        rows.get("local_call").expect("local call ref"),
        Some("local_call"),
        super::CONFIDENCE_EXACT,
    );
    assert_ref(
        rows.get("imported_call").expect("imported call ref"),
        Some("imported::imported_call"),
        super::CONFIDENCE_IMPORT_RESOLVED,
    );
    assert_ref(
        rows.get("module_call").expect("same module call ref"),
        Some("shared::module_call"),
        super::CONFIDENCE_SAME_MODULE,
    );
    assert_ref(
        rows.get("ambiguous_call").expect("ambiguous call ref"),
        None,
        super::CONFIDENCE_FUZZY_NAME,
    );
}

#[test]
fn target_symbol_hint_is_null_when_resolved_qualified_is_ambiguous() {
    let worktree = TestWorktree::new("ambiguous-hint");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    drop(graph);
    let conn = open_test_connection(worktree.path());
    insert_file(&conn, "src/caller.rs");
    insert_file(&conn, "src/left.rs");
    insert_file(&conn, "src/right.rs");
    insert_symbol(&conn, "src/left.rs", "target", "dupe::target");
    insert_symbol(&conn, "src/right.rs", "target", "dupe::target");
    insert_import(&conn, "src/caller.rs", "dupe", Some("target"));
    drop(conn);

    super::run(
        graph_db_path(worktree.path()).as_path(),
        SyncMode::Full,
        vec![ExtractedFileRefs {
            file_path: "src/caller.rs".to_string(),
            refs: vec![raw_ref("src/caller.rs", "target")],
        }],
    )
    .expect("run pass2");

    let conn = open_test_connection(worktree.path());
    let row = call_ref(&conn, "src/caller.rs", "target");
    assert_eq!(row.target_qualified.as_deref(), Some("dupe::target"));
    assert_eq!(row.confidence, super::CONFIDENCE_IMPORT_RESOLVED);
    assert!(row.target_symbol_hint.is_none());
}

#[test]
fn incremental_sync_rewrites_only_changed_file_refs() {
    let worktree = TestWorktree::new("incremental-preserve");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    worktree.write(
        "src/a.rs",
        r#"
fn stable() {}

fn caller() {
    stable();
}
"#,
    );
    worktree.write(
        "src/b.rs",
        r#"
fn before() {}

fn caller() {
    before();
}
"#,
    );
    graph.sync(SyncMode::Full).expect("initial sync");

    let conn = open_test_connection(worktree.path());
    let before = refs_for_file(&conn, "src/a.rs");
    drop(conn);

    std::thread::sleep(Duration::from_millis(5));
    worktree.write(
        "src/b.rs",
        r#"
fn after() {}

fn caller() {
    after();
}
"#,
    );
    graph.sync(SyncMode::Auto).expect("incremental sync");

    let conn = open_test_connection(worktree.path());
    assert_eq!(refs_for_file(&conn, "src/a.rs"), before);
}

#[test]
fn pass2_failure_rolls_back_ref_rewrites_and_meta_update() {
    let worktree = TestWorktree::new("rollback");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    drop(graph);
    let conn = open_test_connection(worktree.path());
    insert_file(&conn, "src/a.rs");
    insert_ref_row(&conn, "src/a.rs", "old", Some("old"), 1, "exact");
    drop(conn);

    let result = super::run(
        graph_db_path(worktree.path()).as_path(),
        SyncMode::Full,
        vec![
            ExtractedFileRefs {
                file_path: "src/a.rs".to_string(),
                refs: vec![raw_ref("src/a.rs", "new")],
            },
            ExtractedFileRefs {
                file_path: "src/missing.rs".to_string(),
                refs: vec![raw_ref("src/missing.rs", "missing")],
            },
        ],
    );

    assert!(result.is_err());
    let conn = open_test_connection(worktree.path());
    let refs = refs_for_file(&conn, "src/a.rs");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].target_name, "old");
    assert_eq!(meta_value(&conn, "last_full_build_at"), 0);
}

fn assert_ref(row: &StoredRef, target_qualified: Option<&str>, confidence: &str) {
    assert_eq!(row.target_qualified.as_deref(), target_qualified);
    assert_eq!(row.confidence, confidence);
    if confidence == super::CONFIDENCE_FUZZY_NAME {
        assert!(row.target_symbol_hint.is_none());
    } else {
        assert!(row.target_qualified.is_some());
    }
}

fn raw_ref(from_file: &str, target_name: &str) -> RawRef {
    RawRef {
        from_file: from_file.to_string(),
        from_span_start: 0,
        from_span_end: target_name.len(),
        target_name: target_name.to_string(),
        target_qualified: None,
        kind: "call".to_string(),
        confidence: super::CONFIDENCE_FUZZY_NAME.to_string(),
    }
}

fn insert_file(conn: &Connection, rel: &str) {
    conn.execute(
        "INSERT INTO files (path, content_hash, mtime_ns, lang, byte_len, extracted_at)
         VALUES (?1, x'00', 1, 'rust', 12, 2)",
        params![rel],
    )
    .expect("insert file");
}

fn insert_symbol(conn: &Connection, rel: &str, name: &str, qualified: &str) {
    conn.execute(
        "INSERT INTO symbols (
            file_path, name, qualified, kind, span_start, span_end, signature, parent_symbol
         ) VALUES (?1, ?2, ?3, 'function', 0, 1, NULL, NULL)",
        params![rel, name, qualified],
    )
    .expect("insert symbol");
}

fn insert_import(
    conn: &Connection,
    from_file: &str,
    target_path: &str,
    target_symbol: Option<&str>,
) {
    conn.execute(
        "INSERT INTO imports (from_file, target_path, target_symbol)
         VALUES (?1, ?2, ?3)",
        params![from_file, target_path, target_symbol],
    )
    .expect("insert import");
}

fn insert_ref_row(
    conn: &Connection,
    from_file: &str,
    target_name: &str,
    target_qualified: Option<&str>,
    target_symbol_hint: i64,
    confidence: &str,
) {
    conn.execute(
        "INSERT INTO refs (
            from_file, from_span_start, from_span_end, target_name, target_qualified,
            target_symbol_hint, kind, confidence
         ) VALUES (?1, 0, 1, ?2, ?3, ?4, 'call', ?5)",
        params![
            from_file,
            target_name,
            target_qualified,
            target_symbol_hint,
            confidence
        ],
    )
    .expect("insert ref");
}

fn call_ref(conn: &Connection, from_file: &str, target_name: &str) -> StoredRef {
    let mut rows = query_refs(
        conn,
        "SELECT id, from_file, from_span_start, from_span_end, target_name, target_qualified,
                target_symbol_hint, kind, confidence
         FROM refs
         WHERE from_file = ?1 AND target_name = ?2 AND kind = 'call'
         ORDER BY id",
        params![from_file, target_name],
    );
    assert_eq!(rows.len(), 1, "expected one call ref for {target_name}");
    rows.remove(0)
}

fn call_refs_by_name(conn: &Connection, from_file: &str) -> BTreeMap<String, StoredRef> {
    query_refs(
        conn,
        "SELECT id, from_file, from_span_start, from_span_end, target_name, target_qualified,
                target_symbol_hint, kind, confidence
         FROM refs
         WHERE from_file = ?1 AND kind = 'call'
         ORDER BY target_name, id",
        params![from_file],
    )
    .into_iter()
    .map(|row| (row.target_name.clone(), row))
    .collect()
}

fn refs_for_file(conn: &Connection, from_file: &str) -> Vec<StoredRef> {
    query_refs(
        conn,
        "SELECT id, from_file, from_span_start, from_span_end, target_name, target_qualified,
                target_symbol_hint, kind, confidence
         FROM refs
         WHERE from_file = ?1
         ORDER BY id",
        params![from_file],
    )
}

fn query_refs<P>(conn: &Connection, sql: &str, params: P) -> Vec<StoredRef>
where
    P: rusqlite::Params,
{
    conn.prepare(sql)
        .expect("prepare refs query")
        .query_map(params, |row| {
            Ok(StoredRef {
                id: row.get(0)?,
                from_file: row.get(1)?,
                from_span_start: row.get(2)?,
                from_span_end: row.get(3)?,
                target_name: row.get(4)?,
                target_qualified: row.get(5)?,
                target_symbol_hint: row.get(6)?,
                kind: row.get(7)?,
                confidence: row.get(8)?,
            })
        })
        .expect("query refs")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect refs")
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

fn meta_value(conn: &Connection, key: &str) -> i64 {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .expect("read meta value")
    .parse()
    .expect("meta value is integer")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredRef {
    id: i64,
    from_file: String,
    from_span_start: i64,
    from_span_end: i64,
    target_name: String,
    target_qualified: Option<String>,
    target_symbol_hint: Option<i64>,
    kind: String,
    confidence: String,
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
            "orbit-graph-pass2-{name}-{}-{stamp}",
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
