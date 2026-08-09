//! ORB-10644. Every fixture is a plain local repository: a stacked branch that
//! never landed, one that landed as a merge, one that landed as a squash under
//! its task marker, and one whose branch was removed from `origin`. None of
//! them consults GitHub, so the gate is decided by deterministic Git state
//! alone.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::tempdir;

use super::super::base_obsolescence::{
    BaseObsolescenceMode, BaseStatus, base_obsolescence_mode_from_input, classify_base,
};
use super::super::git::BaseSyncMode;

const LANDING: &str = "agent-main";
const STACKED_BASE: &str = "orbit/ORB-10643";

#[test]
fn a_stacked_base_that_never_landed_is_live() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    commit_marked(&repo, "ORB-10600", "base.txt", "v1");
    let base_sha = commit_on_branch(&repo, STACKED_BASE, "ORB-10643", "parent.txt", "parent");

    assert_eq!(
        classify_base(
            &repo,
            STACKED_BASE,
            &base_sha,
            Some(LANDING),
            BaseSyncMode::Local,
        )
        .unwrap(),
        BaseStatus::Live
    );
}

#[test]
fn a_base_merged_into_the_landing_branch_is_obsolete() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    commit_marked(&repo, "ORB-10600", "base.txt", "v1");
    let base_sha = commit_on_branch(&repo, STACKED_BASE, "ORB-10643", "parent.txt", "parent");
    git(
        &repo,
        &["merge", "--no-ff", "-m", "merge parent", STACKED_BASE],
    );

    let status = classify_base(
        &repo,
        STACKED_BASE,
        &base_sha,
        Some(LANDING),
        BaseSyncMode::Local,
    )
    .unwrap();

    let BaseStatus::Landed(detail) = status else {
        panic!("a merged base must be obsolete, got {status:?}");
    };
    assert!(
        detail.contains("ancestor"),
        "the merge shape must be named: {detail}"
    );
}

#[test]
fn a_base_squash_landed_under_its_task_marker_is_obsolete() {
    // The shape Orbit's own `merge_batch_pr` produces: the sha is rewritten, so
    // only the task marker carried into the squash commit says the work landed.
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    commit_marked(&repo, "ORB-10600", "base.txt", "v1");
    let base_sha = commit_on_branch(&repo, STACKED_BASE, "ORB-10643", "parent.txt", "parent");
    git(&repo, &["merge", "--squash", STACKED_BASE]);
    git(&repo, &["commit", "-m", "[ORB-10643] parent work (#901)"]);
    assert_ne!(base_sha, git(&repo, &["rev-parse", LANDING]));

    let status = classify_base(
        &repo,
        STACKED_BASE,
        &base_sha,
        Some(LANDING),
        BaseSyncMode::Local,
    )
    .unwrap();

    let BaseStatus::Landed(detail) = status else {
        panic!("a squash-landed base must be obsolete, got {status:?}");
    };
    assert!(
        detail.contains("[ORB-10643]"),
        "the diagnostic must name the delivered marker: {detail}"
    );
}

#[test]
fn one_undelivered_commit_keeps_the_whole_base_live() {
    // Half-landed is not landed: the base still carries work no merge into the
    // landing branch has taken, so delivery through it is still real.
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    commit_marked(&repo, "ORB-10600", "base.txt", "v1");
    commit_on_branch(&repo, STACKED_BASE, "ORB-10643", "parent.txt", "parent");
    git(&repo, &["merge", "--squash", STACKED_BASE]);
    git(&repo, &["commit", "-m", "[ORB-10643] parent work (#901)"]);
    git(&repo, &["checkout", STACKED_BASE]);
    let base_sha = commit_marked(&repo, "ORB-10650", "followup.txt", "more");
    git(&repo, &["checkout", LANDING]);

    assert_eq!(
        classify_base(
            &repo,
            STACKED_BASE,
            &base_sha,
            Some(LANDING),
            BaseSyncMode::Local,
        )
        .unwrap(),
        BaseStatus::Live
    );
}

#[test]
fn an_unmarked_commit_keeps_the_base_live() {
    // Without a marker there is nothing to look for on the landing branch, and
    // guessing would refuse work that never landed.
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    commit_marked(&repo, "ORB-10600", "base.txt", "v1");
    git(&repo, &["checkout", "-b", STACKED_BASE]);
    fs::write(repo.join("parent.txt"), "parent").unwrap();
    git(&repo, &["add", "parent.txt"]);
    git(&repo, &["commit", "-m", "wip without a task marker"]);
    let base_sha = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["checkout", LANDING]);

    assert_eq!(
        classify_base(
            &repo,
            STACKED_BASE,
            &base_sha,
            Some(LANDING),
            BaseSyncMode::Local,
        )
        .unwrap(),
        BaseStatus::Live
    );
}

#[test]
fn the_landing_branch_is_never_obsolete_against_itself() {
    // Ordinary non-stacked delivery: the base is trivially its own ancestor and
    // must not be read as landed.
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    let base_sha = commit_marked(&repo, "ORB-10600", "base.txt", "v1");

    assert_eq!(
        classify_base(
            &repo,
            LANDING,
            &base_sha,
            Some(LANDING),
            BaseSyncMode::Local
        )
        .unwrap(),
        BaseStatus::Live
    );
    assert_eq!(
        classify_base(
            &repo,
            LANDING,
            &base_sha,
            Some("origin/agent-main"),
            BaseSyncMode::Local,
        )
        .unwrap(),
        BaseStatus::Live
    );
}

#[test]
fn without_a_declared_landing_branch_only_removal_is_checked() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    commit_marked(&repo, "ORB-10600", "base.txt", "v1");
    let base_sha = commit_on_branch(&repo, STACKED_BASE, "ORB-10643", "parent.txt", "parent");
    git(
        &repo,
        &["merge", "--no-ff", "-m", "merge parent", STACKED_BASE],
    );

    assert_eq!(
        classify_base(&repo, STACKED_BASE, &base_sha, None, BaseSyncMode::Local).unwrap(),
        BaseStatus::Live
    );
}

#[test]
fn a_base_branch_removed_from_origin_is_obsolete() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    let remote = temp.path().join("remote.git");
    git(
        temp.path(),
        &["init", "--bare", remote.to_str().unwrap(), "-b", LANDING],
    );
    init_repo(&repo);
    commit_marked(&repo, "ORB-10600", "base.txt", "v1");
    git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&repo, &["push", "-u", "origin", LANDING]);
    let base_sha = commit_on_branch(&repo, STACKED_BASE, "ORB-10643", "parent.txt", "parent");
    git(&repo, &["push", "-u", "origin", STACKED_BASE]);
    // The branch merges and is deleted upstream; the local branch and its
    // remote-tracking ref both survive and still resolve.
    git(&repo, &["push", "origin", "--delete", STACKED_BASE]);

    assert_eq!(
        classify_base(
            &repo,
            STACKED_BASE,
            &base_sha,
            Some(LANDING),
            BaseSyncMode::Local,
        )
        .unwrap(),
        BaseStatus::Deleted
    );
    assert_eq!(
        classify_base(&repo, LANDING, &base_sha, None, BaseSyncMode::Local).unwrap(),
        BaseStatus::Live,
        "a base still on origin stays live"
    );
}

#[test]
fn obsolescence_mode_defaults_to_enforce_and_rejects_unknown_values() {
    assert_eq!(
        base_obsolescence_mode_from_input(&serde_json::json!({})).unwrap(),
        BaseObsolescenceMode::Enforce
    );
    assert_eq!(
        base_obsolescence_mode_from_input(&serde_json::json!({"base_obsolescence": "ignore"}))
            .unwrap(),
        BaseObsolescenceMode::Ignore
    );
    let error =
        base_obsolescence_mode_from_input(&serde_json::json!({"base_obsolescence": "maybe"}))
            .unwrap_err();
    assert!(
        error.to_string().contains("base_obsolescence"),
        "unexpected error: {error}"
    );
}

fn init_repo(path: &Path) {
    fs::create_dir_all(path).unwrap();
    git(path, &["init"]);
    git(path, &["checkout", "-b", LANDING]);
    git(path, &["config", "user.name", "Orbit Test"]);
    git(path, &["config", "user.email", "orbit-test@example.com"]);
    // A machine-global `core.hooksPath` rewrites fixture commit messages
    // (ORB-10350); the markers these tests grep for must be exactly what the
    // test wrote.
    let hooks = path.join(".git").join("orbit-test-empty-hooks");
    fs::create_dir_all(&hooks).unwrap();
    git(
        path,
        &["config", "core.hooksPath", &hooks.to_string_lossy()],
    );
}

fn commit_marked(repo: &Path, task_id: &str, file_name: &str, contents: &str) -> String {
    fs::write(repo.join(file_name), contents).unwrap();
    git(repo, &["add", file_name]);
    git(
        repo,
        &["commit", "-m", &format!("[{task_id}] write {file_name}")],
    );
    git(repo, &["rev-parse", "HEAD"])
}

fn commit_on_branch(
    repo: &Path,
    branch: &str,
    task_id: &str,
    file_name: &str,
    contents: &str,
) -> String {
    git(repo, &["checkout", "-b", branch]);
    let sha = commit_marked(repo, task_id, file_name, contents);
    git(repo, &["checkout", LANDING]);
    sha
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
