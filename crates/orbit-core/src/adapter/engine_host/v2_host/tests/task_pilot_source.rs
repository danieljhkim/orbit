//! Git source-snapshot fixtures for task-pilot prepare/apply [ORB-11236].

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use orbit_types::task::{Task, TaskPriority, TaskStatus, TaskType};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::super::task_pilot::{apply, prepare};
use crate::OrbitRuntime;
use crate::adapter::engine_host::v2_host::test_support::runtime_with_workspace_layout;
use crate::application::task::TaskAddParams;

const LANDING: &str = "agent-main";

struct RemoteLandingFixture {
    _root: TempDir,
    runtime: OrbitRuntime,
    repo: PathBuf,
    seed: PathBuf,
    stale_sha: String,
    current_sha: String,
    task: Task,
}

fn seed_task(runtime: &OrbitRuntime, title: &str) -> Task {
    runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            description: format!("Fixture task: {title}"),
            acceptance_criteria: vec!["The fixture outcome is observable.".to_string()],
            plan: "Inspect and update the fixture.".to_string(),
            workspace_path: Some(".".to_string()),
            priority: TaskPriority::Medium,
            task_type: Some(TaskType::Chore),
            status: Some(TaskStatus::Backlog),
            ..TaskAddParams::default()
        })
        .expect("seed task")
}

fn selector_assessment(task: &Task, after: Vec<&str>) -> Value {
    json!({
        "task_id": task.id,
        "context_files_before": task.context_files,
        "context_files_after": after,
        "disposition": "selectors",
        "recommended_crew": "luna",
        "recommended_complexity": "medium",
        "blocked_by": [],
        "duplicate_of": null,
        "already_landed": null,
        "adr_conflicts": [],
        "utility_warnings": [],
        "surface_warnings": [],
    })
}

fn apply_selectors(
    runtime: &OrbitRuntime,
    prepared: &Value,
    task: &Task,
    after: Vec<&str>,
) -> Value {
    apply(
        runtime,
        "apply_task_pilot_results",
        &json!({
            "prepared": prepared,
            "results": [{
                "partition_index": 0,
                "task_ids": [task.id.clone()],
                "tasks": [selector_assessment(task, after)],
                "summary": "fixture partition",
            }],
            "workspace_path": prepared["workspace_path"],
        }),
    )
    .expect("apply partition is a durable output")
}

fn git(current_dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(current_dir)
        .output()
        .unwrap_or_else(|error| panic!("spawn git {}: {error}", args.join(" ")));
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

fn git_allow_fail(current_dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(current_dir)
        .output()
        .unwrap_or_else(|error| panic!("spawn git {}: {error}", args.join(" ")))
}

fn init_repo(path: &Path, branch: &str) {
    fs::create_dir_all(path).expect("create repo dir");
    git(path, &["init"]);
    git(path, &["checkout", "-b", branch]);
    git(path, &["config", "user.name", "Orbit Test"]);
    git(path, &["config", "user.email", "orbit-test@example.com"]);
    git(path, &["config", "commit.gpgsign", "false"]);
}

fn commit_file(repo: &Path, relative: &str, contents: &str) -> String {
    let path = repo.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(&path, contents).expect("write file");
    git(repo, &["add", relative]);
    git(repo, &["commit", "-m", &format!("write {relative}")]);
    git(repo, &["rev-parse", "HEAD"])
}

fn remote_landing_fixture() -> RemoteLandingFixture {
    let (root, runtime, repo) = runtime_with_workspace_layout();
    let remote = root.path().join("remote.git");
    let seed = root.path().join("seed");
    git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
    init_repo(&repo, LANDING);
    fs::create_dir_all(repo.join("src")).expect("src dir");
    fs::write(repo.join(".gitignore"), ".orbit/\n").expect("ignore orbit store");
    fs::write(repo.join("src/existing.rs"), "existing\n").expect("write existing");
    git(&repo, &["add", ".gitignore", "src/existing.rs"]);
    git(&repo, &["commit", "-m", "existing target"]);
    git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&repo, &["push", "-u", "origin", LANDING]);
    let stale_sha = git(&repo, &["rev-parse", "HEAD"]);

    git(
        root.path(),
        &[
            "clone",
            "--branch",
            LANDING,
            remote.to_str().unwrap(),
            seed.to_str().unwrap(),
        ],
    );
    git(&seed, &["config", "user.name", "Orbit Test"]);
    git(&seed, &["config", "user.email", "orbit-test@example.com"]);
    git(&seed, &["config", "commit.gpgsign", "false"]);
    let current_sha = commit_file(&seed, "src/merged.rs", "newly merged\n");
    git(&seed, &["push", "origin", LANDING]);

    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), stale_sha);
    assert!(!repo.join("src/merged.rs").exists());

    let task = seed_task(&runtime, "merged target");
    RemoteLandingFixture {
        _root: root,
        runtime,
        repo,
        seed,
        stale_sha,
        current_sha,
        task,
    }
}

fn prepare_landing(fixture: &RemoteLandingFixture) -> Result<Value, String> {
    prepare(
        &fixture.runtime,
        "prepare_task_pilot",
        &json!({
            "task_ids": [fixture.task.id.clone()],
            "workspace_path": fixture.repo,
            "base_branch": LANDING,
        }),
    )
    .map_err(|error| error.to_string())
}

#[test]
fn clean_stale_primary_fast_forwards_and_apply_admits_newly_merged_file() {
    let fixture = remote_landing_fixture();
    let prepared = prepare_landing(&fixture).expect("clean stale primary prepares");

    assert_eq!(prepared["source"]["base_branch"], LANDING);
    assert_eq!(
        prepared["source"]["source_ref"],
        format!("origin/{LANDING}")
    );
    assert_eq!(prepared["source"]["source_revision"], fixture.current_sha);
    assert_eq!(prepared["source"]["fast_forwarded"], true);
    assert_eq!(
        git(&fixture.repo, &["rev-parse", "HEAD"]),
        fixture.current_sha
    );
    assert!(fixture.repo.join("src/merged.rs").exists());

    let output = apply_selectors(
        &fixture.runtime,
        &prepared,
        &fixture.task,
        vec!["file:src/merged.rs"],
    );
    assert_eq!(output["status"], "succeeded");
    assert_eq!(output["source"]["source_revision"], fixture.current_sha);
    assert_eq!(
        fixture
            .runtime
            .get_task(&fixture.task.id)
            .unwrap()
            .context_files,
        vec!["file:src/merged.rs"]
    );
}

#[test]
fn dirty_primary_reports_source_staleness_without_moving_head() {
    let fixture = remote_landing_fixture();
    fs::write(fixture.repo.join("src/existing.rs"), "dirty local edit\n")
        .expect("dirty tracked file");
    let before = git(&fixture.repo, &["rev-parse", "HEAD"]);

    let error = prepare_landing(&fixture).expect_err("dirty primary must not spend an agent call");
    assert!(
        error.contains("source-staleness"),
        "expected source-staleness, got {error}"
    );
    assert!(error.contains(&fixture.current_sha));
    assert!(error.contains(&fixture.stale_sha));
    assert_eq!(git(&fixture.repo, &["rev-parse", "HEAD"]), before);
    assert!(!fixture.repo.join("src/merged.rs").exists());
}

#[test]
fn untracked_file_on_stale_primary_does_not_reset_or_admit_later_paths() {
    let fixture = remote_landing_fixture();
    fs::write(fixture.repo.join("scratch.txt"), "untracked\n").expect("untracked file");
    let before = git(&fixture.repo, &["rev-parse", "HEAD"]);

    let error = prepare_landing(&fixture).expect_err("untracked files block fast-forward");
    assert!(error.contains("source-staleness"), "{error}");
    assert!(
        error.contains("untracked") || error.contains("dirty"),
        "{error}"
    );
    assert_eq!(git(&fixture.repo, &["rev-parse", "HEAD"]), before);
    assert!(
        git_allow_fail(&fixture.repo, &["status", "--porcelain"])
            .status
            .success()
    );
    let status_output = git_allow_fail(&fixture.repo, &["status", "--porcelain"]);
    let status = String::from_utf8_lossy(&status_output.stdout);
    assert!(status.contains("scratch.txt"));
}

#[test]
fn remote_failure_is_closed_without_git_writes() {
    let fixture = remote_landing_fixture();
    git(
        &fixture.repo,
        &[
            "remote",
            "set-url",
            "origin",
            "/no/such/orbit-pilot-remote.git",
        ],
    );
    let before = git(&fixture.repo, &["rev-parse", "HEAD"]);

    let error = prepare_landing(&fixture).expect_err("broken origin must fail closed");
    assert!(
        error.contains("remote failure") || error.contains("could not fetch"),
        "{error}"
    );
    assert_eq!(git(&fixture.repo, &["rev-parse", "HEAD"]), before);
    assert!(!fixture.repo.join("src/merged.rs").exists());
}

#[test]
fn apply_uses_pinned_revision_after_concurrent_branch_advancement() {
    let fixture = remote_landing_fixture();
    let prepared = prepare_landing(&fixture).expect("prepare pins current landing tip");
    let pinned = prepared["source"]["source_revision"]
        .as_str()
        .expect("pinned sha")
        .to_string();
    assert_eq!(pinned, fixture.current_sha);

    commit_file(&fixture.seed, "src/later.rs", "landed after prepare\n");
    git(&fixture.seed, &["push", "origin", LANDING]);
    git(&fixture.repo, &["fetch", "origin", LANDING]);
    git(
        &fixture.repo,
        &["merge", "--ff-only", &format!("origin/{LANDING}")],
    );
    assert!(fixture.repo.join("src/later.rs").exists());

    let later = apply_selectors(
        &fixture.runtime,
        &prepared,
        &fixture.task,
        vec!["file:src/later.rs"],
    );
    assert_eq!(later["status"], "failed");
    let error = later["partition_decisions"][0]["error"]
        .as_str()
        .expect("failed partition error");
    assert!(error.contains("does not resolve"), "{error}");
    assert!(error.contains(&pinned), "{error}");
    assert!(
        fixture
            .runtime
            .get_task(&fixture.task.id)
            .unwrap()
            .context_files
            .is_empty()
    );

    let merged = apply_selectors(
        &fixture.runtime,
        &prepared,
        &fixture.task,
        vec!["file:src/merged.rs"],
    );
    assert_eq!(merged["status"], "succeeded");
    assert_eq!(
        fixture
            .runtime
            .get_task(&fixture.task.id)
            .unwrap()
            .context_files,
        vec!["file:src/merged.rs"]
    );
}

#[test]
fn untracked_workspace_file_is_not_admitted_against_the_source_snapshot() {
    let fixture = remote_landing_fixture();
    let prepared = prepare_landing(&fixture).expect("fast-forward to landing tip");
    fs::write(fixture.repo.join("src/ghost.rs"), "working tree only\n").expect("untracked ghost");

    let output = apply_selectors(
        &fixture.runtime,
        &prepared,
        &fixture.task,
        vec!["file:src/ghost.rs"],
    );
    assert_eq!(output["status"], "failed");
    assert!(
        output["partition_decisions"][0]["error"]
            .as_str()
            .unwrap()
            .contains("does not resolve")
    );
    assert!(
        fixture
            .runtime
            .get_task(&fixture.task.id)
            .unwrap()
            .context_files
            .is_empty()
    );
}

#[test]
fn non_git_workspace_still_uses_filesystem_existence() {
    let (_root, runtime, repo_root) = runtime_with_workspace_layout();
    fs::create_dir_all(repo_root.join("src")).expect("src");
    fs::write(repo_root.join("src/alpha.rs"), "fn alpha() {}\n").expect("alpha");
    let task = seed_task(&runtime, "filesystem fallback");
    let prepared = prepare(
        &runtime,
        "prepare_task_pilot",
        &json!({
            "task_ids": [task.id.clone()],
            "workspace_path": repo_root,
            "base_branch": LANDING,
        }),
    )
    .expect("non-git prepare");
    assert_eq!(prepared["source"]["source_revision"], Value::Null);

    let output = apply_selectors(&runtime, &prepared, &task, vec!["file:src/alpha.rs"]);
    assert_eq!(output["status"], "succeeded");
}
