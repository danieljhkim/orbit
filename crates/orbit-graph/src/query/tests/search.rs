use std::time::Instant;

use rusqlite::{Connection, params};

use super::{Match, SearchKind, SearchQuery};
use crate::query::tests::support::{
    TestWorktree, assert_json_matches_fixture, graph_db_path, insert_file, insert_symbol,
    open_connection, open_graph,
};
use crate::sync::sync_leader_count;
use crate::{SyncMode, SyncPolicy};

#[test]
fn search_result_shape_matches_golden_fixture() {
    let result = super::SearchResult {
        matches: vec![
            Match::Symbol {
                name: "run_due_schedulers".to_string(),
                path: "crates/orbit-core/src/scheduler/scheduler.rs".to_string(),
                line: 142,
            },
            Match::StringLiteral {
                value: "scheduler tick failed".to_string(),
                path: "crates/orbit-core/src/scheduler/runner.rs".to_string(),
                line: 88,
            },
            Match::Config {
                value: "scheduler.interval".to_string(),
                path: "crates/orbit-core/Cargo.toml".to_string(),
                line: 12,
            },
        ],
    };

    assert_json_matches_fixture(&result, include_str!("search.golden.json"));
}

#[test]
fn search_unions_fts_tables_and_filters_kind_and_lang() {
    let worktree = TestWorktree::new("search-union");
    let rust_source = "\n\npub fn scheduler() {}\n";
    let py_source = "print('scheduler tick failed')\n";
    worktree.write("src/lib.rs", rust_source);
    worktree.write("scripts/tick.py", py_source);
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    insert_file(&conn, "src/lib.rs", "rust", rust_source);
    insert_file(&conn, "scripts/tick.py", "python", py_source);
    let start = rust_source.find("pub fn scheduler").expect("symbol start");
    insert_symbol(
        &conn,
        "src/lib.rs",
        "scheduler",
        "crate::scheduler",
        "function",
        start,
        start + "pub fn scheduler() {}".len(),
    );
    insert_string(&conn, "src/lib.rs", 4, "scheduler tick failed");
    insert_config(&conn, "src/lib.rs", 5, "scheduler.interval");
    insert_string(&conn, "scripts/tick.py", 1, "scheduler tick failed");

    let all = graph
        .search(&SearchQuery::new("scheduler"))
        .expect("search all kinds");

    assert!(all.matches.iter().any(|mat| {
        matches!(
            mat,
            Match::Symbol { name, path, line }
                if name == "scheduler" && path == "src/lib.rs" && *line == 3
        )
    }));
    assert!(all.matches.iter().any(|mat| {
        matches!(
            mat,
            Match::StringLiteral { value, path, line }
                if value == "scheduler tick failed" && path == "src/lib.rs" && *line == 4
        )
    }));
    assert!(all.matches.iter().any(|mat| {
        matches!(
            mat,
            Match::Config { value, path, line }
                if value == "scheduler.interval" && path == "src/lib.rs" && *line == 5
        )
    }));

    let rust_strings = graph
        .search(&SearchQuery {
            query: "scheduler".to_string(),
            kind: Some(SearchKind::String),
            lang: Some("rust".to_string()),
            limit: Some(10),
        })
        .expect("search rust strings");

    assert_eq!(
        rust_strings.matches,
        vec![Match::StringLiteral {
            value: "scheduler tick failed".to_string(),
            path: "src/lib.rs".to_string(),
            line: 4,
        }]
    );
}

#[test]
fn search_defaults_to_twenty_matches_and_limit_overrides() {
    let worktree = TestWorktree::new("search-limit");
    let source = "pub fn needle_fixture() {}\n";
    worktree.write("src/lib.rs", source);
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    insert_file(&conn, "src/lib.rs", "rust", source);
    for index in 0..25 {
        insert_symbol(
            &conn,
            "src/lib.rs",
            &format!("needle_{index}"),
            &format!("crate::needle_{index}"),
            "function",
            0,
            source.len(),
        );
    }

    let defaulted = graph
        .search(&SearchQuery::new("needle"))
        .expect("search default limit");
    let overridden = graph
        .search(&SearchQuery {
            query: "needle".to_string(),
            kind: Some(SearchKind::Symbol),
            lang: None,
            limit: Some(25),
        })
        .expect("search override limit");

    assert_eq!(defaulted.matches.len(), 20);
    assert_eq!(overridden.matches.len(), 25);
}

#[test]
fn search_calls_ensure_synced_at_entry() {
    let worktree = TestWorktree::new("search-ensure");
    worktree.write("src/lib.rs", "pub fn auto_sync_marker() {}\n");
    let graph = open_graph(&worktree, SyncPolicy::OnRead);
    let db_path = graph_db_path(&worktree);

    let result = graph
        .search(&SearchQuery::new("auto_sync_marker"))
        .expect("search with on-read sync");

    assert_eq!(sync_leader_count(db_path.as_path()), 1);
    assert_eq!(result.matches.len(), 1);
}

#[test]
fn search_10000_symbol_performance_smoke_prints_elapsed_ms() {
    let worktree = TestWorktree::new("search-perf");
    let source = "pub fn needle_fixture() {}\n";
    worktree.write("src/lib.rs", source);
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let mut conn = open_connection(&worktree);
    let tx = conn.transaction().expect("begin insert transaction");
    tx.execute(
        "INSERT INTO files (path, content_hash, mtime_ns, lang, byte_len, extracted_at)
         VALUES ('src/lib.rs', x'00', 1, 'rust', ?1, 2)",
        params![i64::try_from(source.len()).expect("source length fits")],
    )
    .expect("insert file");
    for index in 0..10_000 {
        let name = format!("needle_{index}");
        let qualified = format!("crate::{name}");
        tx.execute(
            "INSERT INTO symbols (
                file_path, name, qualified, kind, span_start, span_end, signature, parent_symbol
             ) VALUES ('src/lib.rs', ?1, ?2, 'function', 0, ?3, ?4, NULL)",
            params![
                name,
                qualified,
                i64::try_from(source.len()).expect("source length fits"),
                format!("fn needle_{index}()")
            ],
        )
        .expect("insert symbol");
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO symbols_fts (rowid, name, qualified, signature)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                id,
                format!("needle_{index}"),
                format!("crate::needle_{index}"),
                format!("fn needle_{index}()")
            ],
        )
        .expect("insert symbol fts");
    }
    tx.commit().expect("commit insert transaction");

    let started = Instant::now();
    let result = graph
        .search(&SearchQuery {
            query: "needle".to_string(),
            kind: Some(SearchKind::Symbol),
            lang: None,
            limit: None,
        })
        .expect("search synthetic fixture");
    let elapsed = started.elapsed();

    #[allow(clippy::print_stdout)]
    {
        println!("graph_search_10000_symbols_ms={}", elapsed.as_millis());
    }
    assert_eq!(result.matches.len(), 20);
}

#[test]
fn search_sync_populates_fts_rows() {
    let worktree = TestWorktree::new("search-sync-fts");
    worktree.write("src/lib.rs", "pub fn synced_needle() {}\n");
    let graph = open_graph(&worktree, SyncPolicy::Manual);

    graph.sync(SyncMode::Full).expect("full sync");
    let result = graph
        .search(&SearchQuery::new("synced_needle"))
        .expect("search synced fts");

    assert_eq!(result.matches.len(), 1);
}

fn insert_string(conn: &Connection, file_path: &str, line: usize, value: &str) {
    conn.execute(
        "INSERT INTO strings (file_path, line, value, context_symbol)
         VALUES (?1, ?2, ?3, NULL)",
        params![
            file_path,
            i64::try_from(line).expect("string line fits"),
            value
        ],
    )
    .expect("insert string row");
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO strings_fts (rowid, value) VALUES (?1, ?2)",
        params![id, value],
    )
    .expect("insert string fts row");
}

fn insert_config(conn: &Connection, file_path: &str, line: usize, key: &str) {
    conn.execute(
        "INSERT INTO configs (file_path, line, key, kind)
         VALUES (?1, ?2, ?3, 'toml')",
        params![
            file_path,
            i64::try_from(line).expect("config line fits"),
            key
        ],
    )
    .expect("insert config row");
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO configs_fts (rowid, key) VALUES (?1, ?2)",
        params![id, key],
    )
    .expect("insert config fts row");
}
