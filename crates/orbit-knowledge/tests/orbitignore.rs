use std::fs;
use std::path::Path;
use std::process::Command;

use orbit_knowledge::graph::object_store::{GraphObjectStore, RefName, resolve_graph_read_target};
use orbit_knowledge::pipeline::context::BuildConfig;
use orbit_knowledge::pipeline::{RefreshStatus, ensure_fresh, run_build};
use orbit_knowledge::service::GraphContextService;
use tempfile::tempdir;

fn git(repo: &Path, args: &[&str]) {
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

fn init_repo(repo: &Path) {
    git(repo, &["init", "-q", "--initial-branch=main"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "user.name", "Test"]);
    git(repo, &["config", "commit.gpgsign", "false"]);
}

fn write_file(repo: &Path, rel: &str, content: &str) {
    let path = repo.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn commit_all(repo: &Path, message: &str) {
    git(repo, &["add", "-A"]);
    let status = Command::new("git")
        .args(["commit", "-q", "-m", message])
        .current_dir(repo)
        .env("GIT_AUTHOR_DATE", "2026-04-21T12:00:00+00:00")
        .env("GIT_COMMITTER_DATE", "2026-04-21T12:00:00+00:00")
        .status()
        .expect("git commit runs");
    assert!(status.success(), "git commit failed in {}", repo.display());
}

fn load_graph(
    knowledge_dir: &Path,
    repo_path: &Path,
) -> orbit_knowledge::graph::nodes::CodebaseGraphV1 {
    let read_target = resolve_graph_read_target(Some(repo_path), None).unwrap();
    let store = GraphObjectStore::new(knowledge_dir.join("graph"));
    store
        .read_graph(
            &read_target.requested,
            read_target.fallback.as_ref(),
            read_target.default.as_ref(),
        )
        .unwrap()
}

fn symbol_search_total(knowledge_dir: &Path, repo_path: &Path, query: &str) -> usize {
    let graph = load_graph(knowledge_dir, repo_path);
    let service = GraphContextService::new(&graph);
    service.search_total(query, Some(&["symbol"]), None, None)
}

fn build_config(repo_path: &Path, output_dir: &Path) -> BuildConfig {
    BuildConfig {
        repo_path: repo_path.to_path_buf(),
        output_dir: output_dir.to_path_buf(),
        incremental: false,
        ref_name: Some(RefName::new("main").unwrap()),
    }
}

#[test]
fn orbitignore_root_file_excludes_symbols_from_graph_search() {
    let repo_dir = tempdir().unwrap();
    let repo = repo_dir.path();
    init_repo(repo);

    write_file(repo, "foo.rs", "pub fn unique_marker_symbol_abc123() {}\n");
    write_file(repo, ".orbitignore", "foo.rs\n");
    commit_all(repo, "seed orbitignore root exclusion");

    let knowledge_dir = tempdir().unwrap();
    let output_dir = knowledge_dir.path().join("knowledge");
    run_build(build_config(repo, &output_dir)).unwrap();

    assert_eq!(
        symbol_search_total(&output_dir, repo, "unique_marker_symbol_abc123"),
        0
    );
}

#[test]
fn orbitignore_nested_files_compose_with_root_file() {
    let repo_dir = tempdir().unwrap();
    let repo = repo_dir.path();
    init_repo(repo);

    write_file(repo, ".orbitignore", "# root rules live here\n");
    write_file(
        repo,
        "subdir/included.rs",
        "pub fn included_marker_symbol() {}\n",
    );
    write_file(
        repo,
        "subdir/excluded.rs",
        "pub fn excluded_marker_symbol() {}\n",
    );
    write_file(repo, "subdir/.orbitignore", "excluded.rs\n");
    commit_all(repo, "seed nested orbitignore");

    let knowledge_dir = tempdir().unwrap();
    let output_dir = knowledge_dir.path().join("knowledge");
    run_build(build_config(repo, &output_dir)).unwrap();

    assert_eq!(
        symbol_search_total(&output_dir, repo, "included_marker_symbol"),
        1
    );
    assert_eq!(
        symbol_search_total(&output_dir, repo, "excluded_marker_symbol"),
        0
    );
}

#[test]
fn orbitignore_default_patterns_exclude_target_without_workspace_file() {
    let repo_dir = tempdir().unwrap();
    let repo = repo_dir.path();
    init_repo(repo);

    write_file(
        repo,
        "target/build_artifact.rs",
        "pub fn build_marker_xyz() {}\n",
    );
    commit_all(repo, "seed target artifact");

    let knowledge_dir = tempdir().unwrap();
    let output_dir = knowledge_dir.path().join("knowledge");
    run_build(build_config(repo, &output_dir)).unwrap();

    assert_eq!(
        symbol_search_total(&output_dir, repo, "build_marker_xyz"),
        0
    );
}

#[test]
fn orbitignore_incremental_refresh_removes_previously_indexed_symbols() {
    let repo_dir = tempdir().unwrap();
    let repo = repo_dir.path();
    init_repo(repo);

    write_file(repo, "subdir/leaf.rs", "pub fn removable_marker() {}\n");
    commit_all(repo, "seed removable leaf");

    let knowledge_dir = tempdir().unwrap();
    let output_dir = knowledge_dir.path().join("knowledge");
    run_build(build_config(repo, &output_dir)).unwrap();

    assert_eq!(
        symbol_search_total(&output_dir, repo, "removable_marker"),
        1
    );

    write_file(repo, ".orbitignore", "subdir/leaf.rs\n");

    let status = ensure_fresh(&output_dir, repo).unwrap();
    assert_eq!(status, RefreshStatus::Rebuilt);
    assert_eq!(
        symbol_search_total(&output_dir, repo, "removable_marker"),
        0
    );
}
