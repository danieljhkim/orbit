use rusqlite::{Connection, params};

use crate::query::tests::support::{
    TestWorktree, graph_db_path, insert_file, insert_symbol, open_connection, open_graph,
};
use crate::sync::sync_leader_count;
use crate::{RefConfidence, SyncMode, SyncPolicy, TRACE_NODE_CAP, TraceNode, TraceResult};

#[test]
fn trace_result_shape_matches_golden_fixture() {
    let result = TraceResult {
        root: Some(TraceNode {
            name: "handler".to_string(),
            qualified_name: Some("crate::handler".to_string()),
            confidence: None,
            children: vec![TraceNode {
                name: "helper".to_string(),
                qualified_name: Some("crate::helper".to_string()),
                confidence: Some("exact".to_string()),
                children: Vec::new(),
            }],
        }),
        truncated: false,
        visited_nodes: 2,
    };

    crate::query::tests::support::assert_json_matches_fixture(
        &result,
        include_str!("trace.golden.json"),
    );
}

#[test]
fn synthetic_command_with_three_level_call_tree_returns_full_tree() {
    let worktree = TestWorktree::new("trace-tree");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);

    let root_id = seed_symbol(&conn, "src/root.rs", "handler", "crate::handler");
    seed_symbol(&conn, "src/a.rs", "a", "crate::a");
    seed_symbol(&conn, "src/b.rs", "b", "crate::b");
    seed_symbol(&conn, "src/c.rs", "c", "crate::c");
    seed_symbol(&conn, "src/d.rs", "d", "crate::d");
    insert_command(&conn, "job-run", "src/root.rs", root_id);
    insert_call_ref(&conn, "src/root.rs", 10, 11, "a", "crate::a");
    insert_call_ref(&conn, "src/root.rs", 20, 21, "b", "crate::b");
    insert_call_ref(&conn, "src/a.rs", 10, 11, "c", "crate::c");
    insert_call_ref(&conn, "src/c.rs", 10, 11, "d", "crate::d");

    let result = graph
        .trace("job-run", 0, RefConfidence::SameModule)
        .expect("trace defaults to depth five");

    assert_eq!(result.visited_nodes, 5);
    assert!(!result.truncated);
    let root = result.root.expect("trace root");
    assert_eq!(root.name, "handler");
    assert_eq!(root.qualified_name.as_deref(), Some("crate::handler"));
    assert_eq!(child_names(&root), vec!["a", "b"]);

    let a = child(&root, "a");
    assert_eq!(child_names(a), vec!["c"]);
    let c = child(a, "c");
    assert_eq!(child_names(c), vec!["d"]);
    assert!(child(&root, "b").children.is_empty());
    assert!(child(c, "d").children.is_empty());
}

#[test]
fn command_selector_prefix_resolves_like_bare_command_name() {
    let worktree = TestWorktree::new("trace-command-selector");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);

    let root_id = seed_symbol(&conn, "src/root.rs", "handler", "crate::handler");
    insert_command(&conn, "job-run", "src/root.rs", root_id);

    let result = graph
        .trace("command:job-run", 5, RefConfidence::SameModule)
        .expect("trace command selector");

    assert_eq!(result.visited_nodes, 1);
    let root = result.root.expect("trace root");
    assert_eq!(root.qualified_name.as_deref(), Some("crate::handler"));
}

#[test]
fn branching_factor_five_depth_five_caps_at_200_nodes() {
    let worktree = TestWorktree::new("trace-cap");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    seed_branching_command(&conn, 5, 5);

    let result = graph
        .trace("wide-command", 5, RefConfidence::SameModule)
        .expect("trace branching graph");

    assert_eq!(result.visited_nodes, TRACE_NODE_CAP);
    assert_eq!(result.root.as_ref().map(count_nodes), Some(TRACE_NODE_CAP));
    assert!(result.truncated);
}

#[test]
fn unknown_command_returns_empty_trace_result() {
    let worktree = TestWorktree::new("trace-missing");
    let graph = open_graph(&worktree, SyncPolicy::Manual);

    let result = graph
        .trace("missing-command", 5, RefConfidence::SameModule)
        .expect("trace missing command");

    assert_eq!(result.visited_nodes, 0);
    assert!(!result.truncated);
    assert!(result.root.is_none());
}

#[test]
fn trace_resolves_python_click_command_from_synced_fixture() {
    let worktree = TestWorktree::new("trace-click-command");
    worktree.write(
        "src/cli.py",
        r#"
import click

@click.command()
def ship():
    helper()

def helper():
    leaf()

def leaf():
    return "done"
"#,
    );
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    graph.sync(SyncMode::Full).expect("sync click fixture");

    let result = graph
        .trace("ship", 3, RefConfidence::SameModule)
        .expect("trace click command");

    assert!(result.visited_nodes > 0);
    let root = result.root.expect("trace root");
    assert_eq!(root.name, "ship");
    assert_eq!(root.qualified_name.as_deref(), Some("ship"));
    assert_eq!(child_names(&root), vec!["helper"]);

    let helper = child(&root, "helper");
    assert_eq!(helper.qualified_name.as_deref(), Some("helper"));
    assert_eq!(child_names(helper), vec!["leaf"]);
}

#[test]
fn trace_resolves_rust_clap_command_from_synced_fixture() {
    let worktree = TestWorktree::new("trace-rust-clap-command");
    worktree.write(
        "src/cli.rs",
        r#"
use clap::Subcommand;

#[derive(Subcommand)]
enum TaskSubcommand {
    Add(AddArgs),
}

struct AddArgs;

fn dispatch(command: TaskSubcommand) {
    match command {
        TaskSubcommand::Add(args) => add(args),
    }
}

fn add(_args: AddArgs) {
    helper();
}

fn helper() {}
"#,
    );
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    graph.sync(SyncMode::Full).expect("sync rust clap fixture");

    let result = graph
        .trace("task add", 3, RefConfidence::SameModule)
        .expect("trace rust clap command");

    assert!(result.visited_nodes > 0);
    let root = result.root.expect("trace root");
    assert_eq!(root.name, "add");
    assert_eq!(root.qualified_name.as_deref(), Some("add"));
    assert_eq!(child_names(&root), vec!["helper"]);
}

#[test]
fn trace_resolves_rust_clap_handler_from_later_file_after_full_sync() {
    let worktree = TestWorktree::new("trace-rust-cross-file-clap-handler");
    worktree.write(
        "src/command.rs",
        r#"
use clap::Subcommand;

#[derive(Subcommand)]
enum AuditSubcommand {
    List(AuditListArgs),
}

struct AuditListArgs;

fn dispatch(command: AuditSubcommand) {
    match command {
        AuditSubcommand::List(args) => args.execute(),
    }
}
"#,
    );
    worktree.write(
        "src/handler.rs",
        r#"
struct AuditListArgs;
trait Execute {
    fn execute(self);
}

impl Execute for AuditListArgs {
    fn execute(self) {
        helper();
    }
}

fn helper() {}
"#,
    );
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    graph.sync(SyncMode::Full).expect("sync rust clap fixture");

    let result = graph
        .trace("audit list", 3, RefConfidence::SameModule)
        .expect("trace rust cross-file clap command");

    assert!(result.visited_nodes > 0);
    let root = result.root.expect("trace root");
    assert_eq!(root.name, "execute");
    assert_eq!(
        root.qualified_name.as_deref(),
        Some("<AuditListArgs as Execute>::execute")
    );
    assert_eq!(child_names(&root), vec!["helper"]);
}

#[test]
fn trace_calls_ensure_synced_at_entry() {
    let worktree = TestWorktree::new("trace-ensure-synced");
    worktree.write("src/lib.rs", "pub fn synced_trace_marker() {}\n");
    let graph = open_graph(&worktree, SyncPolicy::OnRead);
    let db_path = graph_db_path(&worktree);

    let result = graph
        .trace("missing-after-sync", 5, RefConfidence::SameModule)
        .expect("trace triggers sync");

    assert_eq!(sync_leader_count(db_path.as_path()), 1);
    assert!(result.root.is_none());
}

#[test]
fn confidence_floor_filters_and_prevents_below_floor_expansion() {
    let worktree = TestWorktree::new("trace-confidence-floor");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);

    let root_id = seed_symbol(&conn, "src/root.rs", "handler", "crate::handler");
    seed_symbol(&conn, "src/exact.rs", "exact", "crate::exact");
    seed_symbol(&conn, "src/same.rs", "same", "crate::same_module");
    seed_symbol(&conn, "src/leaf.rs", "leaf", "crate::leaf");
    seed_symbol(&conn, "src/fuzzy.rs", "fuzzy", "crate::fuzzy");
    insert_command(&conn, "job-run", "src/root.rs", root_id);
    insert_call_ref_with_confidence(
        &conn,
        "src/root.rs",
        10,
        11,
        "exact",
        "crate::exact",
        "exact",
    );
    insert_call_ref_with_confidence(
        &conn,
        "src/root.rs",
        20,
        21,
        "same",
        "crate::same_module",
        "same_module",
    );
    insert_call_ref_with_confidence(
        &conn,
        "src/root.rs",
        30,
        31,
        "fuzzy",
        "crate::fuzzy",
        "fuzzy_name",
    );
    insert_call_ref_with_confidence(&conn, "src/same.rs", 10, 11, "leaf", "crate::leaf", "exact");

    let default_floor = graph
        .trace("job-run", 2, RefConfidence::SameModule)
        .expect("trace default confidence floor");
    let default_root = default_floor.root.expect("default trace root");
    assert_eq!(child_names(&default_root), vec!["exact", "same"]);
    assert_eq!(child_names(child(&default_root, "same")), vec!["leaf"]);

    let exact_floor = graph
        .trace("job-run", 2, RefConfidence::Exact)
        .expect("trace exact confidence floor");
    let exact_root = exact_floor.root.expect("exact trace root");
    assert_eq!(child_names(&exact_root), vec!["exact"]);
}

fn seed_symbol(conn: &Connection, file_path: &str, name: &str, qualified: &str) -> i64 {
    let content = " ".repeat(100);
    insert_file(conn, file_path, "rust", content.as_str());
    insert_symbol(conn, file_path, name, qualified, "function", 0, 100)
}

fn insert_command(conn: &Connection, name: &str, file_path: &str, handler_symbol: i64) {
    conn.execute(
        "INSERT INTO commands (name, file_path, span_start, handler_symbol)
         VALUES (?1, ?2, 0, ?3)",
        params![name, file_path, handler_symbol],
    )
    .expect("insert command row");
}

fn insert_call_ref(
    conn: &Connection,
    from_file: &str,
    span_start: usize,
    span_end: usize,
    target_name: &str,
    target_qualified: &str,
) {
    insert_call_ref_with_confidence(
        conn,
        from_file,
        span_start,
        span_end,
        target_name,
        target_qualified,
        "exact",
    );
}

fn insert_call_ref_with_confidence(
    conn: &Connection,
    from_file: &str,
    span_start: usize,
    span_end: usize,
    target_name: &str,
    target_qualified: &str,
    confidence: &str,
) {
    conn.execute(
        "INSERT INTO refs (
            from_file, from_span_start, from_span_end, target_name, target_qualified,
            target_symbol_hint, kind, confidence
         ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, 'call', ?6)",
        params![
            from_file,
            i64::try_from(span_start).expect("span start fits"),
            i64::try_from(span_end).expect("span end fits"),
            target_name,
            target_qualified,
            confidence,
        ],
    )
    .expect("insert call ref");
}

fn seed_branching_command(conn: &Connection, branching_factor: usize, max_depth: usize) {
    let total_nodes = (0..=max_depth).fold(0usize, |sum, depth| {
        sum + branching_factor.pow(u32::try_from(depth).expect("depth fits"))
    });
    let content = " ".repeat(total_nodes * 20 + 20);
    insert_file(conn, "src/wide.rs", "rust", content.as_str());

    for index in 0..total_nodes {
        let span_start = index * 20;
        let name = node_name(index);
        let qualified = node_qualified(index);
        insert_symbol(
            conn,
            "src/wide.rs",
            name.as_str(),
            qualified.as_str(),
            "function",
            span_start,
            span_start + 10,
        );
    }

    insert_command(conn, "wide-command", "src/wide.rs", 1);

    let mut parent_level = vec![0usize];
    let mut next_index = 1usize;
    for _ in 0..max_depth {
        let mut next_level = Vec::new();
        for parent in parent_level {
            let parent_span_start = parent * 20;
            for branch in 0..branching_factor {
                let child_index = next_index;
                next_index += 1;
                next_level.push(child_index);
                insert_call_ref(
                    conn,
                    "src/wide.rs",
                    parent_span_start + branch + 1,
                    parent_span_start + branch + 2,
                    node_name(child_index).as_str(),
                    node_qualified(child_index).as_str(),
                );
            }
        }
        parent_level = next_level;
    }
}

fn node_name(index: usize) -> String {
    format!("node_{index:04}")
}

fn node_qualified(index: usize) -> String {
    format!("crate::{}", node_name(index))
}

fn child<'a>(node: &'a TraceNode, name: &str) -> &'a TraceNode {
    node.children
        .iter()
        .find(|child| child.name == name)
        .expect("child exists")
}

fn child_names(node: &TraceNode) -> Vec<&str> {
    node.children
        .iter()
        .map(|child| child.name.as_str())
        .collect()
}

fn count_nodes(node: &TraceNode) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}
