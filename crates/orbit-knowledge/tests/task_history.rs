//! End-to-end integration test for T20260421-0528.
//!
//! Exercises the full attribution pipeline against a real git repository:
//! initialize a repo, stage a file, commit with a `[T...]`-tagged message,
//! run `orbit_knowledge::pipeline::run_build`, then assert that the file
//! node's `task_ids` contains the expected ID, that the sidecar maps the task
//! ID to the commit, and that `CurrentRef.last_attributed_commit` advanced to
//! HEAD.

use std::path::{Path, PathBuf};
use std::process::Command;

use orbit_knowledge::Selector;
use orbit_knowledge::graph::object_store::{GraphObjectStore, RefName};
use orbit_knowledge::pipeline::context::BuildConfig;
use orbit_knowledge::task_commits;
use orbit_knowledge::{HistoryQueryOptions, TaskIdPattern, query_task_history};
use tempfile::tempdir;

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .expect("git command runs");
    assert!(
        status.success(),
        "git {:?} failed in {}",
        args,
        repo.display()
    );
}

fn run_git_with_env(repo: &Path, args: &[&str], env: &[(&str, &str)]) {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(repo);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let status = cmd.status().expect("git command runs");
    assert!(
        status.success(),
        "git {:?} failed in {}",
        args,
        repo.display()
    );
}

fn init_repo(repo: &Path) {
    run_git(repo, &["init", "-q", "--initial-branch=main"]);
    run_git(repo, &["config", "user.email", "test@example.com"]);
    run_git(repo, &["config", "user.name", "Test"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
}

fn write_file(repo: &Path, rel: &str, content: &str) {
    let path = repo.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn commit_all(repo: &Path, message: &str) {
    run_git(repo, &["add", "-A"]);
    run_git_with_env(
        repo,
        &["commit", "-q", "-m", message],
        &[
            ("GIT_AUTHOR_DATE", "2026-04-21T12:00:00+00:00"),
            ("GIT_COMMITTER_DATE", "2026-04-21T12:00:00+00:00"),
        ],
    );
}

fn head_sha(repo: &Path) -> String {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .expect("rev-parse runs");
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn full_pipeline_attributes_task_ids_to_file_and_symbol() {
    let repo_dir = tempdir().expect("tempdir");
    let repo = repo_dir.path();
    init_repo(repo);

    // Initial commit untagged — sets the cursor baseline.
    write_file(repo, "README.md", "stub\n");
    commit_all(repo, "chore: initial commit");

    // Tagged commit adds a Rust function.
    write_file(
        repo,
        "src/lib.rs",
        "pub fn hello() -> &'static str {\n    \"hi\"\n}\n",
    );
    commit_all(repo, "[T20260421-0528] add hello");
    let tagged_sha = head_sha(repo);

    // Run the full pipeline into a scratch knowledge dir.
    let knowledge_root = tempdir().expect("knowledge tempdir");
    let output_dir = knowledge_root.path().join("knowledge");

    let config = BuildConfig {
        repo_path: repo.to_path_buf(),
        output_dir: output_dir.clone(),
        incremental: false,
        ref_name: Some(RefName::new("main").unwrap()),
        task_id_pattern: None,
    };
    let ctx = orbit_knowledge::pipeline::run_build(config).expect("pipeline runs");

    // File node gets the task ID.
    let file = ctx
        .graph
        .files
        .iter()
        .find(|f| f.base.location == "src/lib.rs")
        .expect("src/lib.rs file node exists");
    assert!(
        file.base.task_ids.contains(&"T20260421-0528".to_string()),
        "file node task_ids = {:?}",
        file.base.task_ids
    );

    // Leaf node for hello() gets the task ID.
    let leaf = ctx
        .graph
        .leaves
        .iter()
        .find(|l| l.base.location.starts_with("src/lib.rs#"))
        .expect("hello leaf exists");
    assert!(
        leaf.base.task_ids.contains(&"T20260421-0528".to_string()),
        "hello leaf task_ids = {:?}",
        leaf.base.task_ids
    );

    // Sidecar records the tagged commit.
    let sidecar_path = task_commits::sidecar_path(&output_dir, "main");
    let sidecar = task_commits::load(&sidecar_path).expect("sidecar loads");
    let entries = sidecar.get("T20260421-0528").expect("sidecar has entry");
    assert!(entries.iter().any(|c| c.sha == tagged_sha));

    // CurrentRef.last_attributed_commit advanced to HEAD.
    let store = GraphObjectStore::new(output_dir.join("graph"));
    let current_ref = store
        .read_ref(&RefName::new("main").unwrap())
        .expect("ref loads");
    assert_eq!(
        current_ref.last_attributed_commit.as_deref(),
        Some(head_sha(repo).as_str())
    );

    // Query API resolves the node's task IDs via the sidecar.
    let selector: Selector = "file:src/lib.rs".parse().expect("selector parses");
    let branch = RefName::new("main").unwrap();
    let pattern = TaskIdPattern::default();
    let options = HistoryQueryOptions {
        knowledge_dir: &output_dir,
        repo_path: repo,
        branch_ref: &branch,
        selector: &selector,
        staleness_threshold: orbit_knowledge::DEFAULT_STALENESS_THRESHOLD,
        task_id_pattern: &pattern,
    };
    let result = query_task_history(&options).expect("query succeeds");
    assert_eq!(result.task_history.len(), 1);
    assert_eq!(result.task_history[0].task_id, "T20260421-0528");
    assert_eq!(result.task_history[0].commits.len(), 1);
    assert_eq!(result.task_history[0].commits[0].sha, tagged_sha);
}

#[test]
fn rebuild_is_idempotent_byte_for_byte() {
    let repo_dir = tempdir().expect("tempdir");
    let repo = repo_dir.path();
    init_repo(repo);

    write_file(repo, "README.md", "stub\n");
    commit_all(repo, "chore: init");
    write_file(repo, "src/lib.rs", "pub fn hello() -> u8 { 42 }\n");
    commit_all(repo, "[T20260421-0528] add hello");

    let knowledge_root = tempdir().expect("knowledge tempdir");
    let output_dir = knowledge_root.path().join("knowledge");

    let make_config = || BuildConfig {
        repo_path: repo.to_path_buf(),
        output_dir: output_dir.clone(),
        incremental: false,
        ref_name: Some(RefName::new("main").unwrap()),
        task_id_pattern: None,
    };

    orbit_knowledge::pipeline::run_build(make_config()).expect("first build");
    let first_root_object_hash = current_root_object_hash(&output_dir);
    let first_snapshot = snapshot_graph_objects(&output_dir);

    orbit_knowledge::pipeline::run_build(make_config()).expect("second build");
    let second_root_object_hash = current_root_object_hash(&output_dir);
    let second_snapshot = snapshot_graph_objects(&output_dir);

    assert_eq!(
        first_root_object_hash, second_root_object_hash,
        "root object hash diverged between identical rebuilds"
    );
    assert_eq!(
        first_snapshot, second_snapshot,
        "graph objects diverged between identical rebuilds"
    );
}

#[test]
fn selector_with_no_matching_node_falls_back_to_git_log() {
    let repo_dir = tempdir().expect("tempdir");
    let repo = repo_dir.path();
    init_repo(repo);

    write_file(repo, "src/lib.rs", "fn a() {}\n");
    commit_all(repo, "[T20260421-0528] add");

    let knowledge_root = tempdir().expect("knowledge tempdir");
    let output_dir = knowledge_root.path().join("knowledge");

    // Don't run the pipeline — knowledge_dir does not exist. Fallback path
    // should kick in.
    let selector: Selector = "file:src/lib.rs".parse().unwrap();
    let branch = RefName::new("main").unwrap();
    let pattern = TaskIdPattern::default();
    let options = HistoryQueryOptions {
        knowledge_dir: &output_dir,
        repo_path: repo,
        branch_ref: &branch,
        selector: &selector,
        staleness_threshold: orbit_knowledge::DEFAULT_STALENESS_THRESHOLD,
        task_id_pattern: &pattern,
    };
    let result = query_task_history(&options).expect("fallback runs");
    assert!(matches!(
        result.source,
        orbit_knowledge::HistorySource::GitLogFallback
    ));
    assert!(
        result
            .task_history
            .iter()
            .any(|e| e.task_id == "T20260421-0528")
    );
    assert!(!result.warnings.is_empty(), "fallback emits a warning");
}

fn snapshot_graph_objects(output_dir: &Path) -> Vec<(PathBuf, String)> {
    let objects = output_dir.join("graph/objects");
    let mut entries = Vec::new();
    collect_files(&objects, &mut entries);
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

fn current_root_object_hash(output_dir: &Path) -> String {
    let store = GraphObjectStore::new(output_dir.join("graph"));
    store
        .read_ref(&RefName::new("main").unwrap())
        .expect("ref loads")
        .root_object_hash
}

fn collect_files(root: &Path, acc: &mut Vec<(PathBuf, String)>) {
    let Ok(read_dir) = std::fs::read_dir(root) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, acc);
        } else if path.is_file() {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            acc.push((path, content));
        }
    }
}
