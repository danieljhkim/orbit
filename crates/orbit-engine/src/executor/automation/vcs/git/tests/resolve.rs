use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::tempdir;

use super::super::{BaseSyncMode, resolve_worktree_start_point};

#[test]
fn remote_mode_fetches_origin_base_when_local_base_is_stale() {
    let temp = tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    let seed = temp.path().join("seed");
    let local = temp.path().join("local");

    git(temp.path(), &["init", "--bare", remote.to_str().unwrap()]);
    init_repo(&seed, "agent-main");
    let local_v1 = commit_file(&seed, "base.txt", "v1");
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

    let remote_v2 = commit_file(&seed, "base.txt", "v2");
    git(&seed, &["push", "origin", "agent-main"]);

    assert_eq!(git(&local, &["rev-parse", "agent-main"]), local_v1);

    let start_point =
        resolve_worktree_start_point(&local, "agent-main", BaseSyncMode::Remote).unwrap();

    assert_eq!(start_point, "origin/agent-main");
    assert_eq!(git(&local, &["rev-parse", "agent-main"]), local_v1);
    assert_eq!(git(&local, &["rev-parse", "origin/agent-main"]), remote_v2);
}

#[test]
fn local_mode_resolves_local_base_without_origin_remote() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo, "agent-main");
    commit_file(&repo, "base.txt", "local-only");

    let start_point =
        resolve_worktree_start_point(&repo, "agent-main", BaseSyncMode::Local).unwrap();

    assert_eq!(start_point, "agent-main");
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
