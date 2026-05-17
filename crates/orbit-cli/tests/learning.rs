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
    assert_eq!(shown["vote_count"], 0);
    assert!(shown["last_voted_at"].is_null());
}

#[test]
fn cli_upvote_is_task_idempotent_and_show_reports_vote_stats() {
    let workspace = TestWorkspace::new();
    let added = workspace.add_learning("rule one", &["foo/**"], &["perf"]);
    let id = added["id"].as_str().expect("id");

    let first = workspace.run_json(
        &[
            "learning",
            "upvote",
            "--id",
            id,
            "--model",
            "claude",
            "--task",
            "ORB-00095",
            "--json",
        ],
        "upvote first",
    );
    assert_eq!(first["vote_count"], 1);
    assert!(first["last_voted_at"].as_str().is_some());

    let duplicate = workspace.run_json(
        &[
            "learning",
            "upvote",
            "--id",
            id,
            "--model",
            "claude",
            "--task",
            "ORB-00095",
            "--json",
        ],
        "upvote duplicate",
    );
    assert_eq!(duplicate["vote_count"], 1);

    let second_task = workspace.run_json(
        &[
            "learning",
            "upvote",
            "--id",
            id,
            "--model",
            "claude",
            "--task",
            "ORB-OTHER",
            "--json",
        ],
        "upvote second task",
    );
    assert_eq!(second_task["vote_count"], 2);

    let shown = workspace.run_json(&["learning", "show", id, "--json"], "show learning");
    assert_eq!(shown["vote_count"], 2);
    assert!(shown["last_voted_at"].as_str().is_some());
}

#[test]
fn cli_upvote_without_task_rejects_free_floating_vote_policy() {
    let workspace = TestWorkspace::new();
    let added = workspace.add_learning("rule one", &["foo/**"], &["perf"]);
    let id = added["id"].as_str().expect("id");

    let output = run_orbit(
        &workspace.work,
        &workspace.home,
        &["learning", "upvote", "--id", id, "--model", "claude"],
        None,
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("free-floating votes"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_search_returns_matched_by_annotation_array() {
    let workspace = TestWorkspace::new();
    workspace.add_learning("path scope", &["foo/**"], &[]);
    workspace.add_learning("tag scope", &[], &["alpha"]);

    let path_hits = workspace.run_json(
        &["learning", "search", "--path", "foo/bar.rs", "--json"],
        "search by path",
    );
    let arr = path_hits.as_array().expect("array");
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
fn cli_search_accepts_absolute_paths_inside_workspace() {
    let workspace = TestWorkspace::new();
    let learning = workspace.add_learning("path scope", &["foo/**"], &[]);
    let target = workspace.work.join("foo/bar.rs");
    fs::create_dir_all(target.parent().expect("target parent")).expect("create target dir");
    fs::write(&target, "pub fn example() {}\n").expect("write target");
    let absolute = target.to_string_lossy().to_string();

    let path_hits = workspace.run_json(
        &["learning", "search", "--path", &absolute, "--json"],
        "search by absolute path",
    );
    let ids: Vec<&str> = path_hits
        .as_array()
        .expect("array")
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
fn cli_reindex_returns_rebuilt_count() {
    let workspace = TestWorkspace::new();
    workspace.add_learning("a", &[], &[]);
    workspace.add_learning("b", &[], &[]);
    let result = workspace.run_json(&["learning", "reindex", "--json"], "reindex");
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
fn cli_prune_delete_archives_stale_learnings() {
    let workspace = TestWorkspace::new();
    let learning = workspace.add_learning("stale", &["totally-nonexistent-dir-xyz-456/**"], &[]);
    let result = workspace.run_json(&["learning", "prune", "--delete", "--json"], "prune delete");
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
    let output = workspace.run(
        &["learning", "migrate-layout"],
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
        &["learning", "migrate-layout"],
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
    fs::write(learnings.join("L20260517-1.yaml"), "").expect("legacy flat file");

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
