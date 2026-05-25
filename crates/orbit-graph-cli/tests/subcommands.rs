#![allow(missing_docs)]

use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn query_and_admin_subcommands_emit_json() {
    let worktree = fixture_worktree();

    let sync = run_json(worktree.path(), ["sync", "--full", "--backend", "new"]);
    assert_eq!(sync["files_removed"], 0);
    assert!(sync["files_indexed"].as_u64().expect("files_indexed") >= 1);

    let search = run_json(
        worktree.path(),
        [
            "search",
            "helper",
            "--kind",
            "symbol",
            "--limit",
            "5",
            "--backend",
            "new",
        ],
    );
    assert_array_field(&search, "matches");

    let show = run_json(
        worktree.path(),
        [
            "show",
            "symbol:src/lib.rs#entry:function",
            "--max-bytes",
            "256",
            "--backend",
            "new",
        ],
    );
    assert_eq!(show["metadata"]["file"], "src/lib.rs");
    assert_array_field(&show, "bytes");

    let refs = run_json(
        worktree.path(),
        [
            "refs",
            "symbol:src/lib.rs#helper:function",
            "--confidence",
            "fuzzy",
            "--kind",
            "call",
            "--backend",
            "new",
        ],
    );
    assert!(refs.get("target").is_some());
    assert_array_field(&refs, "refs");
    assert_array_field(&refs, "relations");

    let callees = run_json(
        worktree.path(),
        [
            "callees",
            "symbol:src/lib.rs#entry:function",
            "--backend",
            "new",
        ],
    );
    assert_array_field(&callees, "callees");

    let impact = run_json(
        worktree.path(),
        [
            "impact",
            "symbol:src/lib.rs#entry:function",
            "--depth",
            "2",
            "--confidence",
            "same_module",
            "--backend",
            "new",
        ],
    );
    assert_array_field(&impact, "touched");
    assert!(impact.get("visited_nodes").is_some());

    let trace = run_json(
        worktree.path(),
        [
            "trace",
            "missing-command",
            "--depth",
            "2",
            "--confidence",
            "same_module",
            "--backend",
            "new",
        ],
    );
    assert!(trace["root"].is_null());
    assert_eq!(trace["visited_nodes"], 0);

    let version = run_json(worktree.path(), ["version"]);
    assert_eq!(version["crate_version"], env!("CARGO_PKG_VERSION"));
    assert!(
        version["extractor_version"]
            .as_u64()
            .expect("extractor_version")
            > 0
    );

    let db_path = run_json(worktree.path(), ["db-path"]);
    assert!(
        db_path["path"]
            .as_str()
            .expect("db path string")
            .ends_with(".db")
    );
    assert!(db_path.get("branch").is_some());

    let graph_dir = worktree.path().join(".orbit").join("graph");
    fs::create_dir_all(&graph_dir).expect("create graph dir");
    let old_db = graph_dir.join("main.1.db");
    fs::write(&old_db, b"stale").expect("write stale db");
    let clean = run_json(worktree.path(), ["clean"]);
    let deleted = clean["deleted"].as_array().expect("deleted array");
    assert!(deleted.iter().any(|path| {
        path.as_str()
            .is_some_and(|path| path.ends_with("main.1.db"))
    }));
    assert!(!old_db.exists());
}

#[test]
fn invalid_selector_errors_are_json_on_stderr() {
    let worktree = fixture_worktree();
    let mut command = graph_cli_command();
    command
        .current_dir(worktree.path())
        .args(["show", "not-a-selector"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "\"code\":\"selector_parse_error\"",
        ));
}

#[test]
fn backend_cli_override_takes_precedence_over_env() {
    let worktree = fixture_worktree();
    let mut command = graph_cli_command();
    command
        .current_dir(worktree.path())
        .env("ORBIT_GRAPH_BACKEND", "legacy")
        .args(["sync", "--full", "--backend", "new"])
        .assert()
        .success();
}

#[test]
fn backend_env_selects_legacy_by_default() {
    let worktree = fixture_worktree();
    let mut command = graph_cli_command();
    command
        .current_dir(worktree.path())
        .env("ORBIT_GRAPH_BACKEND", "legacy")
        .args(["callees", "symbol:src/lib.rs#entry:function"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("\"code\":\"legacy_unavailable\""));
}

fn run_json<const N: usize>(worktree: &Path, args: [&str; N]) -> Value {
    let mut command = graph_cli_command();
    let output = command
        .current_dir(worktree)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&output).expect("stdout is JSON")
}

fn graph_cli_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_orbit-graph-cli"))
}

fn assert_array_field(value: &Value, field: &str) {
    assert!(
        value.get(field).and_then(Value::as_array).is_some(),
        "{field} should be an array in {value}"
    );
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
        "[package]\nname = \"graph_cli_fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    )
    .expect("write manifest");

    run_git(tempdir.path(), ["add", "."]);
    run_git(tempdir.path(), ["commit", "-m", "fixture"]);
    tempdir
}

fn run_git<const N: usize>(worktree: &Path, args: [&str; N]) {
    let output = StdCommand::new("git")
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
