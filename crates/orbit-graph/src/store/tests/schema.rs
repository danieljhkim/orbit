use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{Oid, Repository, RepositoryInitOptions, Signature, build::CheckoutBuilder};
use rusqlite::Connection;

use crate::store::schema::SCHEMA_VERSION;
use crate::{EXTRACTOR_VERSION, Graph, SyncPolicy, resolve_db_path, resolve_db_path_for_commit};

#[test]
fn graph_open_creates_documented_schema_and_initial_meta() {
    let worktree = TestWorktree::new("creates-schema", "feat/schema-open");
    let commit_sha = worktree.init_git_repo();

    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    drop(graph);

    let db_path = resolve_db_path(worktree.path(), "feat/schema-open", EXTRACTOR_VERSION)
        .path()
        .to_path_buf();
    let conn = open_test_connection(&db_path);

    assert_eq!(
        object_names(&conn, "table"),
        BTreeSet::from([
            "commands".to_string(),
            "configs".to_string(),
            "configs_fts".to_string(),
            "configs_fts_config".to_string(),
            "configs_fts_data".to_string(),
            "configs_fts_docsize".to_string(),
            "configs_fts_idx".to_string(),
            "files".to_string(),
            "imports".to_string(),
            "meta".to_string(),
            "refs".to_string(),
            "relations".to_string(),
            "strings".to_string(),
            "strings_fts".to_string(),
            "strings_fts_config".to_string(),
            "strings_fts_data".to_string(),
            "strings_fts_docsize".to_string(),
            "strings_fts_idx".to_string(),
            "symbols".to_string(),
            "symbols_fts".to_string(),
            "symbols_fts_config".to_string(),
            "symbols_fts_data".to_string(),
            "symbols_fts_docsize".to_string(),
            "symbols_fts_idx".to_string(),
        ])
    );
    assert_eq!(
        object_names(&conn, "index"),
        BTreeSet::from([
            "refs_from_file".to_string(),
            "refs_target_name".to_string(),
            "refs_target_qualified".to_string(),
            "relations_from".to_string(),
            "relations_kind".to_string(),
            "relations_to".to_string(),
            "symbols_file".to_string(),
            "symbols_name".to_string(),
            "symbols_qualified".to_string(),
        ])
    );

    for table in [
        "files",
        "symbols",
        "refs",
        "relations",
        "imports",
        "commands",
        "strings",
        "configs",
        "meta",
    ] {
        assert!(
            table_sql(&conn, table).contains("STRICT"),
            "{table} table should be STRICT"
        );
    }
    assert_eq!(
        table_sql(&conn, "symbols_fts"),
        "CREATE VIRTUAL TABLE symbols_fts USING fts5(name, qualified, signature, content='symbols')"
    );
    assert_eq!(
        table_sql(&conn, "strings_fts"),
        "CREATE VIRTUAL TABLE strings_fts USING fts5(value, content='strings')"
    );
    assert_eq!(
        table_sql(&conn, "configs_fts"),
        "CREATE VIRTUAL TABLE configs_fts USING fts5(key, content='configs')"
    );

    let meta = read_meta(&conn);
    assert_eq!(
        meta.get("extractor_version").map(String::as_str),
        Some(EXTRACTOR_VERSION.to_string().as_str())
    );
    assert_eq!(
        meta.get("schema_version").map(String::as_str),
        Some(SCHEMA_VERSION.to_string().as_str())
    );
    assert_eq!(
        meta.get("branch").map(String::as_str),
        Some("feat/schema-open")
    );
    assert_eq!(
        meta.get("commit_sha").map(String::as_str),
        Some(commit_sha.as_str())
    );
    assert_eq!(
        meta.get("last_full_build_at").map(String::as_str),
        Some("0")
    );
    assert_eq!(
        meta.get("last_incremental_at").map(String::as_str),
        Some("0")
    );
}

#[test]
fn graph_open_cleans_stale_version_databases_without_deleting_active_db() {
    let worktree = TestWorktree::new("cleans-stale-dbs", "main");
    worktree.init_git_repo();

    let [stale_version_a, stale_version_b] = stale_versions();
    let stale_main = resolve_db_path(worktree.path(), "main", stale_version_a)
        .path()
        .to_path_buf();
    let stale_feature = resolve_db_path(worktree.path(), "feat/old", stale_version_b)
        .path()
        .to_path_buf();
    fs::create_dir_all(stale_main.parent().expect("stale db parent")).expect("create graph dir");
    fs::write(&stale_main, "stale main").expect("write stale main db");
    fs::write(&stale_feature, "stale feature").expect("write stale feature db");

    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let active_db = graph.db_path().path().to_path_buf();
    drop(graph);

    assert!(
        active_db.exists(),
        "active DB should remain after auto-clean"
    );
    assert!(
        !stale_main.exists(),
        "same-branch stale DB should be removed"
    );
    assert!(
        !stale_feature.exists(),
        "other-branch stale DB should be removed"
    );
}

#[test]
fn graph_open_records_commit_sha_for_detached_head() {
    let worktree = TestWorktree::new("detached-head-meta", "main");
    let commit_sha = worktree.init_git_repo();
    worktree.detach_head(commit_sha.as_str());

    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open detached graph");
    drop(graph);

    let db_path = resolve_db_path_for_commit(
        worktree.path(),
        "HEAD",
        commit_sha.as_str(),
        EXTRACTOR_VERSION,
    )
    .path()
    .to_path_buf();
    let conn = open_test_connection(&db_path);
    let meta = read_meta(&conn);
    assert_eq!(meta.get("branch").map(String::as_str), Some("HEAD"));
    assert_eq!(
        meta.get("commit_sha").map(String::as_str),
        Some(commit_sha.as_str())
    );
    assert!(!commit_sha.is_empty());
}

#[test]
fn graph_open_uses_distinct_db_files_for_detached_commits() {
    let worktree = TestWorktree::new("detached-distinct-dbs", "main");
    let first_commit = worktree.init_git_repo();
    let second_commit = worktree.commit_file("second.txt", "second\n", "second");

    worktree.detach_head(first_commit.as_str());
    let first_graph =
        Graph::open(worktree.path(), SyncPolicy::Manual).expect("open first detached graph");
    let first_db = first_graph.db_path().path().to_path_buf();
    drop(first_graph);

    worktree.detach_head(second_commit.as_str());
    let second_graph =
        Graph::open(worktree.path(), SyncPolicy::Manual).expect("open second detached graph");
    let second_db = second_graph.db_path().path().to_path_buf();
    drop(second_graph);

    assert_ne!(first_db, second_db);
    assert!(
        first_db.exists(),
        "first detached DB should remain after opening another detached commit"
    );
    assert!(
        second_db.exists(),
        "second detached DB should be created separately"
    );
    let expected_first_file = format!("detached-{}.{}.db", &first_commit[..12], EXTRACTOR_VERSION);
    let expected_second_file =
        format!("detached-{}.{}.db", &second_commit[..12], EXTRACTOR_VERSION);
    assert_eq!(
        first_db.file_name().and_then(|name| name.to_str()),
        Some(expected_first_file.as_str())
    );
    assert_eq!(
        second_db.file_name().and_then(|name| name.to_str()),
        Some(expected_second_file.as_str())
    );
}

#[test]
fn graph_open_cleans_unreachable_detached_databases() {
    let worktree = TestWorktree::new("cleans-unreachable-detached", "main");
    worktree.init_git_repo();
    let reachable_commit = worktree.commit_file("reachable.txt", "reachable\n", "reachable");
    worktree.detach_head(reachable_commit.as_str());
    let reachable_graph =
        Graph::open(worktree.path(), SyncPolicy::Manual).expect("open reachable detached graph");
    let reachable_db = reachable_graph.db_path().path().to_path_buf();
    drop(reachable_graph);

    worktree.checkout_branch();
    let unreachable_commit =
        worktree.create_unreachable_detached_commit("unreachable.txt", "unreachable\n");
    let unreachable_graph =
        Graph::open(worktree.path(), SyncPolicy::Manual).expect("open unreachable detached graph");
    let unreachable_db = unreachable_graph.db_path().path().to_path_buf();
    drop(unreachable_graph);

    worktree.checkout_branch();
    let main_graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open main graph");
    let main_db = main_graph.db_path().path().to_path_buf();
    drop(main_graph);

    assert!(main_db.exists(), "active branch DB should remain");
    assert!(
        reachable_db.exists(),
        "detached DB reachable from a branch ref should remain"
    );
    assert!(
        !unreachable_db.exists(),
        "detached DB for commit {unreachable_commit} should be cleaned once no local ref reaches it"
    );
}

#[test]
fn reopening_existing_db_preserves_meta_rows() {
    let worktree = TestWorktree::new("preserves-meta", "main");
    worktree.init_git_repo();

    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("first open");
    drop(graph);

    let db_path = resolve_db_path(worktree.path(), "main", EXTRACTOR_VERSION)
        .path()
        .to_path_buf();
    {
        let conn = open_test_connection(&db_path);
        conn.execute(
            "UPDATE meta SET value = 'kept' WHERE key = 'last_full_build_at'",
            [],
        )
        .expect("mutate meta before reopen");
        conn.execute("UPDATE meta SET value = 'manual' WHERE key = 'branch'", [])
            .expect("mutate branch before reopen");
    }

    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("second open");
    drop(graph);

    let conn = open_test_connection(&db_path);
    let meta = read_meta(&conn);
    assert_eq!(
        meta.get("last_full_build_at").map(String::as_str),
        Some("kept")
    );
    assert_eq!(meta.get("branch").map(String::as_str), Some("manual"));
}

#[test]
fn refs_target_symbol_hint_has_no_symbol_foreign_key() {
    let worktree = TestWorktree::new("refs-no-fk", "main");
    worktree.init_git_repo();

    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    drop(graph);

    let db_path = resolve_db_path(worktree.path(), "main", EXTRACTOR_VERSION)
        .path()
        .to_path_buf();
    let conn = open_test_connection(&db_path);
    let refs_sql = normalized_sql(&table_sql(&conn, "refs"));

    assert!(refs_sql.contains("target_symbol_hint INTEGER"));
    assert!(
        !refs_sql.contains("target_symbol_hint INTEGER REFERENCES"),
        "refs.target_symbol_hint must stay non-authoritative and must not reference symbols(id): {refs_sql}"
    );
}

#[test]
fn deleting_file_cascades_every_file_anchored_row() {
    let worktree = TestWorktree::new("cascade-delete", "main");
    worktree.init_git_repo();

    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    drop(graph);

    let db_path = resolve_db_path(worktree.path(), "main", EXTRACTOR_VERSION)
        .path()
        .to_path_buf();
    let conn = open_test_connection(&db_path);
    insert_file_anchored_rows(&conn);

    conn.execute("DELETE FROM files WHERE path = 'src/lib.rs'", [])
        .expect("delete file row");

    for table in [
        "symbols",
        "refs",
        "relations",
        "imports",
        "commands",
        "strings",
        "configs",
    ] {
        assert_eq!(
            row_count(&conn, table),
            0,
            "{table} should cascade on file delete"
        );
    }
}

fn insert_file_anchored_rows(conn: &Connection) {
    conn.execute(
        "INSERT INTO files (path, content_hash, mtime_ns, lang, byte_len, extracted_at)
         VALUES ('src/lib.rs', x'00', 1, 'rust', 12, 2)",
        [],
    )
    .expect("insert file");
    conn.execute(
        "INSERT INTO symbols (
            id, file_path, name, qualified, kind, span_start, span_end, signature, parent_symbol
         ) VALUES (1, 'src/lib.rs', 'run', 'crate::run', 'function', 0, 3, 'fn run()', NULL)",
        [],
    )
    .expect("insert symbol");
    conn.execute(
        "INSERT INTO refs (
            from_file, from_span_start, from_span_end, target_name, target_qualified,
            target_symbol_hint, kind, confidence
         ) VALUES ('src/lib.rs', 4, 7, 'run', 'crate::run', 1, 'call', 'exact')",
        [],
    )
    .expect("insert ref");
    conn.execute(
        "INSERT INTO relations (
            from_qualified, to_qualified, kind, def_file, def_span_start, def_span_end, confidence
         ) VALUES ('crate::Type', 'crate::Trait', 'impl', 'src/lib.rs', 0, 10, 'exact')",
        [],
    )
    .expect("insert relation");
    conn.execute(
        "INSERT INTO imports (from_file, target_path, target_symbol)
         VALUES ('src/lib.rs', 'crate::other', 'Other')",
        [],
    )
    .expect("insert import");
    conn.execute(
        "INSERT INTO commands (name, file_path, span_start, handler_symbol)
         VALUES ('run', 'src/lib.rs', 0, 1)",
        [],
    )
    .expect("insert command");
    conn.execute(
        "INSERT INTO strings (file_path, line, value, context_symbol)
         VALUES ('src/lib.rs', 1, 'hello world', 1)",
        [],
    )
    .expect("insert string");
    conn.execute(
        "INSERT INTO configs (file_path, line, key, kind)
         VALUES ('src/lib.rs', 1, 'app.name', 'toml')",
        [],
    )
    .expect("insert config");
}

fn open_test_connection(path: &Path) -> Connection {
    let conn = Connection::open(path).expect("open test sqlite connection");
    conn.pragma_update(None, "foreign_keys", "ON")
        .expect("enable foreign keys");
    conn
}

fn object_names(conn: &Connection, object_type: &str) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type = ?1 AND name NOT LIKE 'sqlite_%'
             ORDER BY name",
        )
        .expect("prepare object name query");
    stmt.query_map([object_type], |row| row.get::<_, String>(0))
        .expect("query object names")
        .collect::<Result<BTreeSet<_>, _>>()
        .expect("collect object names")
}

fn table_sql(conn: &Connection, name: &str) -> String {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE name = ?1",
        [name],
        |row| row.get(0),
    )
    .expect("read sqlite_master sql")
}

fn normalized_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn read_meta(conn: &Connection) -> BTreeMap<String, String> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM meta ORDER BY key")
        .expect("prepare meta query");
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query meta")
        .collect::<Result<BTreeMap<_, _>, _>>()
        .expect("collect meta")
}

fn row_count(conn: &Connection, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    conn.query_row(&sql, [], |row| row.get(0))
        .expect("count table rows")
}

fn stale_versions() -> [u32; 2] {
    [
        if EXTRACTOR_VERSION > 0 {
            EXTRACTOR_VERSION - 1
        } else {
            EXTRACTOR_VERSION + 1
        },
        if EXTRACTOR_VERSION > 1 {
            EXTRACTOR_VERSION - 2
        } else {
            EXTRACTOR_VERSION + 2
        },
    ]
}

struct TestWorktree {
    path: PathBuf,
    branch: String,
}

impl TestWorktree {
    fn new(name: &str, branch: &str) -> Self {
        let mut path = std::env::temp_dir();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos();
        path.push(format!("orbit-graph-{name}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&path).expect("create test worktree");
        Self {
            path,
            branch: branch.to_string(),
        }
    }

    fn path(&self) -> &Path {
        self.path.as_path()
    }

    fn init_git_repo(&self) -> String {
        let mut opts = RepositoryInitOptions::new();
        opts.initial_head(self.branch.as_str());
        let repo = Repository::init_opts(&self.path, &opts).expect("init git repo");
        fs::write(self.path.join("README.md"), "test\n").expect("write initial file");

        let mut index = repo.index().expect("open repo index");
        index
            .add_path(Path::new("README.md"))
            .expect("add initial file");
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("find tree");
        let sig = Signature::now("Orbit Test", "orbit@example.test").expect("test signature");
        let oid = repo
            .commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .expect("initial commit");
        oid.to_string()
    }

    fn commit_file(&self, rel: &str, content: &str, message: &str) -> String {
        let repo = Repository::open(&self.path).expect("open git repo");
        let path = self.path.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create commit file parent");
        }
        fs::write(path, content).expect("write commit file");

        let mut index = repo.index().expect("open repo index");
        index.add_path(Path::new(rel)).expect("add commit file");
        index.write().expect("write index");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repo.find_tree(tree_id).expect("find tree");
        let parent = repo
            .head()
            .expect("read HEAD")
            .peel_to_commit()
            .expect("peel HEAD to commit");
        let sig = Signature::now("Orbit Test", "orbit@example.test").expect("test signature");
        let oid = repo
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
            .expect("create commit");
        oid.to_string()
    }

    fn checkout_branch(&self) {
        let repo = Repository::open(&self.path).expect("open git repo");
        let branch_ref = format!("refs/heads/{}", self.branch);
        repo.set_head(branch_ref.as_str())
            .expect("set HEAD to branch");
        let mut checkout = CheckoutBuilder::new();
        checkout.force();
        repo.checkout_head(Some(&mut checkout))
            .expect("checkout branch");
    }

    fn create_unreachable_detached_commit(&self, rel: &str, content: &str) -> String {
        let repo = Repository::open(&self.path).expect("open git repo");
        let branch_name = format!("refs/heads/{}", self.branch);
        let branch_commit_id = repo
            .find_reference(branch_name.as_str())
            .expect("find branch ref")
            .peel_to_commit()
            .expect("peel branch to commit")
            .id();
        repo.set_head_detached(branch_commit_id)
            .expect("detach from branch commit");
        drop(repo);
        self.commit_file(rel, content, "unreachable detached")
    }

    fn detach_head(&self, commit_sha: &str) {
        let repo = Repository::open(&self.path).expect("open git repo");
        let oid = Oid::from_str(commit_sha).expect("parse commit oid");
        repo.set_head_detached(oid).expect("detach HEAD");
    }
}

impl Drop for TestWorktree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
