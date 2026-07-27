#![allow(missing_docs)]
// ORB-00013: Tests use unwrap/expect to keep fixture setup readable.
#![allow(clippy::expect_used, clippy::unwrap_used)]

//! CLI parity tests for `orbit learning <subcommand>`.
//!
//! Per AC #9 of T20260511-6: every MCP-side `orbit.learning.*` tool has a
//! matching CLI subcommand. These black-box tests invoke the CLI against a
//! fresh workspace and assert the JSON output shape matches the host-side
//! serializer (which is what the MCP form returns).

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use assert_cmd::cargo::cargo_bin_cmd;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};

#[test]
fn cli_add_then_show_round_trips_every_field() {
    let workspace = TestWorkspace::new();
    let added = workspace.add_learning("rule one", &["foo/**"], &["perf"]);
    let id = added["id"].as_str().expect("id");

    let shown = workspace.run_json(&["learning", "show", id, "--json"], "show learning");
    assert_eq!(shown["id"], added["id"]);
    assert_eq!(shown["summary"], "rule one");
    assert_eq!(shown["scope"]["paths"], json!(["foo/**"]));
    assert_eq!(shown["scope"]["tags"], json!(["perf"]));
    assert_eq!(shown["status"], "active");
}

#[test]
fn orbit_search_kind_learning_path_returns_matched_by_annotation_array() {
    // ORB-00202: the `learning search --path` axis migrated to the
    // unified search surface. ORB-00205 then converted the `--path`
    // flag into a `path <path>` subcommand, so the equivalent call is
    // `orbit search path <path> --kind learning`. The `matched_by`
    // annotation is preserved for active-only path queries.
    let workspace = TestWorkspace::new();
    workspace.add_learning("path scope", &["foo/**"], &[]);
    workspace.add_learning("tag scope", &[], &["alpha"]);

    let response = workspace.run_json(
        &[
            "search",
            "path",
            "foo/bar.rs",
            "--kind",
            "learning",
            "--json",
        ],
        "orbit search path --kind learning",
    );
    let arr = response["results"].as_array().expect("results array");
    assert!(
        !arr.is_empty(),
        "path search should return at least one row"
    );
    for row in arr {
        let matched_by = row["matched_by"].as_array().expect("matched_by present");
        assert!(!matched_by.is_empty());
        let first = matched_by[0].as_str().expect("string");
        assert!(
            first.starts_with("path:") || first.starts_with("tag:") || first.starts_with("query:"),
            "matched_by axis prefix must be path:|tag:|query:"
        );
    }
}

#[test]
fn orbit_search_kind_learning_accepts_absolute_paths_inside_workspace() {
    // ORB-00202: absolute-path normalization (inside the workspace root)
    // moved from `learning search --path` to the unified search surface.
    // ORB-00205 converted the `--path` flag into a `path <path>`
    // subcommand, so the equivalent call is
    // `orbit search path <absolute> --kind learning`.
    let workspace = TestWorkspace::new();
    let learning = workspace.add_learning("path scope", &["foo/**"], &[]);
    let target = workspace.work.join("foo/bar.rs");
    fs::create_dir_all(target.parent().expect("target parent")).expect("create target dir");
    fs::write(&target, "pub fn example() {}\n").expect("write target");
    let absolute = target.to_string_lossy().to_string();

    let response = workspace.run_json(
        &["search", "path", &absolute, "--kind", "learning", "--json"],
        "orbit search path <absolute> --kind learning",
    );
    let ids: Vec<&str> = response["results"]
        .as_array()
        .expect("results array")
        .iter()
        .map(|row| row["id"].as_str().expect("id"))
        .collect();
    assert!(ids.contains(&learning["id"].as_str().expect("learning id")));
}

#[test]
fn cli_list_filters_by_status_and_returns_json_array() {
    let workspace = TestWorkspace::new();
    let _a = workspace.add_learning("a", &["a/**"], &[]);
    let b = workspace.add_learning("b", &["b/**"], &[]);
    let c = workspace.add_learning("c", &["c/**"], &[]);

    // Supersede b with c, then list active vs superseded.
    workspace.run(
        &[
            "learning",
            "supersede",
            b["id"].as_str().unwrap(),
            "--with",
            c["id"].as_str().unwrap(),
            "--json",
        ],
        None,
        "supersede",
    );

    let active = workspace.run_json(
        &["learning", "list", "--status", "active", "--json"],
        "list active",
    );
    let active_ids: Vec<&str> = active
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert!(!active_ids.contains(&b["id"].as_str().unwrap()));

    let superseded = workspace.run_json(
        &["learning", "list", "--status", "superseded", "--json"],
        "list superseded",
    );
    let superseded_ids: Vec<&str> = superseded
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert!(superseded_ids.contains(&b["id"].as_str().unwrap()));
}

#[test]
fn cli_update_then_show_reflects_changes() {
    let workspace = TestWorkspace::new();
    let added = workspace.add_learning("original", &["foo/**"], &["alpha"]);
    let id = added["id"].as_str().unwrap();

    workspace.run(
        &["learning", "update", id, "--summary", "revised", "--json"],
        None,
        "update summary",
    );
    let shown = workspace.run_json(&["learning", "show", id, "--json"], "show updated");
    assert_eq!(shown["summary"], "revised");
}

#[test]
fn cli_update_scope_fields_preserve_omitted_fields_and_empty_tag_clears() {
    let workspace = TestWorkspace::new();
    let added = workspace.add_learning("original", &["old/**"], &["alpha", "beta"]);
    let id = added["id"].as_str().unwrap();

    workspace.run(
        &["learning", "update", id, "--path", "new/**", "--json"],
        None,
        "update paths while preserving tags",
    );
    let after_path_update = workspace.run_json(
        &["learning", "show", id, "--json"],
        "show paths-only update",
    );
    assert_eq!(after_path_update["scope"]["paths"], json!(["new/**"]));
    assert_eq!(after_path_update["scope"]["tags"], json!(["alpha", "beta"]));

    workspace.run(
        &["learning", "update", id, "--tag", "", "--json"],
        None,
        "clear tags explicitly",
    );
    let after_tag_clear = workspace.run_json(
        &["learning", "show", id, "--json"],
        "show explicit tag clear",
    );
    assert_eq!(after_tag_clear["scope"]["paths"], json!(["new/**"]));
    assert_eq!(after_tag_clear["scope"]["tags"], json!([]));
}

#[test]
fn cli_archive_retires_a_single_active_learning_without_a_replacement() {
    let workspace = TestWorkspace::new();
    let learning = workspace.add_learning("obsolete rule", &[], &[]);
    let id = learning["id"].as_str().unwrap();

    let archived = workspace.run_json(&["learning", "archive", id, "--json"], "archive");
    assert_eq!(archived["status"], "superseded");
    assert!(archived["superseded_by"].is_null());

    let active_ids = workspace.learning_projection("active");
    assert!(
        active_ids
            .iter()
            .all(|row| !row.starts_with(&format!("{id}|")))
    );
}

#[test]
fn cli_archive_is_idempotent_and_rejects_a_missing_id() {
    let workspace = TestWorkspace::new();
    let learning = workspace.add_learning("obsolete rule", &[], &[]);
    let id = learning["id"].as_str().unwrap();

    workspace.run(
        &["learning", "archive", id, "--json"],
        None,
        "first archive",
    );
    let second = workspace.run_json(
        &["learning", "archive", id, "--json"],
        "second archive is a no-op success",
    );
    assert_eq!(second["status"], "superseded");

    let missing = workspace.try_run_as(&["learning", "archive", "L-9999999"], HUMAN);
    assert!(!missing.status.success(), "missing id must fail");
}

#[test]
fn cli_sync_returns_rebuilt_count() {
    let workspace = TestWorkspace::new();
    workspace.add_learning("a", &[], &[]);
    workspace.add_learning("b", &[], &[]);
    let result = workspace.run_json(&["learning", "sync", "--json"], "sync");
    assert!(result["rebuilt_count"].as_u64().unwrap() >= 2);
}

#[test]
fn cli_prune_stale_only_reports_without_modifying() {
    let workspace = TestWorkspace::new();
    let learning = workspace.add_learning("stale", &["totally-nonexistent-dir-xyz-123/**"], &[]);
    let report = workspace.run_json(&["learning", "prune", "--json"], "prune stale only");
    let stale = report["stale"].as_array().expect("stale array");
    let stale_ids: Vec<&str> = stale.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(stale_ids.contains(&learning["id"].as_str().unwrap()));
    assert!(report["deleted"].as_array().unwrap().is_empty());
}

#[test]
fn cli_prune_confirm_archives_stale_learnings_and_preserves_delete_alias() {
    let workspace = TestWorkspace::new();
    let learning = workspace.add_learning("stale", &["totally-nonexistent-dir-xyz-456/**"], &[]);

    // ORB-10453: pruning is a governed operation, so an agent caller is
    // refused at the CLI chokepoint before the command runs at all.
    let refused = workspace.try_run_as(
        &["learning", "prune", "--confirm", "--json"],
        &[("ORBIT_AGENT_NAME", "claude")],
    );
    assert!(!refused.status.success());
    let refusal = String::from_utf8_lossy(&refused.stderr);
    assert!(refusal.contains("capability denied"), "{refusal}");
    assert!(refusal.contains("ORBIT_OPERATOR"), "{refusal}");

    // A test binary is not a terminal, so it claims the operator capability
    // the same explicit way a deliberate human action would.
    const OPERATOR: &[(&str, &str)] = &[("ORBIT_OPERATOR", "1")];

    let result = json_output(workspace.run_as(
        &["learning", "prune", "--confirm", "--json"],
        OPERATOR,
        "prune confirm",
    ));
    let deleted: Vec<&str> = result["deleted"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(deleted.contains(&learning["id"].as_str().unwrap()));

    // Verify the YAML status is superseded and superseded_by=null per §7.3.
    let shown = workspace.run_json(
        &[
            "learning",
            "show",
            learning["id"].as_str().unwrap(),
            "--json",
        ],
        "show archived",
    );
    assert_eq!(shown["status"], "superseded");
    assert!(shown["superseded_by"].is_null());

    let alias = json_output(workspace.run_as(
        &["learning", "prune", "--delete", "--json"],
        OPERATOR,
        "prune delete alias",
    ));
    assert!(alias["deleted"].as_array().expect("deleted").is_empty());
}

#[test]
fn cli_migrate_layout_preserves_records_and_is_idempotent() {
    let workspace = TestWorkspace::new();
    let _active = workspace.add_learning("active rule", &["active/**"], &["keep"]);
    let old = workspace.add_learning("old rule", &["old/**"], &["archive"]);
    let new = workspace.add_learning("new rule", &["new/**"], &["keep"]);
    workspace.run(
        &[
            "learning",
            "supersede",
            old["id"].as_str().unwrap(),
            "--with",
            new["id"].as_str().unwrap(),
            "--json",
        ],
        None,
        "supersede before migration",
    );
    let active_before = workspace.learning_projection("active");
    let superseded_before = workspace.learning_projection("superseded");

    workspace.convert_learning_store_to_legacy_flat();
    let before_dry_run = snapshot_files(&workspace.work.join(".orbit/learnings"));
    let output = workspace.run(
        &["learning", "migrate-layout"],
        None,
        "inspect legacy learning layout",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Would migrate learning layout"));
    assert_eq!(
        snapshot_files(&workspace.work.join(".orbit/learnings")),
        before_dry_run
    );

    let output = workspace.run(
        &["learning", "migrate-layout", "--confirm"],
        None,
        "migrate legacy learning layout",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Migrated learning layout"));

    assert_eq!(workspace.learning_projection("active"), active_before);
    assert_eq!(
        workspace.learning_projection("superseded"),
        superseded_before
    );
    let learnings_root = workspace.work.join(".orbit/learnings");
    assert!(
        fs::read_dir(&learnings_root)
            .expect("read learnings")
            .all(|entry| {
                let path = entry.expect("entry").path();
                !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('L') && name.ends_with(".yaml"))
            })
    );
    assert!(!learnings_root.join("superseded").exists());

    let before_rerun = snapshot_files(&learnings_root);
    let output = workspace.run(
        &["learning", "migrate-layout", "--confirm"],
        None,
        "rerun migrated layout",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("workspace is already on the per-entity layout"));
    assert_eq!(snapshot_files(&learnings_root), before_rerun);
}

#[test]
fn guardrail_rejects_flat_learning_root_files() {
    let temp = tempdir().expect("tempdir");
    let learnings = temp.path().join(".orbit/learnings");
    fs::create_dir_all(&learnings).expect("create learnings");
    fs::write(learnings.join("L-0001.yaml"), "").expect("legacy flat file");

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf();
    let output = Command::new(repo_root.join("scripts/check-learning-layout.sh"))
        .arg(temp.path())
        .output()
        .expect("run guardrail");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("flat legacy learning file"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// --- ORB-10364: caller-role gate on the learning authoring surfaces --------
//
// Policy: task executors file frictions; learnings are authored by the
// orchestrator or by a human. These tests spawn the CLI with an explicitly
// declared identity environment, so they exercise the real end-to-end
// behaviour of both the `orbit learning *` subcommands and their
// `orbit.learning.*` tool equivalents without mutating this process's env.

/// An agent-context run with no authoring opt-in — i.e. a task executor.
const EXECUTOR: &[(&str, &str)] = &[("ORBIT_AGENT_MODEL", "claude-opus-5")];
/// A human at a terminal: no agent-identity pair at all.
const HUMAN: &[(&str, &str)] = &[];
/// The orchestrator dispatching curation work as an agent, opted in.
const ORCHESTRATOR: &[(&str, &str)] = &[
    ("ORBIT_AGENT_MODEL", "claude-opus-5"),
    ("ORBIT_LEARNING_AUTHOR", "1"),
];

#[test]
fn executor_context_learning_add_is_refused_and_redirected_to_friction_add() {
    let workspace = TestWorkspace::new();

    let output = workspace.try_run_as(
        &[
            "learning",
            "add",
            "--summary",
            "executor observation",
            "--body",
            "the body",
        ],
        EXECUTOR,
    );

    assert!(!output.status.success(), "executor add must be refused");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("policy denied"), "stderr: {stderr}");
    assert!(
        stderr.contains("orbit friction add"),
        "names the correct channel: {stderr}"
    );
    assert!(
        stderr.contains("executor observation") && stderr.contains("the body"),
        "preserves the attempted content: {stderr}"
    );
    assert!(
        stderr.contains("ORBIT_LEARNING_AUTHOR"),
        "names the orchestrator opt-in: {stderr}"
    );
    assert!(
        workspace.learning_projection("active").is_empty(),
        "nothing was written"
    );
}

#[test]
fn executor_context_learning_update_and_supersede_are_refused_leaving_records_untouched() {
    let workspace = TestWorkspace::new();
    let old = workspace.add_learning("original summary", &[], &[]);
    let new = workspace.add_learning("replacement summary", &[], &[]);
    let old_id = old["id"].as_str().expect("old id");
    let new_id = new["id"].as_str().expect("new id");
    let before = workspace.learning_projection("active");

    let update = workspace.try_run_as(
        &["learning", "update", old_id, "--summary", "rewritten"],
        EXECUTOR,
    );
    assert!(!update.status.success(), "executor update must be refused");
    let update_stderr = String::from_utf8_lossy(&update.stderr);
    assert!(
        update_stderr.contains("orbit friction add") && update_stderr.contains("rewritten"),
        "stderr: {update_stderr}"
    );

    let supersede = workspace.try_run_as(
        &["learning", "supersede", old_id, "--with", new_id],
        EXECUTOR,
    );
    assert!(
        !supersede.status.success(),
        "executor supersede must be refused"
    );
    let supersede_stderr = String::from_utf8_lossy(&supersede.stderr);
    assert!(
        supersede_stderr.contains("orbit friction add") && supersede_stderr.contains(old_id),
        "stderr: {supersede_stderr}"
    );

    assert_eq!(workspace.learning_projection("active"), before);
    assert!(workspace.learning_projection("superseded").is_empty());
}

#[test]
fn executor_context_learning_archive_is_refused_leaving_the_record_untouched() {
    let workspace = TestWorkspace::new();
    let learning = workspace.add_learning("obsolete rule", &[], &[]);
    let id = learning["id"].as_str().expect("id");
    let before = workspace.learning_projection("active");

    let archive = workspace.try_run_as(&["learning", "archive", id], EXECUTOR);
    assert!(
        !archive.status.success(),
        "executor archive must be refused"
    );
    let stderr = String::from_utf8_lossy(&archive.stderr);
    assert!(
        stderr.contains("orbit friction add") && stderr.contains(id),
        "stderr: {stderr}"
    );
    assert_eq!(workspace.learning_projection("active"), before);
}

#[test]
fn executor_context_learning_tools_are_refused_with_the_same_redirect() {
    let workspace = TestWorkspace::new();
    let old = workspace.add_learning("tool original", &[], &[]);
    let new = workspace.add_learning("tool replacement", &[], &[]);
    let old_id = old["id"].as_str().expect("old id").to_string();
    let new_id = new["id"].as_str().expect("new id").to_string();
    let before = workspace.learning_projection("active");

    let add_input = json!({ "summary": "tool observation", "body": "tool body" }).to_string();
    let update_input = json!({ "id": old_id, "summary": "tool rewrite" }).to_string();
    let supersede_input = json!({ "id": old_id, "with": new_id }).to_string();
    let archive_input = json!({ "id": old_id }).to_string();
    let attempts = [
        ("orbit.learning.add", &add_input, "tool observation"),
        ("orbit.learning.update", &update_input, "tool rewrite"),
        (
            "orbit.learning.supersede",
            &supersede_input,
            old_id.as_str(),
        ),
        ("orbit.learning.archive", &archive_input, old_id.as_str()),
    ];

    for (tool, input, echoed) in attempts {
        let output = workspace.try_run_as(&["tool", "run", tool, "--input", input], EXECUTOR);
        assert!(!output.status.success(), "{tool} must be refused");
        // `tool run` reports failures as a JSON envelope on stdout.
        let reported: Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|e| panic!("{tool} produced invalid JSON: {e}"));
        assert_eq!(reported["code"], "policy_denied", "{tool}");
        let message = reported["error"].as_str().unwrap_or_default();
        assert!(
            message.contains("orbit friction add"),
            "{tool} names the correct channel: {message}"
        );
        assert!(
            message.contains(echoed),
            "{tool} preserves the attempted content: {message}"
        );
        assert!(
            message.contains("ORBIT_LEARNING_AUTHOR"),
            "{tool} names the orchestrator opt-in: {message}"
        );
    }

    assert_eq!(workspace.learning_projection("active"), before);
}

#[test]
fn human_context_authors_learnings_across_every_write_surface() {
    let workspace = TestWorkspace::new();

    let added = workspace.run_json(
        &["learning", "add", "--summary", "human authored", "--json"],
        "human add",
    );
    let id = added["id"].as_str().expect("id").to_string();

    workspace.run_as(
        &["learning", "update", &id, "--summary", "human rewrote"],
        HUMAN,
        "human update",
    );

    let replacement = workspace.add_learning("human replacement", &[], &[]);
    let replacement_id = replacement["id"].as_str().expect("id").to_string();
    workspace.run_as(
        &["learning", "supersede", &id, "--with", &replacement_id],
        HUMAN,
        "human supersede",
    );

    let shown = workspace.run_json(&["learning", "show", &id, "--json"], "show superseded");
    assert_eq!(shown["summary"], "human rewrote");
    assert_eq!(shown["status"], "superseded");
}

#[test]
fn the_orchestrator_opt_in_authors_learnings_from_an_agent_context() {
    let workspace = TestWorkspace::new();

    let added: Value = serde_json::from_slice(
        &workspace
            .run_as(
                &["learning", "add", "--summary", "curated rule", "--json"],
                ORCHESTRATOR,
                "opt-in add",
            )
            .stdout,
    )
    .expect("opt-in add JSON");
    let id = added["id"].as_str().expect("id").to_string();

    workspace.run_as(
        &[
            "learning",
            "update",
            &id,
            "--summary",
            "curated rule, narrowed",
        ],
        ORCHESTRATOR,
        "opt-in update",
    );

    // The tool surface honours the same opt-in.
    let replacement: Value = serde_json::from_slice(
        &workspace
            .run_as(
                &[
                    "tool",
                    "run",
                    "orbit.learning.add",
                    "--input",
                    &json!({ "summary": "curated replacement" }).to_string(),
                ],
                ORCHESTRATOR,
                "opt-in tool add",
            )
            .stdout,
    )
    .expect("opt-in tool add JSON");
    let replacement_id = replacement["id"].as_str().expect("id").to_string();

    workspace.run_as(
        &["learning", "supersede", &id, "--with", &replacement_id],
        ORCHESTRATOR,
        "opt-in supersede",
    );

    let shown = workspace.run_json(&["learning", "show", &id, "--json"], "show superseded");
    assert_eq!(shown["summary"], "curated rule, narrowed");
    assert_eq!(shown["status"], "superseded");
    assert_eq!(shown["superseded_by"], replacement_id);
}

#[test]
fn human_context_archives_a_learning_without_a_replacement() {
    let workspace = TestWorkspace::new();
    let learning = workspace.add_learning("human obsolete rule", &[], &[]);
    let id = learning["id"].as_str().expect("id").to_string();

    workspace.run_as(&["learning", "archive", &id], HUMAN, "human archive");

    let shown = workspace.run_json(&["learning", "show", &id, "--json"], "show archived");
    assert_eq!(shown["status"], "superseded");
    assert!(shown["superseded_by"].is_null());
}

#[test]
fn the_orchestrator_opt_in_archives_a_learning_from_an_agent_context() {
    let workspace = TestWorkspace::new();
    let added: Value = serde_json::from_slice(
        &workspace
            .run_as(
                &["learning", "add", "--summary", "curated obsolete", "--json"],
                ORCHESTRATOR,
                "opt-in add",
            )
            .stdout,
    )
    .expect("opt-in add JSON");
    let id = added["id"].as_str().expect("id").to_string();

    workspace.run_as(
        &["learning", "archive", &id],
        ORCHESTRATOR,
        "opt-in archive",
    );

    let shown = workspace.run_json(&["learning", "show", &id, "--json"], "show archived");
    assert_eq!(shown["status"], "superseded");
    assert!(shown["superseded_by"].is_null());
}

#[test]
fn learning_reads_are_unaffected_in_every_caller_context() {
    let workspace = TestWorkspace::new();
    let added = workspace.add_learning("readable rule", &["foo/**"], &["perf"]);
    let id = added["id"].as_str().expect("id").to_string();

    let show_input = json!({ "id": id }).to_string();
    // `orbit.learning.list` is deliberately inactive on the agent tool
    // surface, so the tool-side read under test is `orbit.learning.show`.
    let reads: [Vec<&str>; 3] = [
        vec!["learning", "show", &id, "--json"],
        vec!["learning", "list", "--json"],
        vec!["tool", "run", "orbit.learning.show", "--input", &show_input],
    ];

    for (context_name, context) in [
        ("human", HUMAN),
        ("executor", EXECUTOR),
        ("orchestrator", ORCHESTRATOR),
    ] {
        for read in &reads {
            let output = workspace.run_as(read, context, &format!("{context_name} read {read:?}"));
            let value: Value = serde_json::from_slice(&output.stdout)
                .unwrap_or_else(|e| panic!("{context_name} {read:?} invalid JSON: {e}"));
            let summary = match &value {
                Value::Array(rows) => rows[0]["summary"].clone(),
                other => other["summary"].clone(),
            };
            assert_eq!(
                summary, "readable rule",
                "{context_name} read {read:?} must return the record"
            );
        }
    }
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
            &["workspace", "init", "--name", "learning-cli-test"],
            None,
            "initialize workspace",
        );
        workspace
    }

    fn add_learning(&self, summary: &str, paths: &[&str], tags: &[&str]) -> Value {
        let mut args = vec!["learning", "add", "--summary", summary, "--json"];
        for path in paths {
            args.push("--path");
            args.push(*path);
        }
        for tag in tags {
            args.push("--tag");
            args.push(*tag);
        }
        self.run_json(&args, "add learning")
    }

    /// Run with an explicit caller context, returning the raw outcome so a
    /// test can assert on a refusal as well as on success.
    fn try_run_as(&self, args: &[&str], context: &[(&str, &str)]) -> Output {
        run_orbit_with_env(&self.work, &self.home, args, None, context)
    }

    fn run_as(&self, args: &[&str], context: &[(&str, &str)], label: &str) -> Output {
        let output = self.try_run_as(args, context);
        assert!(
            output.status.success(),
            "{label} failed\nargs: {args:?}\ncontext: {context:?}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn run(&self, args: &[&str], stdin: Option<&str>, label: &str) -> Output {
        let output = run_orbit(&self.work, &self.home, args, stdin);
        assert!(
            output.status.success(),
            "{label} failed\nargs: {args:?}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn run_json(&self, args: &[&str], label: &str) -> Value {
        let output = self.run(args, None, label);
        serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
            panic!(
                "{label} produced invalid JSON: {e}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    fn learning_projection(&self, status: &str) -> Vec<String> {
        let rows = self.run_json(
            &["learning", "list", "--status", status, "--json"],
            "list learning projection",
        );
        let mut projection = rows
            .as_array()
            .expect("array")
            .iter()
            .map(|item| {
                format!(
                    "{}|{}|{}|{}",
                    item["id"].as_str().unwrap(),
                    item["status"].as_str().unwrap(),
                    item["summary"].as_str().unwrap(),
                    item["evidence"]
                )
            })
            .collect::<Vec<_>>();
        projection.sort();
        projection
    }

    fn convert_learning_store_to_legacy_flat(&self) {
        let learnings_root = self.work.join(".orbit/learnings");
        let superseded_root = learnings_root.join("superseded");
        fs::create_dir_all(&superseded_root).expect("create legacy superseded");
        let entries = fs::read_dir(&learnings_root)
            .expect("read learnings")
            .map(|entry| entry.expect("entry").path())
            .collect::<Vec<_>>();
        for path in entries {
            if !path.is_dir() {
                continue;
            }
            let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !id.starts_with('L') {
                continue;
            }
            let yaml_path = path.join("learning.yaml");
            let yaml = fs::read_to_string(&yaml_path).expect("read learning yaml");
            let target = if yaml.contains("status: superseded") {
                superseded_root.join(format!("{id}.yaml"))
            } else {
                learnings_root.join(format!("{id}.yaml"))
            };
            fs::rename(&yaml_path, target).expect("move to legacy flat");
            fs::remove_dir_all(&path).expect("remove per-entity dir");
        }
    }
}

fn snapshot_files(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        if path.is_dir() {
            let mut entries = fs::read_dir(path)
                .expect("read snapshot dir")
                .map(|entry| entry.expect("entry").path())
                .collect::<Vec<_>>();
            entries.sort();
            for entry in entries {
                visit(root, &entry, out);
            }
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .expect("strip root")
                .to_string_lossy()
                .replace('\\', "/");
            out.push((relative, fs::read(path).expect("read snapshot file")));
        }
    }

    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn run_orbit(cwd: &Path, home: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    run_orbit_with_env(cwd, home, args, stdin, &[])
}

/// Parse a successful command's stdout as JSON.
fn json_output(output: Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("parse JSON output")
}

/// Spawn the CLI with a fully declared identity environment.
///
/// The [ORB-10364] caller-role gate and audit-role resolution both read
/// `ORBIT_AGENT_*` from the process env, and a child inherits whatever the
/// suite was launched with. Clearing the identity pair and the authoring
/// opt-in here makes "human context" an explicit property of each test rather
/// than an accident of how the suite was started (the ORB-10350 hazard);
/// `extra_env` then declares the context a test actually wants.
fn run_orbit_with_env(
    cwd: &Path,
    home: &Path,
    args: &[&str],
    stdin: Option<&str>,
    extra_env: &[(&str, &str)],
) -> Output {
    let mut command = cargo_bin_cmd!("orbit");
    command
        .current_dir(cwd)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("ORBIT_ROOT")
        .env_remove("ORBIT_AGENT_NAME")
        .env_remove("ORBIT_AGENT_MODEL")
        .env_remove("ORBIT_LEARNING_AUTHOR")
        .args(args);
    for (name, value) in extra_env {
        command.env(name, value);
    }
    if let Some(input) = stdin {
        command.write_stdin(input);
    }
    command.output().expect("run orbit")
}
