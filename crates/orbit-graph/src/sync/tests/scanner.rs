use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ignore::gitignore::GitignoreBuilder;
use rusqlite::{Connection, params};

use super::{
    ContentHasher, DbLockGuard, OrbitIgnoreMatcher, Scanner, add_default_orbitignore_patterns,
    collect_orbitignore_files, mtime_ns, scan_count, scan_diff,
};
use crate::sync::{SyncLeaderGate, set_sync_leader_gate, sync_leader_count};
use crate::{EXTRACTOR_VERSION, Graph, SyncMode, SyncPolicy, resolve_db_path};

#[test]
fn mtime_fast_path_does_not_hash_clean_rescan_of_100_files() {
    let worktree = TestWorktree::new("mtime-fast-path");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let conn = open_test_connection(worktree.path());
    for index in 0..100 {
        let rel = format!("src/file_{index}.rs");
        worktree.write(&rel, "pub fn marker() {}\n");
        insert_file_row(&conn, worktree.path(), &rel, "pub fn marker() {}\n", None);
    }
    drop(conn);
    drop(graph);

    let hasher = CountingHasher::default();
    let scanner = Scanner::new(graph_db_path(worktree.path()).as_path(), worktree.path())
        .expect("create scanner");
    let diff = scanner.scan(SyncMode::Auto, &hasher).expect("scan");

    assert_eq!(diff.unchanged.len(), 100);
    assert!(diff.modified.is_empty());
    assert!(diff.new.is_empty());
    assert!(diff.deleted.is_empty());
    assert_eq!(hasher.calls(), 0);
}

#[test]
fn stale_mtime_with_matching_hash_is_unchanged_and_updates_mtime() {
    let worktree = TestWorktree::new("stale-mtime");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    worktree.write("src/lib.rs", "pub fn stable() {}\n");
    let conn = open_test_connection(worktree.path());
    insert_file_row(
        &conn,
        worktree.path(),
        "src/lib.rs",
        "pub fn stable() {}\n",
        Some(1),
    );
    drop(conn);
    drop(graph);

    let hasher = CountingHasher::default();
    let scanner = Scanner::new(graph_db_path(worktree.path()).as_path(), worktree.path())
        .expect("create scanner");
    let diff = scanner.scan(SyncMode::Auto, &hasher).expect("scan");

    assert_eq!(diff.unchanged, vec![PathBuf::from("src/lib.rs")]);
    assert!(diff.modified.is_empty());
    assert_eq!(hasher.calls(), 1);
    assert_eq!(
        stored_mtime(&open_test_connection(worktree.path()), "src/lib.rs"),
        mtime_ns(worktree.path().join("src/lib.rs").as_path()).expect("mtime")
    );
}

#[test]
fn stale_mtime_with_different_hash_is_modified() {
    let worktree = TestWorktree::new("modified");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    worktree.write("src/lib.rs", "pub fn changed() {}\n");
    let conn = open_test_connection(worktree.path());
    insert_file_row(
        &conn,
        worktree.path(),
        "src/lib.rs",
        "pub fn old() {}\n",
        Some(1),
    );
    drop(conn);
    drop(graph);

    let hasher = CountingHasher::default();
    let scanner = Scanner::new(graph_db_path(worktree.path()).as_path(), worktree.path())
        .expect("create scanner");
    let diff = scanner.scan(SyncMode::Auto, &hasher).expect("scan");

    assert_eq!(diff.modified, vec![PathBuf::from("src/lib.rs")]);
    assert!(diff.unchanged.is_empty());
    assert_eq!(hasher.calls(), 1);
}

#[test]
fn missing_db_rows_and_missing_disk_files_are_new_and_deleted() {
    let worktree = TestWorktree::new("new-deleted");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    worktree.write("src/new.rs", "pub fn fresh() {}\n");
    let conn = open_test_connection(worktree.path());
    insert_file_row(
        &conn,
        worktree.path(),
        "src/deleted.rs",
        "pub fn gone() {}\n",
        Some(1),
    );
    drop(conn);
    drop(graph);

    let diff = scan_diff(
        graph_db_path(worktree.path()).as_path(),
        worktree.path(),
        SyncMode::Auto,
    )
    .expect("scan");

    assert_eq!(diff.new, vec![PathBuf::from("src/new.rs")]);
    assert_eq!(diff.deleted, vec![PathBuf::from("src/deleted.rs")]);
}

#[test]
fn files_without_registered_extractors_are_filtered() {
    let worktree = TestWorktree::new("language-filter");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    drop(graph);
    worktree.write("src/lib.rs", "pub fn indexed() {}\n");
    worktree.write("notes.unsupported", "not indexed\n");

    let diff = scan_diff(
        graph_db_path(worktree.path()).as_path(),
        worktree.path(),
        SyncMode::Auto,
    )
    .expect("scan");

    assert_eq!(diff.new, vec![PathBuf::from("src/lib.rs")]);
}

#[test]
fn orbitignore_semantics_match_orbit_knowledge_fixture_corpus() {
    let worktree = TestWorktree::new("orbitignore-parity");
    worktree.write(
        ".orbitignore",
        "foo.rs\n**/generated.rs\ngenerated/**\n!generated/keep.rs\nfoo/\n# comment\nbar.rs\n",
    );

    let files = [
        ("foo.rs", false),
        ("bar.rs", false),
        ("baz.rs", false),
        ("src/generated.rs", false),
        ("deep/nested/generated.rs", false),
        ("generated/drop.rs", false),
        ("generated/keep.rs", false),
        ("foo", true),
        ("foo", false),
        ("comment", false),
    ];

    let matcher = OrbitIgnoreMatcher::load(worktree.path()).expect("load matcher");
    let actual = files
        .iter()
        .filter(|(path, is_dir)| matcher.is_ignored(Path::new(path), *is_dir))
        .map(|(path, is_dir)| (*path, *is_dir))
        .collect::<Vec<_>>();
    let expected = vec![
        ("foo.rs", false),
        ("bar.rs", false),
        ("src/generated.rs", false),
        ("deep/nested/generated.rs", false),
        ("generated/drop.rs", false),
        ("foo", true),
    ];

    assert_eq!(actual, expected);
}

#[test]
fn orbitignore_discovery_prunes_default_ignored_directories() {
    let worktree = TestWorktree::new("orbitignore-discovery-prunes-defaults");
    worktree.write(".orbitignore", "# root rules\n");
    worktree.write("src/.orbitignore", "generated.rs\n");

    for ignored_dir in [
        "target",
        "node_modules",
        "dist",
        "build",
        ".venv",
        "venv",
        "__pycache__",
        "crate.egg-info",
        ".orbit",
    ] {
        worktree.write(&format!("{ignored_dir}/.orbitignore"), "*.rs\n");
        worktree.write(&format!("{ignored_dir}/nested/.orbitignore"), "*.rs\n");
    }

    let default_orbitignore = default_orbitignore_matcher(worktree.path());
    let mut orbitignore_files = Vec::new();
    collect_orbitignore_files(
        worktree.path(),
        worktree.path(),
        &default_orbitignore,
        &mut orbitignore_files,
    )
    .expect("collect .orbitignore files");

    let mut relative_files = orbitignore_files
        .into_iter()
        .map(|path| {
            path.strip_prefix(worktree.path())
                .expect("strip worktree prefix")
                .to_path_buf()
        })
        .collect::<Vec<_>>();
    relative_files.sort();

    assert_eq!(
        relative_files,
        vec![
            PathBuf::from(".orbitignore"),
            PathBuf::from("src/.orbitignore")
        ]
    );
}

#[test]
fn nested_orbitignore_in_non_ignored_directory_is_discovered() {
    let worktree = TestWorktree::new("nested-orbitignore-discovered");
    worktree.write("subdir/.orbitignore", "ignored.rs\n");

    let matcher = OrbitIgnoreMatcher::load(worktree.path()).expect("load matcher");

    assert!(matcher.is_ignored(Path::new("subdir/ignored.rs"), false));
    assert!(!matcher.is_ignored(Path::new("subdir/kept.rs"), false));
}

#[test]
fn dropping_scanner_releases_flock() {
    let worktree = TestWorktree::new("flock-release");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let db_path = graph_db_path(worktree.path());

    let scanner = Scanner::new(db_path.as_path(), worktree.path()).expect("create scanner");
    drop(scanner);

    let second = DbLockGuard::acquire(db_path.as_path());
    assert!(second.is_ok());
    drop(graph);
}

#[test]
fn concurrent_same_worktree_sync_coalesces_to_one_scan() {
    let worktree = Arc::new(TestWorktree::new("coalesce"));
    worktree.write("src/lib.rs", "pub fn coalesced() {}\n");
    let first_graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open first graph");
    let second_graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open second graph");
    let db_path = graph_db_path(worktree.path());
    let gate = Arc::new(SyncLeaderGate::new());
    set_sync_leader_gate(Some(Arc::clone(&gate)));

    let first = thread::spawn(move || first_graph.sync(SyncMode::Auto).expect("first sync"));
    assert!(gate.wait_started(Duration::from_secs(2)));
    let second = thread::spawn(move || second_graph.sync(SyncMode::Auto).expect("second sync"));
    thread::sleep(Duration::from_millis(50));
    gate.release();

    let first_report = first.join().expect("join first");
    let second_report = second.join().expect("join second");
    set_sync_leader_gate(None);

    assert_eq!(first_report, second_report);
    assert_eq!(sync_leader_count(db_path.as_path()), 1);
    assert_eq!(scan_count(worktree.path()), 1);
}

#[test]
fn empty_change_scan_1000_file_performance_smoke_prints_elapsed_ms() {
    let worktree = TestWorktree::new("perf-smoke");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let conn = open_test_connection(worktree.path());
    for index in 0..1000 {
        let rel = format!("src/file_{index}.rs");
        worktree.write(&rel, "pub fn marker() {}\n");
        insert_file_row(&conn, worktree.path(), &rel, "pub fn marker() {}\n", None);
    }
    drop(conn);
    drop(graph);

    let started = Instant::now();
    let diff = scan_diff(
        graph_db_path(worktree.path()).as_path(),
        worktree.path(),
        SyncMode::Auto,
    )
    .expect("scan");
    let elapsed = started.elapsed();

    #[allow(clippy::print_stdout)]
    {
        println!("empty_change_scan_1000_files_ms={}", elapsed.as_millis());
    }
    assert_eq!(diff.unchanged.len(), 1000);
}

#[derive(Default)]
struct CountingHasher {
    calls: AtomicUsize,
}

impl CountingHasher {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ContentHasher for CountingHasher {
    fn hash(&self, _path: &Path, bytes: &[u8]) -> Vec<u8> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        blake3::hash(bytes).as_bytes().to_vec()
    }
}

fn insert_file_row(
    conn: &Connection,
    worktree: &Path,
    rel: &str,
    content_for_hash: &str,
    mtime_override: Option<i64>,
) {
    let mtime = mtime_override.unwrap_or_else(|| {
        mtime_ns(worktree.join(rel).as_path()).expect("read mtime for inserted row")
    });
    let hash = blake3::hash(content_for_hash.as_bytes())
        .as_bytes()
        .to_vec();
    conn.execute(
        "INSERT OR REPLACE INTO files (path, content_hash, mtime_ns, lang, byte_len, extracted_at)
         VALUES (?1, ?2, ?3, 'rust', ?4, 0)",
        params![rel, hash, mtime, content_for_hash.len()],
    )
    .expect("insert file row");
}

fn stored_mtime(conn: &Connection, rel: &str) -> i64 {
    conn.query_row("SELECT mtime_ns FROM files WHERE path = ?1", [rel], |row| {
        row.get(0)
    })
    .expect("read stored mtime")
}

fn open_test_connection(worktree: &Path) -> Connection {
    Connection::open(graph_db_path(worktree)).expect("open graph database")
}

fn graph_db_path(worktree: &Path) -> PathBuf {
    resolve_db_path(worktree, "HEAD", EXTRACTOR_VERSION)
        .path()
        .to_path_buf()
}

fn default_orbitignore_matcher(worktree: &Path) -> OrbitIgnoreMatcher {
    let mut builder = GitignoreBuilder::new(worktree);
    add_default_orbitignore_patterns(&mut builder).expect("add default .orbitignore patterns");
    OrbitIgnoreMatcher {
        gitignore: builder.build().expect("build default .orbitignore matcher"),
    }
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
            "orbit-graph-scanner-{name}-{}-{stamp}",
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
