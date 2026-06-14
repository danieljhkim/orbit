#![allow(missing_docs)]

use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[test]
fn query_and_admin_subcommands_emit_json() -> TestResult {
    let worktree = fixture_worktree()?;

    let sync = run_json(worktree.path(), ["sync", "--full"])?;
    assert_eq!(sync["files_removed"], 0);
    assert!(
        sync["files_indexed"]
            .as_u64()
            .is_some_and(|files_indexed| files_indexed >= 1),
        "files_indexed should be at least 1 in {sync}"
    );

    let search = run_json(
        worktree.path(),
        ["search", "helper", "--kind", "symbol", "--limit", "5"],
    )?;
    assert_array_field(&search, "matches");

    let show = run_json(
        worktree.path(),
        [
            "show",
            "symbol:src/lib.rs#entry:function",
            "--max-bytes",
            "256",
        ],
    )?;
    assert_eq!(show["metadata"]["file"], "src/lib.rs");
    assert!(
        show["source"]
            .as_str()
            .is_some_and(|source| source.contains("pub fn entry")),
        "source should be UTF-8 text in {show}"
    );
    assert!(show.get("bytes").is_none());

    let refs = run_json(
        worktree.path(),
        [
            "refs",
            "symbol:src/lib.rs#helper:function",
            "--confidence",
            "fuzzy",
            "--kind",
            "call",
        ],
    )?;
    assert!(refs.get("target").is_some());
    assert_array_field(&refs, "refs");
    assert_array_field(&refs, "relations");

    let callees = run_json(
        worktree.path(),
        ["callees", "symbol:src/lib.rs#entry:function"],
    )?;
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
        ],
    )?;
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
        ],
    )?;
    assert!(trace["root"].is_null());
    assert_eq!(trace["visited_nodes"], 0);

    let version = run_json(worktree.path(), ["version"])?;
    assert_eq!(version["crate_version"], env!("CARGO_PKG_VERSION"));
    assert!(
        version["extractor_version"]
            .as_u64()
            .is_some_and(|extractor_version| extractor_version > 0),
        "extractor_version should be positive in {version}"
    );

    let db_path = run_json(worktree.path(), ["db-path"])?;
    assert!(
        db_path["path"]
            .as_str()
            .is_some_and(|path| path.ends_with(".db")),
        "db path should end with .db in {db_path}"
    );
    assert!(db_path.get("branch").is_some());

    let graph_dir = worktree.path().join(".orbit").join("graph");
    fs::create_dir_all(&graph_dir)?;
    let old_db = graph_dir.join("main.1.db");
    fs::write(&old_db, b"stale")?;
    let clean = run_json(worktree.path(), ["clean"])?;
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
    Ok(())
}

#[test]
fn invalid_selector_errors_are_json_on_stderr() -> TestResult {
    let worktree = fixture_worktree()?;
    let mut command = graph_cli_command();
    command
        .current_dir(worktree.path())
        .args(["show", "not-a-selector"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "\"code\":\"selector_parse_error\"",
        ));
    Ok(())
}

fn run_json<const N: usize>(worktree: &Path, args: [&str; N]) -> TestResult<Value> {
    let mut command = graph_cli_command();
    let output = command
        .current_dir(worktree)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    Ok(serde_json::from_slice(&output)?)
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

fn fixture_worktree() -> TestResult<TempDir> {
    let tempdir = TempDir::new()?;
    run_git(tempdir.path(), ["init", "-b", "main"])?;
    run_git(
        tempdir.path(),
        ["config", "user.email", "orbit@example.invalid"],
    )?;
    run_git(tempdir.path(), ["config", "user.name", "Orbit Test"])?;

    fs::create_dir_all(tempdir.path().join("src"))?;
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
    )?;
    fs::write(
        tempdir.path().join("Cargo.toml"),
        "[package]\nname = \"graph_cli_fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    )?;

    run_git(tempdir.path(), ["add", "."])?;
    run_git(tempdir.path(), ["commit", "-m", "fixture"])?;
    Ok(tempdir)
}

fn run_git<const N: usize>(worktree: &Path, args: [&str; N]) -> TestResult {
    let output = StdCommand::new("git")
        .current_dir(worktree)
        .args(args)
        .output()?;
    assert!(
        output.status.success(),
        "git failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
