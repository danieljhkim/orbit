use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::tempdir;

use serde_json::json;

use super::super::{ensure_worktree, worktree_setup_output};

#[test]
fn ensure_worktree_resets_existing_checkout_to_supplied_start_point() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("worktree");
    init_repo(&repo, "agent-main");
    let first_base = commit_file(&repo, "base.txt", "v1");

    ensure_worktree(&repo, &worktree, &first_base, "orbit/test").unwrap();
    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), first_base);

    let second_base = commit_file(&repo, "base.txt", "v2");
    ensure_worktree(&repo, &worktree, &second_base, "orbit/test").unwrap();

    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), second_base);
}

#[test]
fn ensure_worktree_reuses_orphan_branch_from_failed_attempt() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("worktree");
    init_repo(&repo, "agent-main");
    let first_base = commit_file(&repo, "base.txt", "v1");
    git(&repo, &["branch", "orbit/test", &first_base]);

    let second_base = commit_file(&repo, "base.txt", "v2");
    ensure_worktree(&repo, &worktree, &second_base, "orbit/test").unwrap();

    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), second_base);
}

#[test]
fn ensure_worktree_prunes_dangling_metadata_from_failed_attempt() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("worktree");
    init_repo(&repo, "agent-main");
    let base = commit_file(&repo, "base.txt", "v1");

    ensure_worktree(&repo, &worktree, &base, "orbit/test").unwrap();
    fs::remove_dir_all(&worktree).unwrap();

    ensure_worktree(&repo, &worktree, &base, "orbit/test").unwrap();

    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), base);
}

#[test]
fn ensure_worktree_reuses_empty_path_from_failed_attempt() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    let worktree = temp.path().join("worktree");
    init_repo(&repo, "agent-main");
    let base = commit_file(&repo, "base.txt", "v1");
    fs::create_dir_all(&worktree).unwrap();

    ensure_worktree(&repo, &worktree, &base, "orbit/test").unwrap();

    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), base);
}

#[test]
fn ensure_worktree_uses_commit_start_point_without_upstream_config() {
    let temp = tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let local = temp.path().join("local");
    let worktree = temp.path().join("worktree");

    git(temp.path(), &["init", "--bare", remote.to_str().unwrap()]);
    init_repo(&seed, "agent-main");
    let remote_head = commit_file(&seed, "base.txt", "v1");
    git(
        &seed,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&seed, &["push", "-u", "origin", "agent-main"]);
    git(
        temp.path(),
        &[
            "clone",
            "--branch",
            "agent-main",
            remote.to_str().unwrap(),
            local.to_str().unwrap(),
        ],
    );

    ensure_worktree(&local, &worktree, "origin/agent-main", "orbit/test").unwrap();

    assert_eq!(git(&worktree, &["rev-parse", "HEAD"]), remote_head);
    assert_git_fails(&local, &["config", "--get", "branch.orbit/test.remote"]);
    assert_git_fails(&local, &["config", "--get", "branch.orbit/test.merge"]);
}

#[test]
fn worktree_setup_output_includes_legacy_batch_id_alias() {
    let output = worktree_setup_output(
        "jrun-test",
        "/tmp/orbit-worktree".to_string(),
        "orbit/ORB-00010".to_string(),
        "main".to_string(),
    );

    assert_eq!(output["job_run_id"], json!("jrun-test"));
    assert_eq!(output["batch_id"], output["job_run_id"]);
}

fn init_repo(path: &Path, branch: &str) {
    fs::create_dir_all(path).unwrap();
    git(path, &["init"]);
    git(path, &["checkout", "-b", branch]);
    git(path, &["config", "user.name", "Orbit Test"]);
    git(path, &["config", "user.email", "orbit-test@example.com"]);
}

fn commit_file(repo: &Path, file_name: &str, contents: &str) -> String {
    fs::write(repo.join(file_name), contents).unwrap();
    git(repo, &["add", file_name]);
    git(repo, &["commit", "-m", &format!("write {file_name}")]);
    git(repo, &["rev-parse", "HEAD"])
}

fn git(current_dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(current_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed in {}:\nstdout: {}\nstderr: {}",
        args.join(" "),
        current_dir.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn assert_git_fails(current_dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(current_dir)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "git {} unexpectedly succeeded in {}:\nstdout: {}\nstderr: {}",
        args.join(" "),
        current_dir.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
