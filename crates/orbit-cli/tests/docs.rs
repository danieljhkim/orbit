#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;
use std::process::Output;

use assert_cmd::cargo::cargo_bin_cmd;
use rusqlite::{Connection, params};
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
fn cli_orbit_search_limit_help_describes_total_round_robin_limit() {
    let workspace = TestWorkspace::new();

    let output = workspace.run(&["search", "--help"], "orbit search help");
    let help = String::from_utf8_lossy(&output.stdout);

    assert!(help.contains("Maximum total results returned"));
    assert!(help.contains("round-robin per kind"));
    assert!(help.contains("[default: 10]"));
    assert!(help.contains("ADRs use lexical matching regardless of --hybrid."));
    assert!(!help.contains("learnings and ADRs use lexical matching"));
}

#[test]
fn cli_orbit_search_path_notes_doc_branch_skip_in_json_and_table_modes() {
    let workspace = TestWorkspace::new();
    workspace.write(
        "docs/path-note.md",
        "---\ntype: context\nsummary: path note\n---\nBody\n",
    );

    let response = workspace.run_json(
        &["search", "path", "crates/orbit-cli/", "--json"],
        "orbit search path json",
    );
    let notes = response["notes"].as_array().expect("notes");
    assert!(
        notes.iter().any(|note| {
            let note = note.as_str().expect("note");
            note.contains("doc branch skipped") && note.contains("--path")
        }),
        "JSON notes should mention doc branch and --path: {notes:?}"
    );

    let output = workspace.run(
        &["search", "path", "crates/orbit-cli/"],
        "orbit search path table",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("note: ")
            && stderr.contains("doc branch skipped")
            && stderr.contains("--path"),
        "table-mode stderr should include prefixed note: {stderr}"
    );
}

#[test]
fn cli_orbit_search_hybrid_doc_json_reports_lexical_fallback_note() {
    let workspace = TestWorkspace::new();
    workspace.write(
        "docs/hybrid-note.md",
        "---\ntype: context\nsummary: hybrid-note\n---\nhybrid-note body\n",
    );

    let response = workspace.run_json(
        &[
            "search",
            "hybrid-note",
            "--hybrid",
            "--kind",
            "doc",
            "--json",
        ],
        "orbit search hybrid doc",
    );
    let notes = response["notes"].as_array().expect("notes");
    assert!(
        notes.iter().any(|note| {
            note.as_str()
                .expect("note")
                .contains("falling back to lexical doc search")
        }),
        "hybrid doc notes should preserve lexical fallback warning: {notes:?}"
    );
}

#[test]
fn cli_orbit_search_hybrid_learning_json_reports_lexical_fallback_note_missing_companion() {
    let workspace = TestWorkspace::new();
    let learning = workspace.run_json(
        &[
            "learning",
            "add",
            "--summary",
            "hybrid-learning-note literal",
            "--tag",
            "hybrid-learning-note",
            "--json",
        ],
        "add learning",
    );

    let response = workspace.run_json(
        &[
            "search",
            "hybrid-learning-note",
            "--hybrid",
            "--kind",
            "learning",
            "--json",
        ],
        "orbit search hybrid learning missing companion",
    );
    let notes = response["notes"].as_array().expect("notes");
    assert!(
        notes.iter().any(|note| {
            note.as_str()
                .expect("note")
                .contains("falling back to lexical")
        }),
        "hybrid learning notes should preserve lexical fallback warning: {notes:?}"
    );
    assert_eq!(response["results"][0]["source"], "lexical");
    assert_eq!(response["results"][0]["id"], learning["id"]);

    let output = workspace.run(
        &[
            "search",
            "hybrid-learning-note",
            "--hybrid",
            "--kind",
            "learning",
        ],
        "orbit search hybrid learning missing companion table",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("note: ") && stderr.contains("falling back to lexical"),
        "table-mode stderr should include fallback note: {stderr}"
    );
}

#[test]
#[cfg(unix)]
fn cli_orbit_search_hybrid_learning_json_reports_lexical_fallback_note_empty_embeddings() {
    let workspace = TestWorkspace::new();
    workspace.write_mock_companion();
    let learning = workspace.run_json(
        &[
            "learning",
            "add",
            "--summary",
            "hybrid-learning-empty literal",
            "--tag",
            "hybrid-learning-empty",
            "--json",
        ],
        "add learning",
    );

    let response = workspace.run_json_with_companion(
        &[
            "search",
            "hybrid-learning-empty",
            "--hybrid",
            "--kind",
            "learning",
            "--json",
        ],
        "orbit search hybrid learning empty embeddings",
    );
    let notes = response["notes"].as_array().expect("notes");
    assert!(
        notes.iter().any(|note| {
            note.as_str()
                .expect("note")
                .contains("falling back to lexical")
        }),
        "hybrid learning notes should preserve lexical fallback warning: {notes:?}"
    );
    assert_eq!(response["results"][0]["source"], "lexical");
    assert_eq!(response["results"][0]["id"], learning["id"]);
}

#[test]
#[cfg(unix)]
fn cli_orbit_search_hybrid_learning_ranking_differs_from_lexical() {
    let workspace = TestWorkspace::new();
    workspace.write_mock_companion();
    let semantic = workspace.run_json(
        &[
            "learning",
            "add",
            "--summary",
            "conceptual guidance",
            "--body",
            "semantic-target operational insight",
            "--json",
        ],
        "add semantic learning",
    );
    let literal = workspace.run_json(
        &[
            "learning",
            "add",
            "--summary",
            "foo literal guidance",
            "--body",
            "literal body",
            "--priority",
            "100",
            "--json",
        ],
        "add literal learning",
    );
    workspace.run_json_with_companion(
        &[
            "semantic",
            "index",
            "--kind",
            "learnings",
            "--force",
            "--json",
        ],
        "semantic index learnings",
    );

    let lexical = workspace.run_json(
        &["search", "foo", "--kind", "learning", "--json"],
        "learning lexical search",
    );
    let hybrid = workspace.run_json_with_companion(
        &["search", "foo", "--kind", "learning", "--hybrid", "--json"],
        "learning hybrid search",
    );

    assert_eq!(lexical["results"][0]["id"], literal["id"]);
    assert_eq!(hybrid["results"][0]["id"], semantic["id"]);
    assert_ne!(lexical["results"][0]["id"], hybrid["results"][0]["id"]);
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

#[test]
#[cfg(unix)]
fn cli_docs_index_is_semantic_docs_alias_for_json_output() {
    let workspace = TestWorkspace::new();
    workspace.write_mock_companion();
    workspace.write(
        "docs/context.md",
        "---\ntype: context\nsummary: Context document\ntags: [semantic]\n---\nBody\n",
    );

    let docs =
        workspace.run_json_with_companion(&["docs", "index", "--force", "--json"], "docs index");
    let semantic = workspace.run_json_with_companion(
        &["semantic", "index", "--kind", "docs", "--force", "--json"],
        "semantic index docs",
    );

    assert_eq!(semantic, docs);
}

#[test]
#[cfg(unix)]
fn cli_semantic_index_all_json_contains_tasks_and_docs() {
    let workspace = TestWorkspace::new();
    workspace.write_mock_companion();
    workspace.write(
        "docs/context.md",
        "---\ntype: context\nsummary: Context document\ntags: [semantic]\n---\nBody\n",
    );
    workspace.run_json(
        &[
            "task",
            "add",
            "--title",
            "Index all",
            "--description",
            "Exercise task indexing.",
            "--acceptance-criteria",
            "both corpora indexed",
            "--json",
        ],
        "task add",
    );

    let result = workspace.run_json_with_companion(
        &["semantic", "index", "--kind", "all", "--force", "--json"],
        "semantic index all",
    );

    assert_eq!(result["tasks"]["model_id"], "bge-small-en-v1.5");
    assert!(
        result["tasks"]["report"]["embedded_chunks"]
            .as_u64()
            .expect("task chunks")
            > 0
    );
    assert_eq!(result["docs"]["model_id"], "bge-small-en-v1.5");
    assert_eq!(result["docs"]["indexed_sources"], 1);
    assert_eq!(result["learnings"]["model_id"], "bge-small-en-v1.5");
    assert_eq!(result["learnings"]["indexed_sources"], 0);
}

#[test]
#[cfg(unix)]
fn cli_semantic_index_learnings_is_idempotent_status_agnostic_and_sweeps_stale() {
    let workspace = TestWorkspace::new();
    workspace.write_mock_companion();
    let old = workspace.run_json(
        &[
            "learning",
            "add",
            "--summary",
            "learning-index-old",
            "--body",
            "## Rule\nKeep the old rule indexed.\n\n## Why\nSuperseded records remain searchable when explicitly requested.\n",
            "--tag",
            "semantic",
            "--json",
        ],
        "add old learning",
    );
    let new = workspace.run_json(
        &[
            "learning",
            "add",
            "--summary",
            "learning-index-new",
            "--body",
            "## Rule\nReplacement rule.\n\n## How to apply\nUse the replacement.\n",
            "--tag",
            "semantic",
            "--json",
        ],
        "add new learning",
    );
    let old_id = old["id"].as_str().expect("old id");
    let new_id = new["id"].as_str().expect("new id");

    let first = workspace.run_json_with_companion(
        &["semantic", "index", "--kind", "learnings", "--json"],
        "semantic index learnings",
    );
    assert_eq!(first["indexed_sources"], 2);
    assert!(
        first["report"]["embedded_chunks"].as_u64().unwrap_or(0)
            > learning_dir_count(&workspace.work) as u64
    );
    assert!(
        count_learning_embeddings(&workspace.work, None)
            > learning_dir_count(&workspace.work) as i64
    );

    let second = workspace.run_json_with_companion(
        &["semantic", "index", "--kind", "learnings", "--json"],
        "semantic index learnings idempotent",
    );
    assert_eq!(second["report"]["embedded_chunks"], 0);
    assert!(second["report"]["skipped_fields"].as_u64().unwrap_or(0) > 0);

    workspace.run_json(
        &["learning", "supersede", old_id, "--with", new_id, "--json"],
        "supersede learning",
    );
    workspace.run_json_with_companion(
        &["semantic", "index", "--kind", "learnings", "--json"],
        "semantic index superseded learnings",
    );
    assert!(
        count_learning_embeddings(&workspace.work, Some(old_id)) > 0,
        "superseded learning should remain indexed"
    );

    fs::remove_dir_all(workspace.work.join(".orbit/learnings").join(new_id))
        .expect("delete learning directory");
    workspace.run_json_with_companion(
        &["semantic", "index", "--kind", "learnings", "--json"],
        "semantic index after learning deletion",
    );
    assert_eq!(count_learning_embeddings(&workspace.work, Some(new_id)), 0);
    assert!(count_learning_embeddings(&workspace.work, Some(old_id)) > 0);
}

#[test]
#[cfg(unix)]
fn cli_semantic_index_all_keeps_task_rows_when_docs_fail() {
    let workspace = TestWorkspace::new();
    workspace.write_mock_companion();
    workspace.run_json(
        &[
            "task",
            "add",
            "--title",
            "Partial progress",
            "--description",
            "Exercise all-kind resilience.",
            "--acceptance-criteria",
            "task rows persist",
            "--json",
        ],
        "task add",
    );
    workspace.write(
        "docs/broken.md",
        "---\ntype: context\nsummary: Broken doc\n---\nBody\n",
    );
    let broken = workspace.work.join("docs/broken.md");
    make_unreadable(&broken);

    let output = workspace.run_failure_with_companion(
        &["semantic", "index", "--kind", "all", "--json"],
        "semantic index all with unreadable docs",
    );
    restore_readable(&broken);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("read"),
        "stderr should explain docs read failure: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stats =
        workspace.run_json_with_companion(&["semantic", "stats", "--json"], "semantic stats");
    let rows = stats["rows"]["counts"].as_array().expect("counts");
    assert!(
        rows.iter()
            .any(|row| row["source_kind"] == "task" && row["rows"].as_u64().unwrap_or(0) > 0),
        "task rows should be present after docs failure: {rows:?}"
    );
}

struct TestWorkspace {
    _temp: TempDir,
    home: PathBuf,
    work: PathBuf,
    companion: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let temp = tempdir().expect("tempdir");
        let home = temp.path().join("home");
        let work = temp.path().join("work");
        let companion = temp.path().join("mock-companion");
        fs::create_dir_all(&home).expect("home");
        fs::create_dir_all(&work).expect("work");
        let workspace = Self {
            _temp: temp,
            home,
            work,
            companion,
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

    #[cfg(unix)]
    fn run_with_companion(&self, args: &[&str], label: &str) -> Output {
        let output = run_orbit_with_companion(&self.work, &self.home, args, Some(&self.companion));
        assert!(
            output.status.success(),
            "{label} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    #[cfg(unix)]
    fn run_json_with_companion(&self, args: &[&str], label: &str) -> Value {
        let output = self.run_with_companion(args, label);
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "{label} produced invalid JSON: {error}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    #[cfg(unix)]
    fn run_failure_with_companion(&self, args: &[&str], label: &str) -> Output {
        let output = run_orbit_with_companion(&self.work, &self.home, args, Some(&self.companion));
        assert!(
            !output.status.success(),
            "{label} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    #[cfg(unix)]
    fn write_mock_companion(&self) {
        write_executable(
            &self.companion,
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  if [ -z "$id" ]; then
    id=0
  fi
  case "$line" in
    *'"method":"info"'*)
      printf '{"id":%s,"result":{"model_id":"bge-small-en-v1.5","dim":2,"max_input_tokens":512,"version":"0.3.1"}}\n' "$id"
      ;;
    *'"method":"token_count"'*)
      printf '{"id":%s,"result":{"tokens":1}}\n' "$id"
      ;;
    *'"method":"embed"'*)
      case "$line" in
        *'"texts":["foo"]'*|*semantic-target*)
          printf '{"id":%s,"result":{"vectors":[[1.0,0.0]]}}\n' "$id"
          ;;
        *)
          printf '{"id":%s,"result":{"vectors":[[0.0,1.0]]}}\n' "$id"
          ;;
      esac
      ;;
    *'"method":"exit"'*)
      printf '{"id":%s,"result":{"ok":true}}\n' "$id"
      exit 0
      ;;
    *)
      printf '{"id":%s,"error":{"code":"unknown","message":"unknown request"}}\n' "$id"
      ;;
  esac
done
"#,
        );
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
    run_orbit_with_companion(work, home, args, None)
}

fn learning_dir_count(work: &std::path::Path) -> usize {
    fs::read_dir(work.join(".orbit/learnings"))
        .expect("read learnings")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|file_type| file_type.is_dir()))
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("L-"))
        })
        .count()
}

fn count_learning_embeddings(work: &std::path::Path, source_id: Option<&str>) -> i64 {
    let conn = Connection::open(work.join(".orbit/state/semantic.db")).expect("open semantic db");
    match source_id {
        Some(source_id) => conn
            .query_row(
                "SELECT COUNT(*) FROM embeddings WHERE source_kind = 'learning' AND source_id = ?1",
                params![source_id],
                |row| row.get(0),
            )
            .expect("count learning source embeddings"),
        None => conn
            .query_row(
                "SELECT COUNT(*) FROM embeddings WHERE source_kind = 'learning'",
                [],
                |row| row.get(0),
            )
            .expect("count learning embeddings"),
    }
}

fn run_orbit_with_companion(
    work: &PathBuf,
    home: &PathBuf,
    args: &[&str],
    companion: Option<&std::path::Path>,
) -> Output {
    let mut cmd = cargo_bin_cmd!("orbit");
    cmd.current_dir(work)
        .env("HOME", home)
        .env("ORBIT_HOME", home.join(".orbit-global"))
        .env_remove("ORBIT_ROOT")
        .env_remove("ORBIT_SEARCH_COMPANION")
        .args(args);
    if let Some(path) = companion {
        cmd.env("ORBIT_SEARCH_COMPANION", path);
    }
    cmd.output().expect("run orbit")
}

#[cfg(unix)]
fn write_executable(path: &std::path::Path, content: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, content).expect("write executable");
    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("chmod executable");
}

#[cfg(unix)]
fn make_unreadable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(path, permissions).expect("chmod unreadable");
}

#[cfg(unix)]
fn restore_readable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(path, permissions).expect("chmod readable");
}
