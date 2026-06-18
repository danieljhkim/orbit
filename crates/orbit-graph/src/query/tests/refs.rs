use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};

use crate::sync::sync_leader_count;
use crate::{
    EXTRACTOR_VERSION, Graph, RefConfidence, RefKind, RefOpts, RefResult, RefTarget, Selector,
    SyncPolicy, resolve_db_path,
};

#[test]
fn refs_result_shape_matches_golden_fixture_and_skips_unresolved_qualified() {
    let result = RefResult {
        target: RefTarget {
            name: "Missing".to_string(),
            qualified: None,
        },
        refs: Vec::new(),
        relations: Vec::new(),
        skipped_low_confidence: 0,
        fallback: None,
    };

    crate::query::tests::support::assert_json_matches_fixture(
        &result,
        include_str!("refs.golden.json"),
    );
}

#[test]
fn default_floor_skips_fuzzy_refs_and_reports_count() {
    let worktree = TestWorktree::new("skip-fuzzy");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let conn = open_test_connection(worktree.path());
    seed_target(
        &conn,
        worktree.path(),
        "src/target.rs",
        "Target",
        "crate::Target",
    );
    seed_file(
        &conn,
        worktree.path(),
        "src/caller.rs",
        "Target::a();\nTarget::b();\n",
    );
    for index in 0..2 {
        insert_ref(
            &conn,
            "src/caller.rs",
            "Target",
            Some("crate::Target"),
            "call",
            "exact",
            index,
        );
    }
    for index in 2..5 {
        insert_ref(
            &conn,
            "src/caller.rs",
            "Target",
            Some("crate::Target"),
            "call",
            "fuzzy_name",
            index,
        );
    }

    let result = graph
        .refs(&target_selector(), &RefOpts::default())
        .expect("query refs");

    assert_eq!(result.refs.len(), 2);
    assert_eq!(result.relations.len(), 0);
    assert_eq!(result.skipped_low_confidence, 3);
    assert!(
        result.refs.iter().all(|entry| {
            entry.kind == RefKind::Call && entry.confidence == RefConfidence::Exact
        })
    );
}

#[test]
fn fuzzy_floor_includes_fuzzy_name_refs() {
    let worktree = TestWorktree::new("include-fuzzy");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let conn = open_test_connection(worktree.path());
    seed_target(
        &conn,
        worktree.path(),
        "src/target.rs",
        "Target",
        "crate::Target",
    );
    seed_file(&conn, worktree.path(), "src/caller.rs", "Target::a();\n");
    insert_ref(
        &conn,
        "src/caller.rs",
        "Target",
        Some("crate::Target"),
        "call",
        "fuzzy_name",
        0,
    );

    let result = graph
        .refs(
            &target_selector(),
            &RefOpts {
                confidence: RefConfidence::FuzzyName,
                kind: None,
            },
        )
        .expect("query fuzzy refs");

    assert_eq!(result.refs.len(), 1);
    assert_eq!(result.refs[0].confidence, RefConfidence::FuzzyName);
    assert_eq!(result.skipped_low_confidence, 0);
}

#[test]
fn fuzzy_floor_includes_name_only_fuzzy_refs() {
    let worktree = TestWorktree::new("include-name-only-fuzzy");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let conn = open_test_connection(worktree.path());
    seed_target(
        &conn,
        worktree.path(),
        "src/target.rs",
        "Target",
        "crate::Target",
    );
    seed_file(
        &conn,
        worktree.path(),
        "src/caller.rs",
        "Target::fuzzy();\n",
    );
    insert_ref(
        &conn,
        "src/caller.rs",
        "Target",
        None,
        "call",
        "fuzzy_name",
        0,
    );

    let result = graph
        .refs(
            &target_selector(),
            &RefOpts {
                confidence: RefConfidence::FuzzyName,
                kind: None,
            },
        )
        .expect("query name-only fuzzy refs");

    assert_eq!(result.refs.len(), 1);
    assert_eq!(result.refs[0].kind, RefKind::Call);
    assert_eq!(result.refs[0].confidence, RefConfidence::FuzzyName);
    assert_eq!(result.skipped_low_confidence, 0);
}

#[test]
fn precise_floor_empty_falls_back_to_name_only_fuzzy_refs() {
    // Repro for ORB-00383: a public symbol whose only callers resolve at
    // `fuzzy_name` confidence (target_qualified = NULL) looks unreferenced at the
    // default `same_module` floor. The precise query keys on `target_qualified`,
    // so these rows are not even counted as skipped — the fallback must re-query.
    let worktree = TestWorktree::new("fallback-name-only");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let conn = open_test_connection(worktree.path());
    seed_target(
        &conn,
        worktree.path(),
        "src/target.rs",
        "Target",
        "crate::Target",
    );
    seed_file(&conn, worktree.path(), "src/caller.rs", "Target::a();\n");
    insert_ref(
        &conn,
        "src/caller.rs",
        "Target",
        None,
        "call",
        "fuzzy_name",
        0,
    );

    let result = graph
        .refs(&target_selector(), &RefOpts::default())
        .expect("query refs");

    assert!(
        result.refs.is_empty(),
        "precise floor finds no qualified refs"
    );
    assert_eq!(
        result.skipped_low_confidence, 0,
        "fuzzy rows are not counted"
    );
    let fallback = result.fallback.expect("fallback populated");
    assert_eq!(fallback.confidence, RefConfidence::FuzzyName);
    assert_eq!(fallback.refs.len(), 1);
    assert_eq!(fallback.refs[0].kind, RefKind::Call);
    assert_eq!(fallback.refs[0].confidence, RefConfidence::FuzzyName);
    assert!(
        fallback.note.contains("same_module") && fallback.note.contains("fuzzy_name"),
        "note names both the precise floor and the fallback floor: {}",
        fallback.note
    );
}

#[test]
fn precise_match_suppresses_fuzzy_fallback() {
    // A real precise caller plus a same-named fuzzy caller: the precise result is
    // non-empty, so no fallback is emitted (no noise for symbols that resolve).
    let worktree = TestWorktree::new("fallback-suppressed");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let conn = open_test_connection(worktree.path());
    seed_target(
        &conn,
        worktree.path(),
        "src/target.rs",
        "Target",
        "crate::Target",
    );
    seed_file(
        &conn,
        worktree.path(),
        "src/caller.rs",
        "Target::a();\nTarget::b();\n",
    );
    insert_ref(
        &conn,
        "src/caller.rs",
        "Target",
        Some("crate::Target"),
        "call",
        "exact",
        0,
    );
    insert_ref(
        &conn,
        "src/caller.rs",
        "Target",
        None,
        "call",
        "fuzzy_name",
        2,
    );

    let result = graph
        .refs(&target_selector(), &RefOpts::default())
        .expect("query refs");

    assert_eq!(result.refs.len(), 1);
    assert!(
        result.fallback.is_none(),
        "fallback only fires when precise refs are empty"
    );
}

#[test]
fn fuzzy_floor_query_emits_no_fallback() {
    // When the caller already requests the fuzzy floor, name-only matches appear
    // in `refs` directly — there is nothing to fall back to.
    let worktree = TestWorktree::new("fallback-explicit-fuzzy");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let conn = open_test_connection(worktree.path());
    seed_target(
        &conn,
        worktree.path(),
        "src/target.rs",
        "Target",
        "crate::Target",
    );
    seed_file(&conn, worktree.path(), "src/caller.rs", "Target::a();\n");
    insert_ref(
        &conn,
        "src/caller.rs",
        "Target",
        None,
        "call",
        "fuzzy_name",
        0,
    );

    let result = graph
        .refs(
            &target_selector(),
            &RefOpts {
                confidence: RefConfidence::FuzzyName,
                kind: None,
            },
        )
        .expect("query refs at fuzzy floor");

    assert_eq!(result.refs.len(), 1);
    assert!(result.fallback.is_none());
}

#[test]
fn kind_filter_routes_to_textual_refs_or_structural_relations() {
    let worktree = TestWorktree::new("kind-routing");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let conn = open_test_connection(worktree.path());
    seed_target(
        &conn,
        worktree.path(),
        "src/target.rs",
        "Target",
        "crate::Target",
    );
    seed_file(&conn, worktree.path(), "src/caller.rs", "Target::a();\n");
    insert_ref(
        &conn,
        "src/caller.rs",
        "Target",
        Some("crate::Target"),
        "call",
        "exact",
        0,
    );
    insert_relation(
        &conn,
        worktree.path(),
        "src/impl.rs",
        "crate::Worker",
        "crate::Target",
        "impl",
        "exact",
    );

    let union = graph
        .refs(&target_selector(), &RefOpts::default())
        .expect("query union refs");
    assert_eq!(union.refs.len(), 1);
    assert_eq!(union.relations.len(), 1);

    let calls = graph
        .refs(
            &target_selector(),
            &RefOpts {
                confidence: RefConfidence::SameModule,
                kind: Some(RefKind::Call),
            },
        )
        .expect("query call refs");
    assert_eq!(calls.refs.len(), 1);
    assert_eq!(calls.relations.len(), 0);

    let impls = graph
        .refs(
            &target_selector(),
            &RefOpts {
                confidence: RefConfidence::SameModule,
                kind: Some(RefKind::Impl),
            },
        )
        .expect("query impl refs");
    assert_eq!(impls.refs.len(), 0);
    assert_eq!(impls.relations.len(), 1);
}

#[test]
fn unresolved_symbol_selector_returns_empty_result() {
    let worktree = TestWorktree::new("unresolved");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let selector =
        Selector::from_str("symbol:src/missing.rs#Missing:function").expect("parse selector");

    let result = graph
        .refs(&selector, &RefOpts::default())
        .expect("unresolved selector is not an error");

    assert_eq!(result.target.name, "Missing");
    assert_eq!(result.target.qualified, None);
    assert!(result.refs.is_empty());
    assert!(result.relations.is_empty());
    assert_eq!(result.skipped_low_confidence, 0);
}

#[test]
fn refs_calls_ensure_synced_at_entry() {
    let worktree = TestWorktree::new("ensure-synced");
    worktree.write("src/lib.rs", "pub fn synced_target() {}\n");
    let graph = Graph::open(worktree.path(), SyncPolicy::OnRead).expect("open graph");
    let db_path = graph_db_path(worktree.path());
    let selector =
        Selector::from_str("symbol:src/lib.rs#synced_target:function").expect("parse selector");

    let result = graph
        .refs(&selector, &RefOpts::default())
        .expect("refs triggers sync");

    assert_eq!(sync_leader_count(db_path.as_path()), 1);
    assert_eq!(result.target.qualified.as_deref(), Some("synced_target"));
}

#[test]
fn refs_10k_fixture_performance_smoke_prints_elapsed_ms() {
    let worktree = TestWorktree::new("perf-10k");
    let graph = Graph::open(worktree.path(), SyncPolicy::Manual).expect("open graph");
    let conn = open_test_connection(worktree.path());
    seed_target(
        &conn,
        worktree.path(),
        "src/target.rs",
        "Target",
        "crate::Target",
    );
    seed_file(&conn, worktree.path(), "src/caller.rs", "Target::call();\n");
    let tx = conn.unchecked_transaction().expect("begin refs fixture tx");
    for _ in 0..10_000 {
        tx.execute(
            "INSERT INTO refs (
                from_file, from_span_start, from_span_end, target_name, target_qualified,
                target_symbol_hint, kind, confidence
             ) VALUES ('src/caller.rs', 0, 6, 'Target', 'crate::Target', NULL, 'call', 'exact')",
            [],
        )
        .expect("insert refs fixture row");
    }
    tx.commit().expect("commit refs fixture tx");

    let started = Instant::now();
    let result = graph
        .refs(&target_selector(), &RefOpts::default())
        .expect("query 10k refs");
    let elapsed = started.elapsed();

    #[allow(clippy::print_stdout)]
    {
        println!("refs_10k_fixture_ms={}", elapsed.as_millis());
    }
    assert_eq!(result.refs.len(), 10_000);
}

fn target_selector() -> Selector {
    Selector::from_str("symbol:src/target.rs#Target:struct").expect("parse target selector")
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

fn seed_target(conn: &Connection, worktree: &Path, file: &str, name: &str, qualified: &str) {
    seed_file(conn, worktree, file, "pub struct Target;\n");
    conn.execute(
        "INSERT INTO symbols (
            file_path, name, qualified, kind, span_start, span_end, signature, parent_symbol
         ) VALUES (?1, ?2, ?3, 'struct', 0, 6, NULL, NULL)",
        params![file, name, qualified],
    )
    .expect("insert target symbol");
}

fn seed_file(conn: &Connection, worktree: &Path, file: &str, content: &str) {
    worktree_write(worktree, file, content);
    conn.execute(
        "INSERT OR IGNORE INTO files (path, content_hash, mtime_ns, lang, byte_len, extracted_at)
         VALUES (?1, x'00', 1, 'rust', ?2, 2)",
        params![file, content.len()],
    )
    .expect("insert graph file");
}

fn insert_ref(
    conn: &Connection,
    from_file: &str,
    target_name: &str,
    target_qualified: Option<&str>,
    kind: &str,
    confidence: &str,
    index: i64,
) {
    conn.execute(
        "INSERT INTO refs (
            from_file, from_span_start, from_span_end, target_name, target_qualified,
            target_symbol_hint, kind, confidence
         ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7)",
        params![
            from_file,
            index,
            index + 1,
            target_name,
            target_qualified,
            kind,
            confidence
        ],
    )
    .expect("insert ref row");
}

fn insert_relation(
    conn: &Connection,
    worktree: &Path,
    file: &str,
    from_qualified: &str,
    to_qualified: &str,
    kind: &str,
    confidence: &str,
) {
    seed_file(conn, worktree, file, "impl Target for Worker {}\n");
    conn.execute(
        "INSERT INTO relations (
            from_qualified, to_qualified, kind, def_file, def_span_start, def_span_end, confidence
         ) VALUES (?1, ?2, ?3, ?4, 0, 4, ?5)",
        params![from_qualified, to_qualified, kind, file, confidence],
    )
    .expect("insert relation row");
}

fn worktree_write(worktree: &Path, rel: &str, content: &str) {
    let path = worktree.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directory");
    }
    fs::write(path, content).expect("write test source");
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
            "orbit-graph-refs-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test worktree");
        Self { path }
    }

    fn path(&self) -> &Path {
        self.path.as_path()
    }

    fn write(&self, rel: &str, content: &str) {
        worktree_write(self.path.as_path(), rel, content);
    }
}

impl Drop for TestWorktree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
