//! [ORB-10173] Worktree GC tests: the pure reclaim-disposition classifier
//! (which encodes the concurrency-safety rule) plus an end-to-end reclaim over
//! a real git repo with two worktrees.

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{Duration, Utc};
use orbit_common::types::{JobRun, JobRunState};

use super::test_runtime;
use crate::command::job::run::gc::{
    WorktreeDisposition, WorktreeGcOptions, classify_worktree, extract_run_id, is_protected,
};

const RETENTION_DAYS: i64 = 7;

/// Minimal `JobRun` with a given state; timestamps default to "just now".
fn fake_run(state: JobRunState) -> JobRun {
    let now = Utc::now();
    JobRun {
        run_id: "jrun-20260712-2021-4".to_string(),
        job_id: "task_pr_pipeline".to_string(),
        attempt: 1,
        state,
        scheduled_at: now,
        started_at: Some(now),
        finished_at: Some(now),
        duration_ms: Some(0),
        created_at: now,
        pid: None,
        pid_start_time: None,
        input: None,
        retry_source_run_id: None,
        knowledge_metrics: None,
        resolved_crew: None,
        crew_model: None,
        steps: Vec::new(),
    }
}

#[test]
fn live_running_run_is_never_reclaimed() {
    // A running run whose owner process is alive (this test process) must be
    // kept — this is the concurrency-safety guarantee.
    let mut run = fake_run(JobRunState::Running);
    run.pid = Some(std::process::id());
    run.pid_start_time = None; // LegacyLiveUnverified → alive → not stale.
    assert_eq!(
        classify_worktree(Some(&run), RETENTION_DAYS, Utc::now()),
        WorktreeDisposition::KeepLive,
    );
}

#[test]
fn recently_created_pending_run_is_kept() {
    let mut run = fake_run(JobRunState::Pending);
    run.pid = None; // never claimed…
    run.created_at = Utc::now(); // …but inside the unclaimed grace window.
    assert_eq!(
        classify_worktree(Some(&run), RETENTION_DAYS, Utc::now()),
        WorktreeDisposition::KeepLive,
    );
}

#[test]
fn running_run_with_dead_owner_is_orphan_reclaimed() {
    let mut run = fake_run(JobRunState::Running);
    // A PID that is conclusively gone. Signal-0 to a huge PID returns ESRCH.
    run.pid = Some(0x7FFF_FFFE);
    run.pid_start_time = None;
    assert_eq!(
        classify_worktree(Some(&run), RETENTION_DAYS, Utc::now()),
        WorktreeDisposition::ReclaimOrphanRecord,
    );
}

#[test]
fn success_run_reaps_immediately() {
    let run = fake_run(JobRunState::Success);
    assert_eq!(
        classify_worktree(Some(&run), RETENTION_DAYS, Utc::now()),
        WorktreeDisposition::ReclaimTerminal,
    );
}

#[test]
fn cancelled_run_reaps_immediately() {
    let run = fake_run(JobRunState::Cancelled);
    assert_eq!(
        classify_worktree(Some(&run), RETENTION_DAYS, Utc::now()),
        WorktreeDisposition::ReclaimTerminal,
    );
}

#[test]
fn failed_run_is_retained_inside_window_and_reaped_after() {
    let mut run = fake_run(JobRunState::Failed);
    let now = Utc::now();
    run.finished_at = Some(now - Duration::days(1));
    assert_eq!(
        classify_worktree(Some(&run), RETENTION_DAYS, now),
        WorktreeDisposition::Retain,
    );

    run.finished_at = Some(now - Duration::days(RETENTION_DAYS + 1));
    assert_eq!(
        classify_worktree(Some(&run), RETENTION_DAYS, now),
        WorktreeDisposition::ReclaimTerminal,
    );
}

#[test]
fn interrupted_run_is_retained_inside_window() {
    let mut run = fake_run(JobRunState::Interrupted);
    let now = Utc::now();
    run.finished_at = Some(now - Duration::hours(1));
    assert_eq!(
        classify_worktree(Some(&run), RETENTION_DAYS, now),
        WorktreeDisposition::Retain,
    );
}

#[test]
fn zero_retention_reaps_failures_immediately() {
    let run = fake_run(JobRunState::Failed);
    assert_eq!(
        classify_worktree(Some(&run), 0, Utc::now()),
        WorktreeDisposition::ReclaimTerminal,
    );
}

#[test]
fn missing_run_record_is_reclaimed() {
    assert_eq!(
        classify_worktree(None, RETENTION_DAYS, Utc::now()),
        WorktreeDisposition::ReclaimNoRecord,
    );
}

#[test]
fn extract_run_id_handles_known_prefixes() {
    assert_eq!(
        extract_run_id(Path::new("/w/orbit-jrun-20260712-2021-4")).as_deref(),
        Some("jrun-20260712-2021-4"),
    );
    assert_eq!(
        extract_run_id(Path::new("/w/parallel-batch-jrun-20260712-0019-5")).as_deref(),
        Some("jrun-20260712-0019-5"),
    );
    assert_eq!(extract_run_id(Path::new("/w/some-other-dir")), None);
}

#[test]
fn is_protected_matches_self_and_ancestors() {
    let dir = Path::new("/w/orbit-jrun-1");
    assert!(is_protected(dir, Some(Path::new("/w/orbit-jrun-1"))));
    assert!(is_protected(dir, Some(Path::new("/w/orbit-jrun-1/sub"))));
    assert!(!is_protected(dir, Some(Path::new("/w/orbit-jrun-2"))));
    assert!(!is_protected(dir, None));
}

// --- End-to-end reclaim over a real git repo -------------------------------

fn git(repo: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Returns `false` when git is unavailable / the sandbox blocks subprocesses,
/// so the caller can skip rather than fail on an environment limitation.
fn init_repo(repo: &Path) -> bool {
    std::fs::create_dir_all(repo).expect("create repo dir");
    if !git(repo, &["init", "-q"]) {
        return false;
    }
    git(repo, &["config", "user.email", "gc@test"]);
    git(repo, &["config", "user.name", "gc test"]);
    std::fs::write(repo.join("README.md"), "seed").expect("write seed");
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", "seed"])
}

fn add_worktree(repo: &Path, worktree: &Path, branch: &str) -> bool {
    git(
        repo,
        &[
            "worktree",
            "add",
            "-b",
            branch,
            &worktree.to_string_lossy(),
            "HEAD",
        ],
    )
}

#[test]
fn gc_reclaims_terminal_worktree_and_keeps_live_run() {
    let (_tmp, runtime) = test_runtime();
    let repo = runtime.paths().repo_root.clone();
    if !init_repo(&repo) {
        // git unavailable / subprocesses blocked in this environment — the pure
        // classifier tests already cover the reclaim decision; skip the
        // git-backed end-to-end assertion rather than fail on a sandbox limit.
        return;
    }
    let worktrees_dir = runtime.paths().worktrees_dir.clone();
    std::fs::create_dir_all(&worktrees_dir).expect("create worktrees dir");

    // Terminal (success) run + its worktree, seeded with build-artifact junk
    // to prove the reclaim is language-agnostic.
    let done = runtime
        .stores()
        .jobs()
        .insert_run("task_pr_pipeline", 1, Utc::now(), None, None)
        .expect("insert done run");
    runtime
        .stores()
        .jobs()
        .mark_run_running(&done.run_id, Utc::now(), std::process::id())
        .expect("mark done running");
    runtime
        .stores()
        .jobs()
        .finalize_run(&done.run_id, JobRunState::Success, Utc::now(), Some(0))
        .expect("finalize done run");
    let done_wt: PathBuf = worktrees_dir.join(format!("orbit-{}", done.run_id));
    assert!(add_worktree(&repo, &done_wt, "orbit/gc-done"));
    std::fs::create_dir_all(done_wt.join("target/debug")).expect("mk target");
    std::fs::write(done_wt.join("target/debug/artifact.bin"), vec![0u8; 4096]).expect("junk");

    // Live (running) run owned by this process + its worktree.
    let live = runtime
        .stores()
        .jobs()
        .insert_run("task_pr_pipeline", 1, Utc::now(), None, None)
        .expect("insert live run");
    runtime
        .stores()
        .jobs()
        .mark_run_running(&live.run_id, Utc::now(), std::process::id())
        .expect("mark live running");
    let live_wt: PathBuf = worktrees_dir.join(format!("orbit-{}", live.run_id));
    assert!(add_worktree(&repo, &live_wt, "orbit/gc-live"));

    let outcome = runtime
        .gc_worktrees(&WorktreeGcOptions::default())
        .expect("gc worktrees");

    assert!(
        !done_wt.exists(),
        "terminal-success worktree (with build junk) must be reclaimed",
    );
    assert!(
        live_wt.exists(),
        "live running run's worktree must never be reclaimed",
    );
    assert_eq!(outcome.reclaimed, 1, "exactly one worktree reclaimed");

    let live_entry = outcome
        .entries
        .iter()
        .find(|e| e.run_id.as_deref() == Some(live.run_id.as_str()))
        .expect("live entry present");
    assert_eq!(live_entry.action.as_str(), "kept_live");
}
