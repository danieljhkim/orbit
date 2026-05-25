use rusqlite::{Connection, params};

use crate::query::tests::support::{
    TestWorktree, graph_db_path, insert_file, insert_symbol, open_connection, open_graph,
};
use crate::sync::sync_leader_count;
use crate::{SyncPolicy, TRACE_NODE_CAP, TraceNode};

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
        .trace("job-run", 0)
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
fn branching_factor_five_depth_five_caps_at_200_nodes() {
    let worktree = TestWorktree::new("trace-cap");
    let graph = open_graph(&worktree, SyncPolicy::Manual);
    let conn = open_connection(&worktree);
    seed_branching_command(&conn, 5, 5);

    let result = graph
        .trace("wide-command", 5)
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
        .trace("missing-command", 5)
        .expect("trace missing command");

    assert_eq!(result.visited_nodes, 0);
    assert!(!result.truncated);
    assert!(result.root.is_none());
}

#[test]
fn trace_calls_ensure_synced_at_entry() {
    let worktree = TestWorktree::new("trace-ensure-synced");
    worktree.write("src/lib.rs", "pub fn synced_trace_marker() {}\n");
    let graph = open_graph(&worktree, SyncPolicy::OnRead);
    let db_path = graph_db_path(&worktree);

    let result = graph
        .trace("missing-after-sync", 5)
        .expect("trace triggers sync");

    assert_eq!(sync_leader_count(db_path.as_path()), 1);
    assert!(result.root.is_none());
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
    conn.execute(
        "INSERT INTO refs (
            from_file, from_span_start, from_span_end, target_name, target_qualified,
            target_symbol_hint, kind, confidence
         ) VALUES (?1, ?2, ?3, ?4, ?5, NULL, 'call', 'exact')",
        params![
            from_file,
            i64::try_from(span_start).expect("span start fits"),
            i64::try_from(span_end).expect("span end fits"),
            target_name,
            target_qualified,
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
