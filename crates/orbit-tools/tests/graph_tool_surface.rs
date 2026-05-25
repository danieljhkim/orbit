#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::Command;

use orbit_tools::{ToolContext, ToolRegistry};
use serde_json::{Value, json};
use tempfile::TempDir;

#[test]
fn graph_tools_expose_orbit_graph_query_surface() {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();

    for name in [
        "orbit.graph.sync",
        "orbit.graph.search",
        "orbit.graph.show",
        "orbit.graph.refs",
        "orbit.graph.callees",
        "orbit.graph.impact",
        "orbit.graph.trace",
    ] {
        assert!(registry.is_active(name), "{name} should be active");
    }

    for removed in [
        "orbit.graph.callers",
        "orbit.graph.deps",
        "orbit.graph.implementors",
        "orbit.graph.overview",
        "orbit.graph.pack",
    ] {
        assert!(!registry.has(removed), "{removed} should not be registered");
    }

    let show_schema = registry
        .schemas()
        .into_iter()
        .find(|schema| schema.name == "orbit.graph.show")
        .expect("show schema");
    assert!(show_schema.description.contains("`text`"));
    assert!(show_schema.description.contains("fallback `bytes`"));
}

#[test]
fn graph_tools_query_synced_worktree() {
    let worktree = fixture_worktree();

    let sync = execute_graph_tool(
        worktree.path(),
        "orbit.graph.sync",
        json!({
            "full": true
        }),
    );
    assert!(sync["files_indexed"].as_u64().expect("files_indexed") >= 1);
    assert!(sync.get("duration_ms").is_some());

    let search = execute_graph_tool(
        worktree.path(),
        "orbit.graph.search",
        json!({
            "query": "helper",
            "kind": "symbol",
            "limit": 5
        }),
    );
    assert_array_field(&search, "matches");

    let show = execute_graph_tool(
        worktree.path(),
        "orbit.graph.show",
        json!({
            "selector": "symbol:src/lib.rs#entry:function",
            "max_bytes": 256
        }),
    );
    assert_eq!(show["metadata"]["file"], "src/lib.rs");
    assert_eq!(
        show["text"].as_str().expect("show text"),
        expected_entry_source()
    );
    assert!(show.get("bytes").is_none());

    let refs = execute_graph_tool(
        worktree.path(),
        "orbit.graph.refs",
        json!({
            "symbol": "symbol:src/lib.rs#helper:function",
            "confidence": "fuzzy",
            "kind": "call"
        }),
    );
    assert!(refs.get("target").is_some());
    assert_array_field(&refs, "refs");
    assert_array_field(&refs, "relations");

    let callees = execute_graph_tool(
        worktree.path(),
        "orbit.graph.callees",
        json!({
            "symbol": "symbol:src/lib.rs#entry:function"
        }),
    );
    assert_array_field(&callees, "callees");

    let impact = execute_graph_tool(
        worktree.path(),
        "orbit.graph.impact",
        json!({
            "selector": "symbol:src/lib.rs#entry:function",
            "depth": 2,
            "confidence": "same_module"
        }),
    );
    assert_array_field(&impact, "touched");
    assert!(impact.get("visited_nodes").is_some());

    let trace = execute_graph_tool(
        worktree.path(),
        "orbit.graph.trace",
        json!({
            "command_name": "missing-command",
            "depth": 2,
            "confidence": "same_module"
        }),
    );
    assert!(trace["root"].is_null());
    assert_eq!(trace["visited_nodes"], 0);
}

fn execute_graph_tool(workspace_root: &Path, name: &str, input: Value) -> Value {
    let mut registry = ToolRegistry::new();
    registry.register_builtins();
    let ctx = ToolContext {
        workspace_root: Some(workspace_root.to_path_buf()),
        ..ToolContext::default()
    };
    registry
        .execute(name, &ctx, input)
        .unwrap_or_else(|error| panic!("{name} failed: {error}"))
}

fn assert_array_field(value: &Value, field: &str) {
    assert!(
        value.get(field).and_then(Value::as_array).is_some(),
        "{field} should be an array in {value}"
    );
}

fn expected_entry_source() -> &'static str {
    "pub fn entry() -> i32 {\n    helper()\n}"
}

fn fixture_worktree() -> TempDir {
    let tempdir = TempDir::new().expect("temp worktree");
    run_git(tempdir.path(), ["init", "-b", "main"]);
    run_git(
        tempdir.path(),
        ["config", "user.email", "orbit@example.invalid"],
    );
    run_git(tempdir.path(), ["config", "user.name", "Orbit Test"]);

    fs::create_dir_all(tempdir.path().join("src")).expect("create src");
    fs::write(
        tempdir.path().join("src/lib.rs"),
        r#"
pub fn helper() -> i32 {
    1
}

pub fn entry() -> i32 {
    helper()
}

pub fn caller() -> i32 {
    entry()
}
"#,
    )
    .expect("write fixture");
    fs::write(
        tempdir.path().join("Cargo.toml"),
        "[package]\nname = \"graph_tool_fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    )
    .expect("write manifest");

    run_git(tempdir.path(), ["add", "."]);
    run_git(tempdir.path(), ["commit", "-m", "fixture"]);
    tempdir
}

fn run_git<const N: usize>(worktree: &Path, args: [&str; N]) {
    let output = Command::new("git")
        .current_dir(worktree)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
