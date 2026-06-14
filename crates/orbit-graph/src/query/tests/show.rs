use std::fs;

use orbit_graph_extract::Selector;
use rusqlite::{Connection, params};
use serde_json::json;

use super::{DEFAULT_SHOW_MAX_BYTES, NodeMetadata, NodeView, SourceSpan};
use crate::SyncPolicy;
use crate::query::tests::support::{
    TestWorktree, assert_json_matches_fixture, graph_db_path, insert_file, insert_symbol,
    open_connection, open_graph,
};
use crate::sync::sync_leader_count;

#[test]
fn show_result_shape_matches_golden_fixture() {
    let result = NodeView {
        bytes: b"pub fn handler() {}\n".to_vec(),
        metadata: NodeMetadata {
            file: "src/lib.rs".to_string(),
            span: SourceSpan { start: 0, end: 20 },
            kind: "function".to_string(),
            name: Some("handler".to_string()),
            qualified: Some("crate::handler".to_string()),
            truncated: false,
        },
    };

    assert_json_matches_fixture(&result, include_str!("show.golden.json"));
}

#[test]
fn show_resolves_symbol_file_module_and_command_selectors() {
    let worktree = TestWorktree::new("show-selectors");
    let source = "pub mod api {\n    pub fn handler() {\n        println!(\"hi\");\n    }\n}\n";
    worktree.write("src/lib.rs", source);
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    insert_file(&conn, "src/lib.rs", "rust", source);
    let module_start = source.find("pub mod api").expect("module start");
    let module_end = source.len();
    insert_symbol(
        &conn,
        "src/lib.rs",
        "api",
        "crate::api",
        "module",
        module_start,
        module_end,
    );
    let handler_start = source.find("pub fn handler").expect("handler start");
    let handler_end = source[handler_start..]
        .find("\n    }\n")
        .map(|offset| handler_start + offset + "\n    }".len())
        .expect("handler end");
    let handler_id = insert_symbol(
        &conn,
        "src/lib.rs",
        "handler",
        "crate::api::handler",
        "function",
        handler_start,
        handler_end,
    );
    insert_command(
        &conn,
        "serve",
        "src/lib.rs",
        handler_start,
        Some(handler_id),
    );

    let symbol = graph
        .show(
            &"symbol:src/lib.rs#handler:function"
                .parse()
                .expect("symbol selector"),
            DEFAULT_SHOW_MAX_BYTES,
        )
        .expect("show symbol")
        .expect("symbol resolves");
    assert_eq!(symbol.metadata.file, "src/lib.rs");
    assert_eq!(symbol.metadata.kind, "function");
    assert_eq!(symbol.metadata.name.as_deref(), Some("handler"));
    assert_eq!(
        symbol.metadata.qualified.as_deref(),
        Some("crate::api::handler")
    );
    assert_eq!(
        &symbol.bytes,
        &source.as_bytes()[handler_start..handler_end]
    );

    let file = graph
        .show(
            &"file:src/lib.rs".parse().expect("file selector"),
            DEFAULT_SHOW_MAX_BYTES,
        )
        .expect("show file")
        .expect("file resolves");
    assert_eq!(file.metadata.kind, "file");
    assert_eq!(
        file.metadata.span,
        SourceSpan {
            start: 0,
            end: source.len()
        }
    );
    assert_eq!(file.bytes, source.as_bytes());

    let module = graph
        .show(
            &Selector::Module {
                qualified: "crate::api".to_string(),
            },
            DEFAULT_SHOW_MAX_BYTES,
        )
        .expect("show module")
        .expect("module resolves");
    assert_eq!(module.metadata.kind, "module");
    assert_eq!(module.metadata.qualified.as_deref(), Some("crate::api"));
    assert_eq!(&module.bytes, &source.as_bytes()[module_start..module_end]);

    let command = graph
        .show(
            &Selector::Command {
                name: "serve".to_string(),
            },
            DEFAULT_SHOW_MAX_BYTES,
        )
        .expect("show command")
        .expect("command resolves");
    assert_eq!(command.metadata.kind, "command");
    assert_eq!(command.metadata.name.as_deref(), Some("serve"));
    assert_eq!(
        command.metadata.qualified.as_deref(),
        Some("crate::api::handler")
    );
    assert_eq!(
        &command.bytes,
        &source.as_bytes()[handler_start..handler_end]
    );
}

#[test]
fn show_truncates_source_when_max_bytes_is_shorter_than_span() {
    let worktree = TestWorktree::new("show-truncate");
    let source = "pub fn long_body() {\n    let message = \"abcdef\";\n}\n";
    worktree.write("src/lib.rs", source);
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    insert_file(&conn, "src/lib.rs", "rust", source);
    insert_symbol(
        &conn,
        "src/lib.rs",
        "long_body",
        "crate::long_body",
        "function",
        0,
        source.len(),
    );

    let view = graph
        .show(
            &"symbol:src/lib.rs#long_body:function"
                .parse()
                .expect("selector"),
            7,
        )
        .expect("show symbol")
        .expect("symbol resolves");

    assert_eq!(view.bytes, b"pub fn ");
    assert_eq!(
        view.metadata.span,
        SourceSpan {
            start: 0,
            end: source.len()
        }
    );
    assert!(view.metadata.truncated);
}

#[test]
fn show_serializes_truncated_utf8_source_on_char_boundary() {
    let worktree = TestWorktree::new("show-truncate-utf8");
    let source = "pub fn cafe() -> &'static str { \"cafeé\" }\n";
    worktree.write("src/lib.rs", source);
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    insert_file(&conn, "src/lib.rs", "rust", source);
    insert_symbol(
        &conn,
        "src/lib.rs",
        "cafe",
        "crate::cafe",
        "function",
        0,
        source.len(),
    );
    let max_bytes = source.find("é").expect("accent byte") + 1;

    let view = graph
        .show(
            &"symbol:src/lib.rs#cafe:function".parse().expect("selector"),
            max_bytes,
        )
        .expect("show symbol")
        .expect("symbol resolves");
    let json = serde_json::to_value(&view).expect("serialize view");

    assert!(
        json["source"]
            .as_str()
            .expect("source string")
            .ends_with("cafe")
    );
    assert!(json.get("bytes").is_none());
    assert!(view.metadata.truncated);
}

#[test]
fn show_serializes_non_utf8_source_with_labeled_byte_fallback() {
    let worktree = TestWorktree::new("show-non-utf8");
    let source = b"valid prefix \xFF invalid\n";
    let path = worktree.path().join("src/lib.bin");
    fs::create_dir_all(path.parent().expect("source parent")).expect("create source parent");
    fs::write(path.as_path(), source).expect("write non-utf8 source");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    conn.execute(
        "INSERT INTO files (path, content_hash, mtime_ns, lang, byte_len, extracted_at)
         VALUES (?1, x'00', 1, ?2, ?3, 2)",
        params![
            "src/lib.bin",
            "binary",
            i64::try_from(source.len()).expect("source length fits")
        ],
    )
    .expect("insert non-utf8 file row");

    let view = graph
        .show(
            &"file:src/lib.bin".parse().expect("file selector"),
            DEFAULT_SHOW_MAX_BYTES,
        )
        .expect("show file")
        .expect("file resolves");
    let json = serde_json::to_value(&view).expect("serialize view");

    assert_eq!(
        json["source"],
        json!({
            "encoding": "bytes",
            "bytes": source
        })
    );
    assert!(json.get("bytes").is_none());
}

#[test]
fn show_missing_selector_returns_none() {
    let worktree = TestWorktree::new("show-missing");
    let source = "pub fn present() {}\n";
    worktree.write("src/lib.rs", source);
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    insert_file(&conn, "src/lib.rs", "rust", source);

    let missing = graph
        .show(
            &"symbol:src/lib.rs#missing:function"
                .parse()
                .expect("selector"),
            DEFAULT_SHOW_MAX_BYTES,
        )
        .expect("show missing");

    assert!(missing.is_none());
}

#[test]
fn show_calls_ensure_synced_at_entry() {
    let worktree = TestWorktree::new("show-ensure");
    worktree.write("src/lib.rs", "pub fn auto_sync_show() {}\n");
    let graph = open_graph(&worktree, SyncPolicy::OnRead);
    let db_path = graph_db_path(&worktree);

    let view = graph
        .show(
            &"file:src/lib.rs".parse().expect("file selector"),
            DEFAULT_SHOW_MAX_BYTES,
        )
        .expect("show with on-read sync")
        .expect("file resolves");

    assert_eq!(sync_leader_count(db_path.as_path()), 1);
    assert_eq!(view.metadata.file, "src/lib.rs");
}

#[test]
fn show_unresolved_dir_selector_returns_none() {
    let worktree = TestWorktree::new("show-dir");
    let graph = open_graph(&worktree, SyncPolicy::Manual);

    let view = graph
        .show(
            &"dir:src".parse().expect("dir selector"),
            DEFAULT_SHOW_MAX_BYTES,
        )
        .expect("show dir");

    assert!(view.is_none());
}

fn insert_command(
    conn: &Connection,
    name: &str,
    file_path: &str,
    span_start: usize,
    handler_symbol: Option<i64>,
) {
    conn.execute(
        "INSERT INTO commands (name, file_path, span_start, handler_symbol)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            name,
            file_path,
            i64::try_from(span_start).expect("command span fits"),
            handler_symbol
        ],
    )
    .expect("insert command row");
}
