use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

use super::{DefaultExtractorBackend, ExtractFileError, ExtractedSourceFile, ExtractorBackend};
use crate::sync::scanner::Diff;
use crate::{EXTRACTOR_VERSION, Graph, SyncMode, SyncPolicy, resolve_db_path};

#[test]
fn panicking_extractor_skips_file_and_preserves_other_writes() {
    let worktree = TestWorktree::new("panic-skip");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    drop(graph);
    worktree.write("src/good.rs", "pub fn good() {}\n");
    worktree.write("src/bad.rs", "pub fn bad() {}\n");
    let diff = Diff {
        new: vec![PathBuf::from("src/good.rs"), PathBuf::from("src/bad.rs")],
        ..Diff::default()
    };

    let output = super::run_with_backend(
        graph_db_path(worktree.path()).as_path(),
        worktree.path(),
        SyncMode::Auto,
        &diff,
        &PanickingBackend,
    )
    .expect("pass1 skips panicking file");

    let conn = open_test_connection(worktree.path());
    assert_eq!(output.files_written, 1);
    assert_eq!(file_count(&conn, "src/good.rs"), 1);
    assert_eq!(file_count(&conn, "src/bad.rs"), 0);
    assert_eq!(row_count(&conn, "symbols"), 1);
}

#[test]
fn modified_file_replaces_prior_symbol_rows() {
    let worktree = TestWorktree::new("modified-replace");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    worktree.write("src/lib.rs", "pub fn only_now() {}\n");
    {
        let conn = open_test_connection(worktree.path());
        insert_file_with_three_symbols(&conn, "src/lib.rs");
    }

    graph.sync(SyncMode::Auto).expect("sync modified file");

    let conn = open_test_connection(worktree.path());
    assert_eq!(
        row_count_for_file(&conn, "symbols", "file_path", "src/lib.rs"),
        1
    );
    assert_eq!(file_count(&conn, "src/lib.rs"), 1);
}

#[test]
fn deleted_file_row_cascades_all_pass1_tables() {
    let worktree = TestWorktree::new("deleted-cascade");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    {
        let conn = open_test_connection(worktree.path());
        insert_file_anchored_rows(&conn, "src/deleted.rs");
    }

    graph.sync(SyncMode::Auto).expect("sync deleted file");

    let conn = open_test_connection(worktree.path());
    for table in [
        "files",
        "symbols",
        "refs",
        "relations",
        "imports",
        "commands",
        "strings",
        "configs",
    ] {
        assert_eq!(row_count(&conn, table), 0, "{table} rows should be gone");
    }
}

#[test]
fn full_sync_rebuilds_cross_file_command_handlers() {
    let worktree = TestWorktree::new("cross-file-command-handler");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    worktree.write(
        "src/add.rs",
        r#"
struct TaskAddArgs;

trait Execute {
    fn execute(self);
}

impl Execute for TaskAddArgs {
    fn execute(self) {
        helper();
    }
}

fn helper() {}
"#,
    );
    worktree.write(
        "src/command.rs",
        r#"
use clap::Subcommand;

#[derive(Subcommand)]
enum TaskSubcommand {
    Add(TaskAddArgs),
}

fn dispatch(command: TaskSubcommand) {
    match command {
        TaskSubcommand::Add(args) => args.execute(),
    }
}
"#,
    );

    graph.sync(SyncMode::Full).expect("initial full sync");
    graph
        .sync(SyncMode::Full)
        .expect("second full sync keeps cross-file command handlers valid");

    let conn = open_test_connection(worktree.path());
    let handler = conn
        .query_row(
            "SELECT s.qualified
             FROM commands c
             JOIN symbols s ON s.id = c.handler_symbol
             WHERE c.name = 'task add'",
            [],
            |row| row.get::<_, String>(0),
        )
        .expect("resolved task add handler");
    assert_eq!(handler, "<TaskAddArgs as Execute>::execute");
}

#[test]
fn pass1_writes_relations_but_not_refs() {
    let worktree = TestWorktree::new("relations-no-refs");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    drop(graph);
    worktree.write(
        "src/lib.rs",
        r#"
trait Service {
    fn run(&self);
}

struct Worker;

impl Service for Worker {
    fn run(&self) {
        helper();
    }
}

fn helper() {}
"#,
    );
    let diff = Diff {
        new: vec![PathBuf::from("src/lib.rs")],
        ..Diff::default()
    };

    super::run(
        graph_db_path(worktree.path()).as_path(),
        worktree.path(),
        SyncMode::Full,
        &diff,
    )
    .expect("run pass1");

    let conn = open_test_connection(worktree.path());
    assert_eq!(row_count(&conn, "relations"), 1);
    assert_eq!(row_count(&conn, "refs"), 0);
}

#[test]
fn pass1_returns_extracted_refs_for_pass2_handoff() {
    let worktree = TestWorktree::new("refs-handoff");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    drop(graph);
    worktree.write(
        "src/lib.rs",
        r#"
fn helper() {}

fn main() {
    helper();
}
"#,
    );
    let diff = Diff {
        new: vec![PathBuf::from("src/lib.rs")],
        ..Diff::default()
    };

    let output = super::run(
        graph_db_path(worktree.path()).as_path(),
        worktree.path(),
        SyncMode::Full,
        &diff,
    )
    .expect("run pass1");

    assert!(output.refs.iter().any(|file_refs| {
        file_refs.file_path == "src/lib.rs"
            && file_refs
                .refs
                .iter()
                .any(|raw_ref| raw_ref.target_name == "helper")
    }));
    assert_eq!(row_count(&open_test_connection(worktree.path()), "refs"), 0);
}

#[test]
fn sync_meta_timestamps_track_full_and_auto_modes() {
    let worktree = TestWorktree::new("meta-timestamps");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    worktree.write("src/lib.rs", "pub fn full_build() {}\n");

    graph.sync(SyncMode::Full).expect("full sync");

    let conn = open_test_connection(worktree.path());
    let full_after_full = meta_value(&conn, "last_full_build_at");
    let incremental_after_full = meta_value(&conn, "last_incremental_at");
    assert!(full_after_full > 0);
    assert_eq!(incremental_after_full, 0);
    drop(conn);

    worktree.write("src/next.rs", "pub fn incremental() {}\n");
    graph.sync(SyncMode::Auto).expect("auto sync");

    let conn = open_test_connection(worktree.path());
    assert!(meta_value(&conn, "last_full_build_at") >= full_after_full);
    assert!(meta_value(&conn, "last_incremental_at") > 0);
}

#[test]
fn cold_sync_100_rust_file_performance_smoke_prints_elapsed_ms() {
    let worktree = TestWorktree::new("perf-smoke");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    for index in 0..100 {
        worktree.write(
            &format!("src/file_{index}.rs"),
            &format!("pub fn marker_{index}() {{}}\n"),
        );
    }

    let started = Instant::now();
    let report = graph.sync(SyncMode::Full).expect("cold sync");
    let elapsed = started.elapsed();

    #[allow(clippy::print_stdout)]
    {
        println!("pass1_cold_sync_100_rust_files_ms={}", elapsed.as_millis());
    }
    assert_eq!(report.files_indexed, 100);
    assert_eq!(
        row_count(&open_test_connection(worktree.path()), "files"),
        100
    );
}

struct PanickingBackend;

impl ExtractorBackend for PanickingBackend {
    fn extract(
        &self,
        worktree_root: &Path,
        rel_path: &Path,
    ) -> Result<ExtractedSourceFile, ExtractFileError> {
        if rel_path == Path::new("src/bad.rs") {
            panic!("intentional extractor panic");
        }
        DefaultExtractorBackend.extract(worktree_root, rel_path)
    }
}

fn insert_file_with_three_symbols(conn: &Connection, rel: &str) {
    conn.execute(
        "INSERT INTO files (path, content_hash, mtime_ns, lang, byte_len, extracted_at)
         VALUES (?1, x'00', 1, 'rust', 12, 2)",
        params![rel],
    )
    .expect("insert file");
    for index in 0..3 {
        conn.execute(
            "INSERT INTO symbols (
                file_path, name, qualified, kind, span_start, span_end, signature, parent_symbol
             ) VALUES (?1, ?2, ?3, 'function', ?4, ?5, NULL, NULL)",
            params![
                rel,
                format!("old_{index}"),
                format!("crate::old_{index}"),
                index,
                index + 1
            ],
        )
        .expect("insert old symbol");
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO symbols_fts (rowid, name, qualified, signature)
             VALUES (?1, ?2, ?3, NULL)",
            params![id, format!("old_{index}"), format!("crate::old_{index}")],
        )
        .expect("insert old symbol fts");
    }
}

fn insert_file_anchored_rows(conn: &Connection, rel: &str) {
    conn.execute(
        "INSERT INTO files (path, content_hash, mtime_ns, lang, byte_len, extracted_at)
         VALUES (?1, x'00', 1, 'rust', 12, 2)",
        params![rel],
    )
    .expect("insert file");
    conn.execute(
        "INSERT INTO symbols (
            id, file_path, name, qualified, kind, span_start, span_end, signature, parent_symbol
         ) VALUES (1, ?1, 'run', 'crate::run', 'function', 0, 3, 'fn run()', NULL)",
        params![rel],
    )
    .expect("insert symbol");
    conn.execute(
        "INSERT INTO symbols_fts (rowid, name, qualified, signature)
         VALUES (1, 'run', 'crate::run', 'fn run()')",
        [],
    )
    .expect("insert symbol fts");
    conn.execute(
        "INSERT INTO refs (
            from_file, from_span_start, from_span_end, target_name, target_qualified,
            target_symbol_hint, kind, confidence
         ) VALUES (?1, 4, 7, 'run', 'crate::run', 1, 'call', 'exact')",
        params![rel],
    )
    .expect("insert ref");
    conn.execute(
        "INSERT INTO relations (
            from_qualified, to_qualified, kind, def_file, def_span_start, def_span_end, confidence
         ) VALUES ('crate::Type', 'crate::Trait', 'impl', ?1, 0, 10, 'exact')",
        params![rel],
    )
    .expect("insert relation");
    conn.execute(
        "INSERT INTO imports (from_file, target_path, target_symbol)
         VALUES (?1, 'crate::other', 'Other')",
        params![rel],
    )
    .expect("insert import");
    conn.execute(
        "INSERT INTO commands (name, file_path, span_start, handler_symbol)
         VALUES ('run', ?1, 0, 1)",
        params![rel],
    )
    .expect("insert command");
    conn.execute(
        "INSERT INTO strings (file_path, line, value, context_symbol)
         VALUES (?1, 1, 'hello world', 1)",
        params![rel],
    )
    .expect("insert string");
    let string_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO strings_fts (rowid, value) VALUES (?1, 'hello world')",
        params![string_id],
    )
    .expect("insert string fts");
    conn.execute(
        "INSERT INTO configs (file_path, line, key, kind)
         VALUES (?1, 1, 'app.name', 'toml')",
        params![rel],
    )
    .expect("insert config");
    let config_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO configs_fts (rowid, key) VALUES (?1, 'app.name')",
        params![config_id],
    )
    .expect("insert config fts");
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

fn row_count_for_file(conn: &Connection, table: &str, column: &str, rel: &str) -> i64 {
    let sql = format!("SELECT count(*) FROM {table} WHERE {column} = ?1");
    conn.query_row(&sql, [rel], |row| row.get(0))
        .expect("count rows for file")
}

fn file_count(conn: &Connection, rel: &str) -> i64 {
    row_count_for_file(conn, "files", "path", rel)
}

fn meta_value(conn: &Connection, key: &str) -> i64 {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get::<_, String>(0)
    })
    .expect("read meta value")
    .parse()
    .expect("meta value is integer")
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
            "orbit-graph-pass1-{name}-{}-{stamp}",
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
