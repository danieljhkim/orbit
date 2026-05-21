#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;
use std::process::Output;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

#[test]
fn cli_docs_list_and_show_json() {
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
}

#[test]
fn cli_orbit_search_federates_docs_and_adrs() {
    // ORB-00202: federated lexical search moved from `orbit docs search` to
    // `orbit search <query> --kind all` / `--kind doc` / `--kind adr`.
    let workspace = TestWorkspace::new();
    workspace.write(
        "docs/orbit-docs.md",
        "---\ntype: design\nsummary: Docs search context\ntags: [orbit-docs]\n---\n# Docs Search\n",
    );
    let adr_id = workspace.add_adr(
        "Federated ADR search",
        &["orbit-docs"],
        "## Context\nDocs search needs ADR metadata.\n\n## Decision\nKeep stores sibling and search both.\n\n## Consequences\n- Results carry origin tags.\n- Cost: docs search owns a small federation overlay.\n",
    );

    let results = workspace.run_json(
        &["search", "orbit-docs", "--limit", "5", "--json"],
        "orbit search federated",
    );
    let hits = results["results"].as_array().expect("results array");
    assert!(
        hits.iter()
            .any(|hit| hit["kind"] == "doc" && hit["path"] == "docs/orbit-docs.md"),
        "expected doc hit in {hits:?}"
    );
    assert!(
        hits.iter()
            .any(|hit| hit["kind"] == "adr" && hit["id"] == adr_id),
        "expected adr hit ({adr_id}) in {hits:?}"
    );
}

#[test]
fn cli_orbit_search_kind_adr_all_includes_superseded() {
    // ORB-00202: the old `orbit docs search --include-superseded` case is
    // covered by `orbit search --kind adr --all`.
    let workspace = TestWorkspace::new();
    let old_id = workspace.add_adr(
        "Archive policy old",
        &["archive-policy"],
        "## Context\nAn old archive decision existed.\n\n## Decision\nUse the old archive policy.\n\n## Consequences\n- Superseded records stay searchable only by opt-in.\n- Cost: archaeology requires an explicit flag.\n",
    );
    workspace.accept_adr(&old_id);
    let new_id = workspace.add_adr(
        "Archive policy replacement",
        &["archive-policy-current"],
        "## Context\nThe archive decision changed.\n\n## Decision\nUse the replacement archive policy.\n\n## Consequences\n- Current search should prefer active records.\n- Cost: the old record moves to superseded state.\n",
    );
    workspace.accept_adr(&new_id);
    workspace.supersede_adr(&old_id, &new_id);

    let default_results = workspace.run_json(
        &["search", "archive-policy", "--kind", "adr", "--json"],
        "orbit search adr default",
    );
    let default_hits = default_results["results"].as_array().expect("results");
    assert!(
        !default_hits.iter().any(|hit| hit["id"] == old_id),
        "default --kind adr must exclude superseded ADRs"
    );

    let widened = workspace.run_json(
        &[
            "search",
            "archive-policy",
            "--kind",
            "adr",
            "--all",
            "--json",
        ],
        "orbit search adr all",
    );
    let widened_hits = widened["results"].as_array().expect("results");
    assert!(
        widened_hits
            .iter()
            .any(|hit| hit["id"] == old_id && hit["status"] == "superseded"),
        "--kind adr --all should surface the superseded record"
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
fn cli_task_show_with_context_includes_related_docs_json() {
    let workspace = TestWorkspace::new();
    workspace.write("crates/orbit-cli/src/command/docs.rs", "// fixture\n");
    workspace.write(
        "docs/cli.md",
        "---\ntype: design\nsummary: CLI docs command design\npaths: [\"crates/orbit-cli/**\"]\n---\n# CLI Docs\n\nBody\n",
    );

    let task = workspace.run_json(
        &[
            "task",
            "add",
            "--title",
            "Wire docs",
            "--description",
            "Exercise docs context injection.",
            "--context",
            "file:crates/orbit-cli/src/command/docs.rs",
            "--json",
        ],
        "task add",
    );
    let task_id = task["id"].as_str().expect("task id");

    let shown = workspace.run_json(
        &[
            "task",
            "show",
            task_id,
            "--with-context",
            "--max-docs",
            "1",
            "--json",
        ],
        "task show with context",
    );
    assert_eq!(
        shown["related_docs"],
        json!([
            {
                "path": "docs/cli.md",
                "type": "design",
                "summary": "CLI docs command design",
                "excerpt": "CLI Docs",
                "matched_by": ["path:crates/orbit-cli/**"]
            }
        ])
    );

    let plain = workspace.run_json(&["task", "show", task_id, "--json"], "task show");
    assert!(plain.get("related_docs").is_none());
}

#[test]
fn cli_task_show_with_context_returns_empty_docs_when_roots_are_empty() {
    let workspace = TestWorkspace::new();
    workspace.write(".orbit/config.toml", "[docs]\nroots = []\n");
    workspace.write("crates/orbit-cli/src/command/docs.rs", "// fixture\n");
    workspace.write(
        "docs/cli.md",
        "---\ntype: design\nsummary: CLI docs command design\npaths: [\"crates/orbit-cli/**\"]\n---\n# CLI Docs\n",
    );
    let task = workspace.run_json(
        &[
            "task",
            "add",
            "--title",
            "No roots",
            "--description",
            "Exercise empty docs roots.",
            "--context",
            "file:crates/orbit-cli/src/command/docs.rs",
            "--json",
        ],
        "task add",
    );
    let task_id = task["id"].as_str().expect("task id");

    let shown = workspace.run_json(
        &["task", "show", task_id, "--with-context", "--json"],
        "task show with context",
    );

    assert_eq!(shown["related_docs"], json!([]));
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
        "orbit.docs.add",
        "orbit.docs.index",
        "orbit.docs.migrate",
    ] {
        assert!(names.contains(&name), "missing docs tool {name}");
    }
    // ORB-00202: `orbit.docs.search` deleted in phase 2.
    assert!(
        !names.contains(&"orbit.docs.search"),
        "orbit.docs.search must be deleted in phase 2"
    );

    let output = workspace.run_json(
        &["tool", "run", "orbit.docs.list", "--input", "{}"],
        "tool run docs list",
    );
    assert!(!output.as_array().expect("array").is_empty());
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

    fn tool_run(&self, tool: &str, input: Value, label: &str) -> Value {
        let input = serde_json::to_string(&input).expect("serialize tool input");
        self.run_json(&["tool", "run", tool, "--input", &input], label)
    }

    fn add_adr(&self, title: &str, related_features: &[&str], body: &str) -> String {
        let adr = self.tool_run(
            "orbit.adr.add",
            json!({
                "title": title,
                "body": body,
                "owner": "codex",
                "related_features": related_features,
            }),
            "add adr",
        );
        adr["id"].as_str().expect("adr id").to_string()
    }

    fn accept_adr(&self, id: &str) {
        self.tool_run(
            "orbit.adr.update",
            json!({
                "id": id,
                "status": "accepted",
                "related_tasks": ["ORB-00001"],
            }),
            "accept adr",
        );
    }

    fn supersede_adr(&self, old_id: &str, new_id: &str) {
        self.tool_run(
            "orbit.adr.supersede",
            json!({
                "old_id": old_id,
                "new_id": new_id,
            }),
            "supersede adr",
        );
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
