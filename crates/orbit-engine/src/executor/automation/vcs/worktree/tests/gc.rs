#![allow(missing_docs)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use chrono::Utc;
use orbit_common::types::{JobRun, JobRunState};
use serde_json::json;
use tempfile::tempdir;

use super::super::gc::{WorktreeGcOptions, collect_worktrees};
use super::super::resolve_worktree_path_from_prefix;

static WORKTREE_ROOT_ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn unrecognized_hand_made_worktree_survives_collection() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    let hand_made = repo
        .join(".orbit/state/worktrees")
        .join("orbit-ORB-10354-part2");
    fs::create_dir_all(&hand_made).unwrap();
    fs::write(hand_made.join("rescue.txt"), "keep me").unwrap();

    let result = collect_worktrees(
        &repo,
        &[],
        &WorktreeGcOptions {
            delete: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(hand_made.exists());
    let report = result
        .reports
        .iter()
        .find(|report| report.path == hand_made)
        .expect("unrecognized report");
    assert_eq!(report.action, "skipped:unrecognized");
    assert_eq!(report.run_id, None);
}

#[test]
fn terminal_dirty_worktree_is_a_rescue_candidate() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    let run = run("jrun-dirty", JobRunState::Failed);
    let worktree = resolved_task_worktree(&repo, &run);
    add_worktree(&repo, &worktree, "orbit/dirty");
    fs::write(worktree.join("uncommitted.txt"), "valuable").unwrap();

    let result = collect_worktrees(
        &repo,
        &[run],
        &WorktreeGcOptions {
            delete: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(worktree.exists());
    assert_eq!(result.reports[0].action, "skipped:dirty_rescue_candidate");
    assert_eq!(result.reports[0].bytes_reclaimed, 0);
}

#[test]
fn dry_run_and_yes_share_eligibility_but_only_yes_removes() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    let run = run("jrun-clean", JobRunState::Success);
    let worktree = resolved_task_worktree(&repo, &run);
    add_worktree(&repo, &worktree, "orbit/clean");
    fs::write(worktree.join("bytes.bin"), [1_u8; 32]).unwrap();
    git(&worktree, &["add", "bytes.bin"]);
    git(&worktree, &["commit", "-m", "worktree content"]);
    git(&repo, &["merge", "--ff-only", "orbit/clean"]);

    let dry = collect_worktrees(
        &repo,
        std::slice::from_ref(&run),
        &WorktreeGcOptions::default(),
    )
    .unwrap();
    assert!(worktree.exists());
    assert_eq!(dry.reports[0].action, "would_remove");

    let applied = collect_worktrees(
        &repo,
        &[run],
        &WorktreeGcOptions {
            delete: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!worktree.exists());
    assert_eq!(applied.reports[0].action, "removed");
    assert!(applied.reports[0].bytes_reclaimed > 0);
}

#[test]
fn colliding_sanitized_run_ids_are_ambiguous_and_never_removed() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    let first = run("jrun:collision", JobRunState::Success);
    let second = run("jrun-collision", JobRunState::Success);
    let worktree = resolved_task_worktree(&repo, &first);
    assert_eq!(worktree, resolved_task_worktree(&repo, &second));
    add_worktree(&repo, &worktree, "orbit/collision");

    let result = collect_worktrees(
        &repo,
        &[first, second],
        &WorktreeGcOptions {
            delete: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(worktree.exists());
    assert_eq!(result.reports.len(), 2);
    assert!(
        result
            .reports
            .iter()
            .all(|report| report.action == "skipped:ambiguous_run_path")
    );
}

#[test]
fn resolver_uses_workspace_local_root_without_override() {
    let _lock = WORKTREE_ROOT_ENV_LOCK.lock().unwrap();
    let old = std::env::var_os("ORBIT_WORKTREE_ROOT");
    // SAFETY: this module serializes mutations of this test-only variable and
    // restores its prior value before releasing the lock.
    unsafe { std::env::remove_var("ORBIT_WORKTREE_ROOT") };
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    let path = resolve_worktree_path_from_prefix(&repo, "orbit", "jrun-local").unwrap();
    assert_eq!(path, repo.join(".orbit/state/worktrees/orbit-jrun-local"));
    restore_worktree_root(old);
}

#[test]
fn resolver_uses_external_root_and_repository_name_when_configured() {
    let _lock = WORKTREE_ROOT_ENV_LOCK.lock().unwrap();
    let old = std::env::var_os("ORBIT_WORKTREE_ROOT");
    let temp = tempdir().unwrap();
    let repo = temp.path().join("my-repo");
    let root = temp.path().join("worktrees");
    // SAFETY: this module serializes mutations of this test-only variable and
    // restores its prior value before releasing the lock.
    unsafe { std::env::set_var("ORBIT_WORKTREE_ROOT", &root) };
    let path = resolve_worktree_path_from_prefix(&repo, "orbit", "jrun-external").unwrap();
    assert_eq!(path, root.join("my-repo/orbit-jrun-external"));
    restore_worktree_root(old);
}

fn run(id: &str, state: JobRunState) -> JobRun {
    let now = Utc::now();
    JobRun {
        run_id: id.to_string(),
        job_id: "task_pr_pipeline".to_string(),
        attempt: 1,
        state,
        scheduled_at: now,
        started_at: Some(now),
        finished_at: Some(now),
        duration_ms: Some(1),
        created_at: now,
        pid: None,
        pid_start_time: None,
        input: Some(json!({"task_id": "ORB-10374"})),
        retry_source_run_id: None,
        knowledge_metrics: None,
        resolved_crew: None,
        crew_model: None,
        steps: Vec::new(),
    }
}

fn resolved_task_worktree(repo: &Path, run: &JobRun) -> PathBuf {
    resolve_worktree_path_from_prefix(repo, "orbit", &run.run_id).unwrap()
}

fn init_repo(path: &Path) {
    fs::create_dir_all(path).unwrap();
    git(path, &["init"]);
    git(path, &["checkout", "-b", "agent-main"]);
    git(path, &["config", "user.name", "Orbit Test"]);
    git(path, &["config", "user.email", "orbit-test@example.com"]);
    fs::write(path.join("base.txt"), "base").unwrap();
    git(path, &["add", "base.txt"]);
    git(path, &["commit", "-m", "base"]);
}

fn add_worktree(repo: &Path, path: &Path, branch: &str) {
    git(
        repo,
        &[
            "worktree",
            "add",
            "-b",
            branch,
            path.to_str().unwrap(),
            "HEAD",
        ],
    );
}

fn git(current_dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(current_dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed in {}:\n{}",
        args.join(" "),
        current_dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn restore_worktree_root(value: Option<std::ffi::OsString>) {
    // SAFETY: callers hold WORKTREE_ROOT_ENV_LOCK.
    unsafe {
        match value {
            Some(value) => std::env::set_var("ORBIT_WORKTREE_ROOT", value),
            None => std::env::remove_var("ORBIT_WORKTREE_ROOT"),
        }
    }
}
