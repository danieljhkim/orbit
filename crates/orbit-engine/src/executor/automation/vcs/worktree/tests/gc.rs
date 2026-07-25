#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use chrono::Utc;
use orbit_common::types::{
    ExternalRef, JobRun, JobRunState, NotFoundKind, OrbitError, Task, TaskArtifact, TaskPriority,
    TaskStatus, TaskType,
};
use serde_json::json;
use tempfile::tempdir;

use crate::context::TaskReadHost;

use super::super::gc::{WorktreeGcOptions, collect_worktrees};
use super::super::{resolve_shared_worktree_path, resolve_worktree_path_from_prefix};

static WORKTREE_ROOT_ENV_LOCK: Mutex<()> = Mutex::new(());

struct FakeTaskHost {
    tasks: BTreeMap<String, Task>,
}

impl FakeTaskHost {
    fn new(tasks: Vec<Task>) -> Self {
        Self {
            tasks: tasks
                .into_iter()
                .map(|task| (task.id.clone(), task))
                .collect(),
        }
    }
}

impl TaskReadHost for FakeTaskHost {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        self.tasks
            .get(task_id)
            .cloned()
            .ok_or_else(|| OrbitError::not_found(NotFoundKind::Task, task_id.to_string()))
    }

    fn get_task_artifacts(&self, _task_id: &str) -> Result<Vec<TaskArtifact>, OrbitError> {
        Ok(Vec::new())
    }

    fn list_tasks_filtered(
        &self,
        _status: Option<TaskStatus>,
        _priority: Option<TaskPriority>,
        _parent_id: Option<&str>,
        _job_run_id: Option<&str>,
        _external_ref: Option<&ExternalRef>,
        _has_external_ref_system: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        Ok(self.tasks.values().cloned().collect())
    }
}

fn task_fixture(id: &str, status: TaskStatus) -> Task {
    let now = Utc::now();
    Task {
        id: id.to_string(),
        title: "fixture task".to_string(),
        description: String::new(),
        acceptance_criteria: Vec::new(),
        tags: Vec::new(),
        plan: String::new(),
        execution_summary: String::new(),
        context_files: Vec::new(),
        created_by: None,
        planned_by: None,
        implemented_by: None,
        status,
        priority: TaskPriority::Medium,
        complexity: None,
        task_type: TaskType::Chore,
        pr_status: None,
        external_refs: Vec::new(),
        relations: Vec::new(),
        job_run_id: None,
        crew: None,
        created_at: now,
        updated_at: now,
    }
}

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

    let host = FakeTaskHost::new(Vec::new());
    let result = collect_worktrees(
        &repo,
        &[],
        &host,
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
    let run = run("jrun-dirty", JobRunState::Failed, Some("ORB-DIRTY"));
    let worktree = resolved_task_worktree(&repo, &run);
    add_worktree(&repo, &worktree, "orbit/dirty");
    fs::write(worktree.join("uncommitted.txt"), "valuable").unwrap();
    let host = FakeTaskHost::new(vec![task_fixture("ORB-DIRTY", TaskStatus::Done)]);

    let result = collect_worktrees(
        &repo,
        &[run],
        &host,
        &WorktreeGcOptions {
            delete: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(worktree.exists());
    assert_eq!(result.reports[0].action, "skipped:dirty_rescue_candidate");
    assert_eq!(result.reports[0].bytes_reclaimed, 0);
    assert_eq!(result.reports[0].task_status, Some(TaskStatus::Done));
}

#[test]
fn dry_run_and_yes_share_eligibility_but_only_yes_removes() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    let run = run("jrun-clean", JobRunState::Success, Some("ORB-CLEAN"));
    let worktree = resolved_task_worktree(&repo, &run);
    add_worktree(&repo, &worktree, "orbit/clean");
    fs::write(worktree.join("bytes.bin"), [1_u8; 32]).unwrap();
    git(&worktree, &["add", "bytes.bin"]);
    git(&worktree, &["commit", "-m", "worktree content"]);
    let host = FakeTaskHost::new(vec![task_fixture("ORB-CLEAN", TaskStatus::Done)]);

    let dry = collect_worktrees(
        &repo,
        std::slice::from_ref(&run),
        &host,
        &WorktreeGcOptions::default(),
    )
    .unwrap();
    assert!(worktree.exists());
    assert_eq!(dry.reports[0].action, "would_remove");
    assert_eq!(dry.reports[0].task_id.as_deref(), Some("ORB-CLEAN"));

    let applied = collect_worktrees(
        &repo,
        &[run],
        &host,
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
    let first = run(
        "jrun:collision",
        JobRunState::Success,
        Some("ORB-COLLISION"),
    );
    let second = run(
        "jrun-collision",
        JobRunState::Success,
        Some("ORB-COLLISION"),
    );
    let worktree = resolved_task_worktree(&repo, &first);
    assert_eq!(worktree, resolved_task_worktree(&repo, &second));
    add_worktree(&repo, &worktree, "orbit/collision");
    let host = FakeTaskHost::new(vec![task_fixture("ORB-COLLISION", TaskStatus::Done)]);

    let result = collect_worktrees(
        &repo,
        &[first, second],
        &host,
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
fn terminal_run_with_blocked_task_is_retained() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    let run = run("jrun-blocked", JobRunState::Success, Some("ORB-BLOCKED"));
    let worktree = resolved_task_worktree(&repo, &run);
    add_worktree(&repo, &worktree, "orbit/blocked");
    let host = FakeTaskHost::new(vec![task_fixture("ORB-BLOCKED", TaskStatus::Blocked)]);

    let result = collect_worktrees(
        &repo,
        &[run],
        &host,
        &WorktreeGcOptions {
            delete: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(worktree.exists());
    assert_eq!(result.reports[0].action, "skipped:task_status_ineligible");
    assert_eq!(result.reports[0].task_id.as_deref(), Some("ORB-BLOCKED"));
    assert_eq!(result.reports[0].task_status, Some(TaskStatus::Blocked));
}

#[test]
fn terminal_run_with_review_task_is_retained() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    let run = run("jrun-review", JobRunState::Success, Some("ORB-REVIEW"));
    let worktree = resolved_task_worktree(&repo, &run);
    add_worktree(&repo, &worktree, "orbit/review");
    let host = FakeTaskHost::new(vec![task_fixture("ORB-REVIEW", TaskStatus::Review)]);

    let result = collect_worktrees(
        &repo,
        &[run],
        &host,
        &WorktreeGcOptions {
            delete: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(worktree.exists());
    assert_eq!(result.reports[0].action, "skipped:task_status_ineligible");
    assert_eq!(result.reports[0].task_status, Some(TaskStatus::Review));
}

#[test]
fn unattributed_run_is_retained_and_reported() {
    let temp = tempdir().unwrap();
    let repo = temp.path().join("repo");
    init_repo(&repo);
    let run = run("jrun-unattributed", JobRunState::Success, None);
    let worktree = resolve_shared_worktree_path(&repo, &run.run_id).unwrap();
    add_worktree(&repo, &worktree, "orbit/unattributed");
    let host = FakeTaskHost::new(Vec::new());

    let result = collect_worktrees(
        &repo,
        &[run],
        &host,
        &WorktreeGcOptions {
            delete: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(worktree.exists());
    assert_eq!(result.reports[0].action, "skipped:unattributed");
    assert_eq!(result.reports[0].task_id, None);
    assert_eq!(result.reports[0].task_status, None);
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

fn run(id: &str, state: JobRunState, task_id: Option<&str>) -> JobRun {
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
        input: Some(match task_id {
            Some(task_id) => json!({"task_id": task_id}),
            None => json!({}),
        }),
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
