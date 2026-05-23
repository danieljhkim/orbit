#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::Output;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

#[test]
fn task_cli_roundtrips_filters_and_replaces_tags() {
    let workspace = TestWorkspace::new();
    let perf = workspace.add_task("Perf task", &["perf"]);
    workspace.add_task("Bench task", &["bench"]);
    let both = workspace.add_task("Perf bench task", &["  Perf ", "BENCH"]);

    assert_eq!(both["tags"], json!(["perf", "bench"]));

    let perf_list = workspace.run(
        &["task", "list", "--all", "--tag", "perf", "--json"],
        None,
        "list perf tasks",
    );
    assert_task_titles(&perf_list, &["Perf task", "Perf bench task"]);

    // ORB-00202: `orbit task search` was deleted; the substring+tag case
    // migrates to `orbit search --kind task --tag <...>`. Results land
    // under `output["results"]` rather than at the top level.
    let both_search = workspace.run(
        &[
            "search",
            "tag-search",
            "--kind",
            "task",
            "--tag",
            "perf",
            "--tag",
            "bench",
            "--json",
        ],
        None,
        "orbit search perf+bench tasks",
    );
    assert_orbit_search_titles(&both_search, &["Perf bench task"]);

    let perf_id = perf["id"].as_str().expect("perf task id");
    let updated = workspace.run(
        &["task", "update", perf_id, "--tag", "docs", "--json"],
        None,
        "replace tags",
    );
    let updated: Value = serde_json::from_slice(&updated.stdout).expect("update JSON");
    assert_eq!(updated["tags"], json!(["docs"]));
}

fn assert_task_titles(output: &Output, expected: &[&str]) {
    let tasks: Value = serde_json::from_slice(&output.stdout).expect("task array JSON");
    let mut titles = tasks
        .as_array()
        .expect("task array")
        .iter()
        .map(|task| task["title"].as_str().expect("task title").to_string())
        .collect::<Vec<_>>();
    titles.sort();

    let mut expected = expected
        .iter()
        .map(|title| (*title).to_string())
        .collect::<Vec<_>>();
    expected.sort();

    assert_eq!(titles, expected);
}

fn assert_orbit_search_titles(output: &Output, expected: &[&str]) {
    let response: Value = serde_json::from_slice(&output.stdout).expect("search response JSON");
    let mut titles = response["results"]
        .as_array()
        .expect("results array")
        .iter()
        .map(|hit| hit["title"].as_str().expect("hit title").to_string())
        .collect::<Vec<_>>();
    titles.sort();

    let mut expected = expected
        .iter()
        .map(|title| (*title).to_string())
        .collect::<Vec<_>>();
    expected.sort();

    assert_eq!(titles, expected);
}

struct TestWorkspace {
    _temp: TempDir,
    home: std::path::PathBuf,
    work: std::path::PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let temp = tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let work = temp.path().join("work");
        fs::create_dir_all(&home).expect("create home");
        fs::create_dir_all(&work).expect("create work");

        let workspace = Self {
            _temp: temp,
            home,
            work,
        };
        workspace.run(
            &["workspace", "init", "--name", "task-tags-test"],
            None,
            "initialize workspace",
        );
        workspace
    }

    fn add_task(&self, title: &str, tags: &[&str]) -> Value {
        let mut args = vec![
            "task",
            "add",
            "--title",
            title,
            "--description",
            "Shared tag-search marker.",
            "--json",
        ];
        for tag in tags {
            args.push("--tag");
            args.push(tag);
        }
        let output = self.run(&args, None, "add tagged task");
        serde_json::from_slice(&output.stdout).expect("task add JSON")
    }

    fn run(&self, args: &[&str], stdin: Option<&str>, label: &str) -> Output {
        let output = run_orbit(&self.work, &self.home, args, stdin);
        assert!(
            output.status.success(),
            "{label} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
}

fn run_orbit(cwd: &Path, home: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut command = cargo_bin_cmd!("orbit");
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("ORBIT_ROOT")
        .args(args);
    if let Some(input) = stdin {
        command.write_stdin(input);
    }
    command.output().expect("run orbit")
}
