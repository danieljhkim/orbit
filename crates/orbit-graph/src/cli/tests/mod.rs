#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

use clap::Parser;
use serde_json::Value;
use tempfile::TempDir;

use super::{Cli, CommandContext};

/// Folded from the former `orbit-graph-cli` crate's `tests/subcommands.rs`.
///
/// The original test spawned the standalone `orbit-graph-cli` binary per
/// invocation (subprocess isolation meant each call could safely set its own
/// current directory). No binary exists anymore, so this drives the same
/// dispatch in-process via [`CommandContext::for_worktree`], which sidesteps
/// the process-wide current directory instead of relying on it.
#[test]
fn query_and_admin_subcommands_emit_json() {
    let worktree = fixture_worktree();
    let context = CommandContext::for_worktree(worktree.path().to_path_buf());

    let sync = run_json(&context, &["sync", "--full"]);
    assert_eq!(sync["files_removed"], 0);
    assert!(
        sync["files_indexed"]
            .as_u64()
            .is_some_and(|files_indexed| files_indexed >= 1),
        "files_indexed should be at least 1 in {sync}"
    );

    let search = run_json(
        &context,
        &["search", "helper", "--kind", "symbol", "--limit", "5"],
    );
    assert_array_field(&search, "matches");

    let show = run_json(
        &context,
        &[
            "show",
            "symbol:src/lib.rs#entry:function",
            "--max-bytes",
            "256",
        ],
    );
    assert_eq!(show["metadata"]["file"], "src/lib.rs");
    assert!(
        show["source"]
            .as_str()
            .is_some_and(|source| source.contains("pub fn entry")),
        "source should be UTF-8 text in {show}"
    );
    assert!(show.get("bytes").is_none());

    let refs = run_json(
        &context,
        &[
            "refs",
            "symbol:src/lib.rs#helper:function",
            "--confidence",
            "fuzzy",
            "--kind",
            "call",
        ],
    );
    assert!(refs.get("target").is_some());
    assert_array_field(&refs, "refs");
    assert_array_field(&refs, "relations");

    let callees = run_json(&context, &["callees", "symbol:src/lib.rs#entry:function"]);
    assert_array_field(&callees, "callees");

    let impact = run_json(
        &context,
        &[
            "impact",
            "symbol:src/lib.rs#entry:function",
            "--depth",
            "2",
            "--confidence",
            "same_module",
        ],
    );
    assert_array_field(&impact, "touched");
    assert!(impact.get("visited_nodes").is_some());

    let trace = run_json(
        &context,
        &[
            "trace",
            "missing-command",
            "--depth",
            "2",
            "--confidence",
            "same_module",
        ],
    );
    assert!(trace["root"].is_null());
    assert_eq!(trace["visited_nodes"], 0);

    let trace_selector = run_json(
        &context,
        &[
            "trace",
            "command:ship",
            "--depth",
            "2",
            "--confidence",
            "same_module",
        ],
    );
    assert!(trace_selector["root"].is_object());
    assert!(
        trace_selector["visited_nodes"]
            .as_u64()
            .is_some_and(|visited_nodes| visited_nodes > 0),
        "command selector should trace a non-empty tree in {trace_selector}"
    );

    let version = run_json(&context, &["version"]);
    assert_eq!(version["crate_version"], env!("CARGO_PKG_VERSION"));
    assert!(
        version["extractor_version"]
            .as_u64()
            .is_some_and(|extractor_version| extractor_version > 0),
        "extractor_version should be positive in {version}"
    );

    let db_path = run_json(&context, &["db-path"]);
    assert!(
        db_path["path"]
            .as_str()
            .is_some_and(|path| path.ends_with(".db")),
        "db path should end with .db in {db_path}"
    );
    assert!(db_path.get("branch").is_some());

    let graph_dir = worktree.path().join(".orbit").join("graph");
    fs::create_dir_all(&graph_dir).expect("create graph dir");
    let old_db = graph_dir.join("main.1.db");
    fs::write(&old_db, b"stale").expect("write stale db");
    let clean = run_json(&context, &["clean"]);
    assert!(
        clean["deleted"].as_array().is_some_and(|deleted| {
            deleted.iter().any(|path| {
                path.as_str()
                    .is_some_and(|path| path.ends_with("main.1.db"))
            })
        }),
        "deleted should include main.1.db in {clean}"
    );
    assert!(!old_db.exists());
}

#[test]
fn invalid_selector_errors_are_selector_parse_errors() {
    let worktree = fixture_worktree();
    let context = CommandContext::for_worktree(worktree.path().to_path_buf());

    let cli = Cli::try_parse_from(["orbit-graph-cli", "show", "not-a-selector"])
        .expect("parse show command");
    let error = cli
        .command
        .run_with_context(&context)
        .expect_err("not-a-selector should fail to parse");
    assert_eq!(error.code(), "selector_parse_error");
}

fn run_json(context: &CommandContext, args: &[&str]) -> Value {
    let cli = Cli::try_parse_from(std::iter::once("orbit-graph-cli").chain(args.iter().copied()))
        .unwrap_or_else(|error| panic!("parse {args:?}: {error}"));
    cli.command
        .run_with_context(context)
        .unwrap_or_else(|error| panic!("run {args:?}: {error}"))
}

fn assert_array_field(value: &Value, field: &str) {
    assert!(
        value.get(field).and_then(Value::as_array).is_some(),
        "{field} should be an array in {value}"
    );
}

fn fixture_worktree() -> TempDir {
    let tempdir = TempDir::new().expect("create tempdir");
    run_git(tempdir.path(), ["init", "-b", "main"]);
    run_git(
        tempdir.path(),
        ["config", "user.email", "orbit@example.invalid"],
    );
    run_git(tempdir.path(), ["config", "user.name", "Orbit Test"]);

    fs::create_dir_all(tempdir.path().join("src")).expect("create src dir");
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
    .expect("write lib.rs fixture");
    fs::write(
        tempdir.path().join("src/cli.py"),
        r#"
import click

@click.command()
def ship():
    helper()

def helper():
    return "ok"
"#,
    )
    .expect("write cli.py fixture");
    fs::write(
        tempdir.path().join("Cargo.toml"),
        "[package]\nname = \"graph_cli_fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml fixture");

    run_git(tempdir.path(), ["add", "."]);
    run_git(tempdir.path(), ["commit", "-m", "fixture"]);
    tempdir
}

fn run_git<const N: usize>(worktree: &Path, args: [&str; N]) {
    let output = StdCommand::new("git")
        .current_dir(worktree)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
