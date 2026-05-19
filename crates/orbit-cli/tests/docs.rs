#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;
use std::process::Output;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

#[test]
fn cli_docs_list_show_and_search_json() {
    let workspace = TestWorkspace::new();
    workspace.write(
        "docs/pattern.md",
        "---\ntype: pattern\nsummary: RAII guard pattern\ntags: [rust, guard]\nrelated_artifacts: [ORB-00160]\n---\n# Guard\n\nBody\n",
    );
    workspace.write(".orbit/adrs/ADR-0001/body.md", "# Hidden ADR\n");

    let listed = workspace.run_json(&["docs", "list", "--json"], "docs list");
    let rows = listed.as_array().expect("array");
    assert!(rows.iter().any(|row| row["path"] == "docs/pattern.md"));
    assert!(
        rows.iter()
            .all(|row| { !row["path"].as_str().expect("path").starts_with(".orbit/") })
    );

    let shown = workspace.run_json(&["docs", "show", "docs/pattern.md", "--json"], "docs show");
    assert_eq!(shown["frontmatter"]["type"], "pattern");
    assert!(shown["body"].as_str().expect("body").contains("# Guard"));

    let results = workspace.run_json(
        &["docs", "search", "RAII", "--limit", "1", "--json"],
        "docs search",
    );
    assert_eq!(
        results,
        json!([
            {
                "path": "docs/pattern.md",
                "type": "pattern",
                "summary": "RAII guard pattern",
                "tags": ["rust", "guard"],
                "related_artifacts": ["ORB-00160"],
                "score": 84,
                "matched_by": ["summary"]
            }
        ])
    );
}

#[test]
fn cli_docs_add_is_idempotent_and_rejects_dot_orbit() {
    let workspace = TestWorkspace::new();
    fs::create_dir_all(workspace.work.join("extra-docs")).expect("extra docs");
    let first = workspace.run_json(&["docs", "add", "extra-docs", "--json"], "docs add");
    assert_eq!(first["added"], true);
    let second = workspace.run_json(&["docs", "add", "extra-docs", "--json"], "docs add again");
    assert_eq!(second["added"], false);

    let output = run_orbit(
        &workspace.work,
        &workspace.home,
        &["docs", "add", ".orbit", "--json"],
    );
    assert!(!output.status.success());
    let payload: Value = serde_json::from_slice(&output.stderr)
        .unwrap_or_else(|_| serde_json::from_slice(&output.stdout).expect("json error payload"));
    assert_eq!(payload["code"], "invalid_input");
}

#[test]
fn mcp_docs_tools_are_listed_and_callable_through_tool_run() {
    let workspace = TestWorkspace::new();
    workspace.write(
        "docs/context.md",
        "---\ntype: context\nsummary: Context document\n---\nBody\n",
    );

    let tools = workspace.run_json(&["tool", "list", "--json"], "tool list");
    let names = tools
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| tool["name"].as_str().expect("name"))
        .collect::<Vec<_>>();
    for name in [
        "orbit.docs.list",
        "orbit.docs.show",
        "orbit.docs.search",
        "orbit.docs.add",
        "orbit.docs.reindex",
        "orbit.docs.migrate",
    ] {
        assert!(names.contains(&name), "missing docs tool {name}");
    }

    let output = workspace.run_json(
        &["tool", "run", "orbit.docs.list", "--input", "{}"],
        "tool run docs list",
    );
    assert!(output.as_array().expect("array").len() >= 1);
}

struct TestWorkspace {
    _temp: TempDir,
    home: PathBuf,
    work: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let temp = tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let work = temp.path().join("work");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&work).expect("work");
        let workspace = Self {
            _temp: temp,
            home,
            work,
        };
        workspace.run(
            &["workspace", "init", "--name", "docs-cli-test"],
            "workspace init",
        );
        workspace
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.work.join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, content).expect("write file");
    }

    fn run(&self, args: &[&str], label: &str) -> Output {
        let output = run_orbit(&self.work, &self.home, args);
        assert!(
            output.status.success(),
            "{label} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn run_json(&self, args: &[&str], label: &str) -> Value {
        let output = self.run(args, label);
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "{label} produced invalid JSON: {error}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }
}

fn run_orbit(work: &PathBuf, home: &PathBuf, args: &[&str]) -> Output {
    let mut cmd = cargo_bin_cmd!("orbit");
    cmd.current_dir(work)
        .env("HOME", home)
        .env("ORBIT_HOME", home.join(".orbit-global"))
        .env_remove("ORBIT_ROOT")
        .args(args)
        .output()
        .expect("run orbit")
}
