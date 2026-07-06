//! qa-sweep pass tests [ORB-10039]: end-to-end over a seeded global root and
//! a real temp git repo — watermark advance/hold, ledger run recording,
//! fingerprint dedupe, mute handling, branch guard, dry-run inertness.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use chrono::Utc;
use orbit_common::types::{
    JobRunState, TaskPriority, TaskStatus, Workspace, WorkspaceRegistry, WorkspaceStatus,
};
use tempfile::TempDir;

use crate::OrbitRuntime;
use crate::qa::state::{load_state, state_path};
use crate::qa::sweep::execute_check;
use crate::qa::{QA_SWEEP_JOB, QA_SWEEP_TAG, QaSweepOptions, run_qa_sweep_at};
use crate::workspace_registry;

const WS_NAME: &str = "polaris";

struct Fixture {
    _tmp: TempDir,
    global: PathBuf,
    repo: PathBuf,
    orbit_dir: PathBuf,
}

impl Fixture {
    /// Sweep once (non-dry) and return the single workspace report.
    fn sweep(&self) -> crate::qa::QaWorkspaceReport {
        self.sweep_with(QaSweepOptions::default())
    }

    fn sweep_with(&self, options: QaSweepOptions) -> crate::qa::QaWorkspaceReport {
        let mut outcome = run_qa_sweep_at(&self.global, options).expect("sweep pass");
        assert!(!outcome.lock_busy, "pass lock unexpectedly busy");
        assert_eq!(outcome.reports.len(), 1, "one configured workspace");
        outcome.reports.remove(0)
    }

    fn runtime(&self) -> OrbitRuntime {
        OrbitRuntime::from_roots(&self.global, &self.orbit_dir).expect("open workspace runtime")
    }

    fn watermark(&self) -> Option<String> {
        load_state(&state_path(&self.global))
            .workspaces
            .get(WS_NAME)
            .map(|watermark| watermark.last_validated_sha.clone())
    }

    fn head(&self) -> String {
        git_stdout(&self.repo, &["rev-parse", "HEAD"])
    }

    fn commit(&self, name: &str) {
        std::fs::write(self.repo.join(name), name).expect("write file");
        git(&self.repo, &["add", "."]);
        git(&self.repo, &["commit", "-m", &format!("add {name}")]);
    }
}

/// Seed a global root + registered git workspace whose `[qa]` section carries
/// the given `[[qa.workspace.check]]` entries (TOML snippet).
fn fixture(checks_toml: &str) -> Fixture {
    fixture_with_qa(&format!(
        "[qa]\n\n[[qa.workspace]]\nname = \"{WS_NAME}\"\n{checks_toml}"
    ))
}

fn fixture_with_qa(qa_toml: &str) -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let global = tmp.path().join("global");
    let repo = tmp.path().join(WS_NAME);
    let orbit_dir = repo.join(".orbit");
    std::fs::create_dir_all(&global).expect("global root");
    std::fs::create_dir_all(&orbit_dir).expect("workspace orbit dir");

    std::fs::write(global.join("config.toml"), qa_toml).expect("write global config");

    git(&repo, &["init", "--quiet"]);
    git(&repo, &["checkout", "-q", "-b", "agent-main"]);
    std::fs::write(repo.join("README.md"), "seed").expect("seed file");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "seed"]);

    let mut registry = WorkspaceRegistry::default();
    registry.workspaces.push(Workspace {
        id: "ws-qa-test".to_string(),
        name: WS_NAME.to_string(),
        root: repo.clone(),
        orbit_dir: orbit_dir.clone(),
        git_remote: None,
        base_branch: "agent-main".to_string(),
        status: WorkspaceStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    });
    workspace_registry::save_registry_to(
        &registry,
        &workspace_registry::registry_path_for(&global),
    )
    .expect("save registry");

    Fixture {
        _tmp: tmp,
        global,
        repo,
        orbit_dir,
    }
}

fn git(repo: &Path, args: &[&str]) {
    let status = git_command(repo, args).status().expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = git_command(repo, args).output().expect("run git");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn git_command(repo: &Path, args: &[&str]) -> Command {
    std::fs::create_dir_all(repo).expect("repo dir");
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "qa")
        .env("GIT_AUTHOR_EMAIL", "qa@test")
        .env("GIT_COMMITTER_NAME", "qa")
        .env("GIT_COMMITTER_EMAIL", "qa@test")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null");
    command
}

const PASSING_CHECK: &str = "[[qa.workspace.check]]\nname = \"ok\"\ncommand = \"echo fine\"\n";
const FAILING_CHECK: &str =
    "[[qa.workspace.check]]\nname = \"broken\"\ncommand = \"echo boom >&2; exit 3\"\n";

// ---- green / skip / watermark ---------------------------------------------

#[test]
fn green_pass_records_ledger_run_and_advances_watermark() {
    let fixture = fixture(PASSING_CHECK);
    let head = fixture.head();

    let report = fixture.sweep();
    assert_eq!(report.action, "validated", "reason: {:?}", report.reason);
    assert_eq!(report.baseline, None, "first validation has no baseline");
    assert_eq!(report.head.as_deref(), Some(head.as_str()));
    assert_eq!(report.checks.len(), 1);
    assert_eq!(report.checks[0].outcome, "passed");
    assert_eq!(report.checks[0].exit_code, Some(0));

    // Watermark advanced to the validated HEAD.
    assert_eq!(fixture.watermark().as_deref(), Some(head.as_str()));

    // The pass is a first-class ledger run with one step per executed check.
    let runtime = fixture.runtime();
    let runs = runtime.job_history(QA_SWEEP_JOB).expect("qa_sweep history");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].state, JobRunState::Success);
    assert_eq!(runs[0].run_id, report.run_id.expect("run id reported"));
    assert_eq!(runs[0].steps.len(), 1);
    assert_eq!(runs[0].steps[0].state, JobRunState::Success);
}

#[test]
fn unchanged_head_is_skipped_without_a_run() {
    let fixture = fixture(PASSING_CHECK);
    assert_eq!(fixture.sweep().action, "validated");

    let report = fixture.sweep();
    assert_eq!(report.action, "skipped");
    assert_eq!(report.reason.as_deref(), Some("no_new_commits"));
    assert!(report.run_id.is_none());

    let runtime = fixture.runtime();
    assert_eq!(
        runtime.job_history(QA_SWEEP_JOB).expect("history").len(),
        1,
        "skip records no second run"
    );
}

#[test]
fn new_commits_are_revalidated_with_the_range_reported() {
    let fixture = fixture(PASSING_CHECK);
    assert_eq!(fixture.sweep().action, "validated");
    let baseline = fixture.watermark().expect("baseline");

    fixture.commit("one.txt");
    fixture.commit("two.txt");
    let head = fixture.head();

    let report = fixture.sweep();
    assert_eq!(report.action, "validated");
    assert_eq!(report.baseline.as_deref(), Some(baseline.as_str()));
    assert_eq!(report.new_commits.as_ref().map(Vec::len), Some(2));
    assert_eq!(fixture.watermark().as_deref(), Some(head.as_str()));
}

// ---- failures: task filing, dedupe, watermark hold -------------------------

#[test]
fn failing_check_files_a_tagged_task_and_holds_the_watermark() {
    let fixture = fixture(FAILING_CHECK);
    let report = fixture.sweep();

    assert_eq!(report.action, "failed");
    assert_eq!(report.checks[0].outcome, "failed");
    assert_eq!(report.checks[0].exit_code, Some(3));
    let fingerprint = report.checks[0].fingerprint.clone().expect("fingerprint");
    let task_id = report.checks[0].filed_task.clone().expect("task filed");
    assert!(report.checks[0].deduped_task.is_none());

    // Failure never advances the watermark.
    assert_eq!(fixture.watermark(), None);

    // The task carries evidence and the dedupe tags.
    let runtime = fixture.runtime();
    let tasks = runtime.list_tasks().expect("list tasks");
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.id, task_id);
    assert_eq!(task.status, TaskStatus::Backlog, "default files as backlog");
    assert_eq!(task.priority, TaskPriority::Medium);
    assert!(task.tags.contains(&QA_SWEEP_TAG.to_string()));
    assert!(task.tags.contains(&format!("fp-{fingerprint}")));
    assert!(task.description.contains("boom"), "output excerpt attached");
    assert!(
        task.description
            .contains(&report.run_id.clone().expect("run id")),
        "ledger run referenced"
    );

    // The ledger run is failed, with the failing step carrying the evidence.
    let runs = runtime.job_history(QA_SWEEP_JOB).expect("history");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].state, JobRunState::Failed);
    assert_eq!(runs[0].steps.len(), 1);
    assert_eq!(runs[0].steps[0].state, JobRunState::Failed);
    assert!(
        runs[0].steps[0]
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("boom"))
    );
}

#[test]
fn repeated_failure_dedupes_against_the_open_task() {
    let fixture = fixture(FAILING_CHECK);
    let first = fixture.sweep();
    let filed = first.checks[0].filed_task.clone().expect("task filed");

    // New commit, same breakage: no second task.
    fixture.commit("more.txt");
    let second = fixture.sweep();
    assert_eq!(second.action, "failed");
    assert_eq!(second.checks[0].filed_task, None);
    assert_eq!(
        second.checks[0].deduped_task.as_deref(),
        Some(filed.as_str())
    );
    assert_eq!(second.checks[0].fingerprint, first.checks[0].fingerprint);

    let runtime = fixture.runtime();
    let qa_tasks = runtime
        .list_tasks_by_tags(&[QA_SWEEP_TAG.to_string()])
        .expect("tag query");
    assert_eq!(qa_tasks.len(), 1, "one open task per distinct failure");
}

#[test]
fn closed_task_with_same_fingerprint_is_refiled() {
    let fixture = fixture(FAILING_CHECK);
    let first = fixture.sweep();
    let filed = first.checks[0].filed_task.clone().expect("task filed");

    // Close the finding without fixing it; a recurrence must file anew.
    let runtime = fixture.runtime();
    runtime
        .stores()
        .tasks()
        .update(
            &filed,
            crate::runtime::TaskRecordUpdateParams {
                actor: "test".to_string(),
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        )
        .expect("close task");
    drop(runtime);

    fixture.commit("again.txt");
    let second = fixture.sweep();
    let refiled = second.checks[0].filed_task.clone().expect("refiled task");
    assert_ne!(refiled, filed);
    assert_eq!(second.checks[0].deduped_task, None);
}

#[test]
fn per_check_priority_override_applies_to_the_filed_task() {
    let fixture = fixture(
        "[[qa.workspace.check]]\nname = \"broken\"\ncommand = \"exit 1\"\npriority = \"critical\"\n",
    );
    let report = fixture.sweep();
    let task_id = report.checks[0].filed_task.clone().expect("task filed");

    let runtime = fixture.runtime();
    let tasks = runtime.list_tasks().expect("list tasks");
    let task = tasks.iter().find(|task| task.id == task_id).expect("task");
    assert_eq!(task.priority, TaskPriority::Critical);
}

// ---- mutes, branch guard, dry-run, misconfig -------------------------------

#[test]
fn muted_failing_check_does_not_block_a_green_pass() {
    let fixture = fixture(&format!(
        "{PASSING_CHECK}\
         [[qa.workspace.check]]\nname = \"flaky\"\ncommand = \"exit 1\"\nmute = true\n"
    ));
    let head = fixture.head();

    let report = fixture.sweep();
    assert_eq!(report.action, "validated");
    let flaky = report
        .checks
        .iter()
        .find(|check| check.name == "flaky")
        .expect("flaky check reported");
    assert_eq!(flaky.outcome, "muted");
    assert!(flaky.filed_task.is_none(), "muted checks never file tasks");
    assert_eq!(fixture.watermark().as_deref(), Some(head.as_str()));

    // Muted checks are not executed, so no ledger step exists for them.
    let runtime = fixture.runtime();
    let runs = runtime.job_history(QA_SWEEP_JOB).expect("history");
    assert_eq!(runs[0].steps.len(), 1);
}

#[test]
fn checkout_on_another_branch_is_skipped() {
    let fixture = fixture(PASSING_CHECK);
    git(&fixture.repo, &["checkout", "-q", "-b", "task/side-branch"]);

    let report = fixture.sweep();
    assert_eq!(report.action, "skipped");
    assert!(
        report
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("not_on_branch")),
        "reason: {:?}",
        report.reason
    );
    assert_eq!(fixture.watermark(), None);
}

#[test]
fn dry_run_reports_but_records_nothing() {
    let fixture = fixture(FAILING_CHECK);
    let report = fixture.sweep_with(QaSweepOptions { dry_run: true });

    assert_eq!(report.action, "would_validate");
    assert_eq!(report.checks[0].outcome, "would_run");
    assert!(report.run_id.is_none());
    assert_eq!(fixture.watermark(), None);

    let runtime = fixture.runtime();
    assert!(runtime.list_tasks().expect("tasks").is_empty());
    assert!(
        runtime.job_history(QA_SWEEP_JOB).is_err()
            || runtime
                .job_history(QA_SWEEP_JOB)
                .expect("history")
                .is_empty(),
        "dry run must not create ledger runs"
    );
}

#[test]
fn unregistered_configured_workspace_is_an_error_row() {
    let fixture = fixture_with_qa(
        "[qa]\n\n[[qa.workspace]]\nname = \"ghost\"\n\
         [[qa.workspace.check]]\nname = \"ok\"\ncommand = \"true\"\n",
    );
    let outcome = run_qa_sweep_at(&fixture.global, QaSweepOptions::default()).expect("sweep");
    assert_eq!(outcome.reports.len(), 1);
    assert_eq!(outcome.reports[0].action, "error");
    assert!(
        outcome.reports[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("not found in the global registry"))
    );
}

#[test]
fn no_configured_workspaces_is_a_clean_noop() {
    let fixture = fixture_with_qa("[workflow]\nbase_branch = \"agent-main\"\n");
    let outcome = run_qa_sweep_at(&fixture.global, QaSweepOptions::default()).expect("sweep");
    assert!(outcome.reports.is_empty());
}

// ---- check execution --------------------------------------------------------

#[test]
fn execute_check_captures_both_streams_and_exit_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let execution = execute_check(
        dir.path(),
        "echo out; echo err >&2; exit 3",
        Duration::from_secs(10),
    )
    .expect("run check");
    assert_eq!(execution.exit_code, Some(3));
    assert!(!execution.timed_out);
    assert!(execution.output.contains("out"));
    assert!(execution.output.contains("err"));
}

#[test]
fn execute_check_kills_and_flags_a_hung_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let execution =
        execute_check(dir.path(), "sleep 30", Duration::from_millis(200)).expect("run check");
    assert!(execution.timed_out);
    assert_eq!(execution.exit_code, None);
    assert!(execution.duration_ms < 10_000, "killed promptly");
}
