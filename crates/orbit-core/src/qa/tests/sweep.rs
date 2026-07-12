//! qa-sweep pass tests [ORB-10039, reworked ORB-10146]: end-to-end over a
//! seeded global root and a real temp git repo, with an injected fake QA agent
//! standing in for the worker daemon. Covers watermark advance/hold rules,
//! ledger run recording, finding-task filing + fingerprint dedupe, severity
//! clamping, the prompt contract, dry-run inertness, the branch guard, and the
//! worker-client failure paths (daemon down, timeout, bad JSON, non-success).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use chrono::Utc;
use orbit_common::types::{
    JobRunState, TaskPriority, TaskStatus, Workspace, WorkspaceRegistry, WorkspaceStatus,
};
use tempfile::TempDir;

use crate::OrbitRuntime;
use crate::qa::state::{load_state, state_path};
use crate::qa::sweep::{
    QaAgent, QaAgentError, QaAgentRequest, QaAgentRun, QaWorkspaceReport, run_qa_sweep_with,
};
use crate::qa::{QA_SWEEP_JOB, QA_SWEEP_TAG, QaSweepOptions};
use crate::workspace_registry;

const WS_NAME: &str = "polaris";

// ---- fake QA agent ---------------------------------------------------------

/// A [`QaAgent`] that returns a canned outcome and records the last prompt it
/// was handed, so tests can exercise every pass branch without a live daemon.
struct FakeAgent {
    result: Result<QaAgentRun, QaAgentError>,
    last_prompt: Mutex<Option<String>>,
    calls: Mutex<usize>,
}

impl FakeAgent {
    fn new(result: Result<QaAgentRun, QaAgentError>) -> Self {
        Self {
            result,
            last_prompt: Mutex::new(None),
            calls: Mutex::new(0),
        }
    }

    /// Agent that completes `ok` with the given findings-report text.
    fn reporting(report: &str) -> Self {
        Self::new(Ok(QaAgentRun {
            agent_run_id: "wrk-1".to_string(),
            status: "ok".to_string(),
            report_text: Some(report.to_string()),
        }))
    }

    fn called(&self) -> usize {
        *self.calls.lock().unwrap()
    }

    fn last_prompt(&self) -> Option<String> {
        self.last_prompt.lock().unwrap().clone()
    }
}

impl QaAgent for FakeAgent {
    fn run(&self, request: QaAgentRequest) -> Result<QaAgentRun, QaAgentError> {
        *self.calls.lock().unwrap() += 1;
        *self.last_prompt.lock().unwrap() = Some(request.prompt);
        self.result.clone()
    }
}

const CLEAN: &str = r#"{"findings": []}"#;

fn one_finding(name: &str, severity: &str) -> String {
    format!(
        r#"{{"findings":[{{"name":"{name}","severity":"{severity}","summary":"broken","evidence":"repro steps","commits":["abc feat"]}}]}}"#
    )
}

// ---- fixture ---------------------------------------------------------------

struct Fixture {
    _tmp: TempDir,
    global: PathBuf,
    repo: PathBuf,
    orbit_dir: PathBuf,
}

impl Fixture {
    /// Sweep once (non-dry) with `agent` and return the single workspace report.
    fn sweep(&self, agent: &dyn QaAgent) -> QaWorkspaceReport {
        self.sweep_with(QaSweepOptions::default(), agent)
    }

    fn sweep_with(&self, options: QaSweepOptions, agent: &dyn QaAgent) -> QaWorkspaceReport {
        let mut outcome = run_qa_sweep_with(&self.global, options, agent).expect("sweep pass");
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
        // Stage only the named file: `git add .` races with the live SQLite WAL
        // files the sweep's runtime writes under `.orbit/`.
        git(&self.repo, &["add", name]);
        git(&self.repo, &["commit", "-m", &format!("add {name}")]);
    }
}

/// Seed a global root + registered git workspace with the default `[qa]`
/// section (one workspace, no crew override → default crew resolution).
fn fixture() -> Fixture {
    fixture_with_qa(&format!("[qa]\n\n[[qa.workspace]]\nname = \"{WS_NAME}\"\n"))
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
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);

    let mut registry = WorkspaceRegistry::default();
    registry.workspaces.push(Workspace {
        id: "ws-qa-test".to_string(),
        name: WS_NAME.to_string(),
        root: repo.clone(),
        orbit_dir: orbit_dir.clone(),
        git_remote: None,
        ship_mode: None,
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

// ---- clean pass / watermark advance ----------------------------------------

#[test]
fn clean_pass_records_ledger_run_and_advances_watermark() {
    let fixture = fixture();
    let head = fixture.head();
    let agent = FakeAgent::reporting(CLEAN);

    let report = fixture.sweep(&agent);
    assert_eq!(report.action, "validated", "reason: {:?}", report.reason);
    assert_eq!(report.baseline, None, "first validation has no baseline");
    assert_eq!(report.head.as_deref(), Some(head.as_str()));
    assert!(report.findings.is_empty());
    assert_eq!(
        report.crew.as_deref(),
        Some("claude"),
        "default crew resolved"
    );
    assert_eq!(report.agent_run_id.as_deref(), Some("wrk-1"));

    // Watermark advanced to the validated HEAD.
    assert_eq!(fixture.watermark().as_deref(), Some(head.as_str()));

    // The pass is a first-class ledger run with one agent step linking the run.
    let runtime = fixture.runtime();
    let runs = runtime.job_history(QA_SWEEP_JOB).expect("qa_sweep history");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].state, JobRunState::Success);
    assert_eq!(runs[0].run_id, report.run_id.expect("run id reported"));
    assert_eq!(runs[0].steps.len(), 1);
    assert_eq!(runs[0].steps[0].state, JobRunState::Success);
}

#[test]
fn findings_are_filed_as_deduped_tasks_and_watermark_advances() {
    // default_priority critical so a high-severity finding maps straight to High.
    let fixture = fixture_with_qa(&format!(
        "[qa]\ndefault_priority = \"critical\"\n\n[[qa.workspace]]\nname = \"{WS_NAME}\"\n"
    ));
    let head = fixture.head();
    let agent = FakeAgent::reporting(&one_finding("login-loops", "high"));

    let report = fixture.sweep(&agent);
    assert_eq!(report.action, "validated");
    assert_eq!(report.findings.len(), 1);
    let finding = &report.findings[0];
    assert_eq!(finding.severity, "high");
    let task_id = finding.filed_task.clone().expect("task filed");
    assert!(finding.deduped_task.is_none());

    // Findings captured as tasks → watermark advances (re-validating adds nothing).
    assert_eq!(fixture.watermark().as_deref(), Some(head.as_str()));

    let runtime = fixture.runtime();
    let tasks = runtime.list_tasks().expect("list tasks");
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.id, task_id);
    assert_eq!(task.status, TaskStatus::Backlog, "default files as backlog");
    assert_eq!(
        task.priority,
        TaskPriority::High,
        "severity high under critical ceiling"
    );
    assert!(task.tags.contains(&QA_SWEEP_TAG.to_string()));
    assert!(task.tags.contains(&format!("fp-{}", finding.fingerprint)));
    assert!(
        task.description.contains("repro steps"),
        "evidence attached"
    );
    assert!(task.description.contains("login-loops"));

    let runs = runtime.job_history(QA_SWEEP_JOB).expect("history");
    assert_eq!(
        runs[0].state,
        JobRunState::Success,
        "findings still validate the pass"
    );
}

#[test]
fn severity_is_clamped_to_the_default_priority_ceiling() {
    // default ceiling medium: a critical finding is clamped down.
    let fixture = fixture();
    let agent = FakeAgent::reporting(&one_finding("data-loss", "critical"));
    let report = fixture.sweep(&agent);
    let task_id = report.findings[0].filed_task.clone().expect("filed");

    let runtime = fixture.runtime();
    let task = runtime.get_task(&task_id).expect("task");
    assert_eq!(task.priority, TaskPriority::Medium, "clamped to ceiling");
}

#[test]
fn unchanged_head_is_skipped_without_a_run() {
    let fixture = fixture();
    assert_eq!(
        fixture.sweep(&FakeAgent::reporting(CLEAN)).action,
        "validated"
    );

    let agent = FakeAgent::reporting(CLEAN);
    let report = fixture.sweep(&agent);
    assert_eq!(report.action, "skipped");
    assert_eq!(report.reason.as_deref(), Some("no_new_commits"));
    assert!(report.run_id.is_none());
    assert_eq!(agent.called(), 0, "no agent invoked on a skip");

    let runtime = fixture.runtime();
    assert_eq!(
        runtime.job_history(QA_SWEEP_JOB).expect("history").len(),
        1,
        "skip records no second run"
    );
}

#[test]
fn new_commits_are_revalidated_with_the_range_reported() {
    let fixture = fixture();
    assert_eq!(
        fixture.sweep(&FakeAgent::reporting(CLEAN)).action,
        "validated"
    );
    let baseline = fixture.watermark().expect("baseline");

    fixture.commit("one.txt");
    fixture.commit("two.txt");
    let head = fixture.head();

    let report = fixture.sweep(&FakeAgent::reporting(CLEAN));
    assert_eq!(report.action, "validated");
    assert_eq!(report.baseline.as_deref(), Some(baseline.as_str()));
    assert_eq!(report.new_commits.as_ref().map(Vec::len), Some(2));
    assert_eq!(fixture.watermark().as_deref(), Some(head.as_str()));
}

// ---- worker-client failure paths (watermark held, error row) ---------------

#[test]
fn non_success_run_holds_watermark_and_files_no_task() {
    let fixture = fixture();
    let agent = FakeAgent::new(Ok(QaAgentRun {
        agent_run_id: "wrk-9".to_string(),
        status: "error".to_string(),
        report_text: Some("crashed".to_string()),
    }));

    let report = fixture.sweep(&agent);
    assert_eq!(report.action, "error");
    assert!(
        report
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("non-success")),
        "reason: {:?}",
        report.reason
    );
    assert_eq!(report.agent_run_id.as_deref(), Some("wrk-9"));
    assert_eq!(
        fixture.watermark(),
        None,
        "failure never advances the watermark"
    );

    let runtime = fixture.runtime();
    assert!(runtime.list_tasks().expect("tasks").is_empty());
    let runs = runtime.job_history(QA_SWEEP_JOB).expect("history");
    assert_eq!(runs[0].state, JobRunState::Failed);
    assert_eq!(runs[0].steps.len(), 1);
    assert_eq!(runs[0].steps[0].state, JobRunState::Failed);
}

#[test]
fn unparseable_report_holds_watermark() {
    let fixture = fixture();
    let agent = FakeAgent::reporting("the build looked fine to me, no JSON here");

    let report = fixture.sweep(&agent);
    assert_eq!(report.action, "error");
    assert!(
        report
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("unparseable")),
        "reason: {:?}",
        report.reason
    );
    assert_eq!(fixture.watermark(), None);

    let runtime = fixture.runtime();
    assert_eq!(
        runtime.job_history(QA_SWEEP_JOB).expect("history")[0].state,
        JobRunState::Failed
    );
}

#[test]
fn daemon_down_holds_watermark_but_records_a_failed_ledger_run() {
    let fixture = fixture();
    let agent = FakeAgent::new(Err(QaAgentError {
        agent_run_id: None,
        message: "worker daemon unreachable: connection refused".to_string(),
    }));

    let report = fixture.sweep(&agent);
    assert_eq!(report.action, "error");
    assert!(
        report
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("unreachable"))
    );
    assert!(report.agent_run_id.is_none());
    assert_eq!(fixture.watermark(), None);

    // A ledger run is still recorded for the swept workspace (finalized failed).
    let runtime = fixture.runtime();
    let runs = runtime.job_history(QA_SWEEP_JOB).expect("history");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].state, JobRunState::Failed);
    assert_eq!(runs[0].steps.len(), 1);
}

#[test]
fn timeout_preserves_the_worker_run_id_on_the_report() {
    let fixture = fixture();
    let agent = FakeAgent::new(Err(QaAgentError {
        agent_run_id: Some("wrk-timeout".to_string()),
        message: "agent run timed out after 7200s".to_string(),
    }));

    let report = fixture.sweep(&agent);
    assert_eq!(report.action, "error");
    assert_eq!(report.agent_run_id.as_deref(), Some("wrk-timeout"));
    assert_eq!(fixture.watermark(), None);
}

// ---- dedupe / refile -------------------------------------------------------

#[test]
fn repeated_finding_dedupes_against_the_open_task() {
    let fixture = fixture();
    let first = fixture.sweep(&FakeAgent::reporting(&one_finding("flaky-x", "medium")));
    let filed = first.findings[0].filed_task.clone().expect("task filed");

    // New commit, same finding name: no second task.
    fixture.commit("more.txt");
    let second = fixture.sweep(&FakeAgent::reporting(&one_finding("flaky-x", "medium")));
    assert_eq!(second.action, "validated");
    assert_eq!(second.findings[0].filed_task, None);
    assert_eq!(
        second.findings[0].deduped_task.as_deref(),
        Some(filed.as_str())
    );
    assert_eq!(
        second.findings[0].fingerprint,
        first.findings[0].fingerprint
    );

    let runtime = fixture.runtime();
    let qa_tasks = runtime
        .list_tasks_by_tags(&[QA_SWEEP_TAG.to_string()])
        .expect("tag query");
    assert_eq!(qa_tasks.len(), 1, "one open task per distinct finding");
}

#[test]
fn closed_task_with_same_finding_is_refiled() {
    let fixture = fixture();
    let first = fixture.sweep(&FakeAgent::reporting(&one_finding("regression-y", "high")));
    let filed = first.findings[0].filed_task.clone().expect("task filed");

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
    let second = fixture.sweep(&FakeAgent::reporting(&one_finding("regression-y", "high")));
    let refiled = second.findings[0].filed_task.clone().expect("refiled task");
    assert_ne!(refiled, filed);
    assert_eq!(second.findings[0].deduped_task, None);
}

// ---- prompt contract -------------------------------------------------------

#[test]
fn prompt_carries_the_range_and_commit_list() {
    let fixture = fixture();
    assert_eq!(
        fixture.sweep(&FakeAgent::reporting(CLEAN)).action,
        "validated"
    );
    let baseline = fixture.watermark().expect("baseline");
    fixture.commit("feature.txt");
    let head = fixture.head();

    let agent = FakeAgent::reporting(CLEAN);
    fixture.sweep(&agent);
    let prompt = agent.last_prompt().expect("prompt captured");
    assert!(
        prompt.contains(&format!("{baseline}..{head}")),
        "range in prompt"
    );
    assert!(
        prompt.contains("add feature.txt"),
        "commit subject in prompt"
    );
    assert!(prompt.contains("findings"), "JSON contract in prompt");
    assert!(prompt.contains(WS_NAME), "workspace named");
}

// ---- dry-run, branch guard, misconfig --------------------------------------

#[test]
fn dry_run_reports_but_records_nothing_and_never_invokes_the_agent() {
    let fixture = fixture();
    let agent = FakeAgent::reporting(&one_finding("x", "high"));
    let report = fixture.sweep_with(
        QaSweepOptions {
            dry_run: true,
            ..QaSweepOptions::default()
        },
        &agent,
    );

    assert_eq!(report.action, "would_validate");
    assert_eq!(report.crew.as_deref(), Some("claude"));
    assert!(report.findings.is_empty());
    assert!(report.run_id.is_none());
    assert_eq!(agent.called(), 0, "dry run never invokes the agent");
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
fn checkout_on_another_branch_is_skipped() {
    let fixture = fixture();
    git(&fixture.repo, &["checkout", "-q", "-b", "task/side-branch"]);

    let agent = FakeAgent::reporting(CLEAN);
    let report = fixture.sweep(&agent);
    assert_eq!(report.action, "skipped");
    assert!(
        report
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("not_on_branch")),
        "reason: {:?}",
        report.reason
    );
    assert_eq!(agent.called(), 0);
    assert_eq!(fixture.watermark(), None);
}

#[test]
fn workspace_filter_excludes_other_configured_workspaces() {
    let fixture = fixture_with_qa(&format!(
        "[qa]\n\n[[qa.workspace]]\nname = \"{WS_NAME}\"\n\n[[qa.workspace]]\nname = \"ghost\"\n"
    ));
    let outcome = run_qa_sweep_with(
        &fixture.global,
        QaSweepOptions {
            dry_run: true,
            workspace: Some(WS_NAME.to_string()),
        },
        &FakeAgent::reporting(CLEAN),
    )
    .expect("filtered sweep");

    assert_eq!(outcome.reports.len(), 1);
    assert_eq!(outcome.reports[0].workspace, WS_NAME);
    assert_eq!(outcome.reports[0].action, "would_validate");
}

#[test]
fn unregistered_configured_workspace_is_an_error_row() {
    let fixture = fixture_with_qa("[qa]\n\n[[qa.workspace]]\nname = \"ghost\"\n");
    let outcome = run_qa_sweep_with(
        &fixture.global,
        QaSweepOptions::default(),
        &FakeAgent::reporting(CLEAN),
    )
    .expect("sweep");
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
    let outcome = run_qa_sweep_with(
        &fixture.global,
        QaSweepOptions::default(),
        &FakeAgent::reporting(CLEAN),
    )
    .expect("sweep");
    assert!(outcome.reports.is_empty());
}
