//! Tests for the age-based terminal-task archival collector ([`TaskGcCollector`]).

#![allow(missing_docs)]

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use orbit_common::types::{ReviewThreadStatus, Task, TaskStatus};
use tempfile::TempDir;

use crate::OrbitRuntime;
use crate::command::gc::{GcClock, GcOutcome, GcReport, GcRequest, GcScope, GcTarget, execute_gc};
use crate::command::task::{TaskAddParams, TaskUpdateParams};
use crate::command::task_gc::{GC_KEEP_TAG, TaskGcCollector};

struct FakeClock(DateTime<Utc>);

impl GcClock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

struct Fixture {
    _temp: TempDir,
    runtime: OrbitRuntime,
    state_dir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().expect("tempdir");
        let global_root = temp.path().join("global");
        let workspace_root = temp.path().join("repo").join(".orbit");
        std::fs::create_dir_all(&global_root).expect("global root");
        std::fs::create_dir_all(&workspace_root).expect("workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build runtime");
        let state_dir = temp.path().join("gc-state");
        Self {
            _temp: temp,
            runtime,
            state_dir,
        }
    }

    fn add_proposed(&self, title: &str) -> Task {
        self.runtime
            .add_task(TaskAddParams {
                title: title.to_string(),
                description: "Exercise task GC.".to_string(),
                acceptance_criteria: vec!["archives when eligible".to_string()],
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task")
    }

    fn update_status(&self, id: &str, status: TaskStatus) {
        self.runtime
            .update_task(
                id,
                TaskUpdateParams {
                    status: Some(status),
                    ..Default::default()
                },
            )
            .expect("update status");
    }

    /// Walks a proposed task to `done`, satisfying the plan and
    /// execution-summary guards along the way.
    fn drive_to_done(&self, id: &str) {
        self.update_status(id, TaskStatus::Backlog);
        self.runtime
            .update_task(
                id,
                TaskUpdateParams {
                    plan: Some("1) do 2) verify".to_string()),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .expect("in-progress with plan");
        self.runtime
            .update_task(
                id,
                TaskUpdateParams {
                    execution_summary: Some("did it; verified".to_string()),
                    status: Some(TaskStatus::Review),
                    ..Default::default()
                },
            )
            .expect("review with summary");
        self.update_status(id, TaskStatus::Done);
    }

    fn terminal_at(&self, id: &str, status: TaskStatus) -> DateTime<Utc> {
        self.runtime
            .get_task_history(id)
            .expect("history")
            .into_iter()
            .filter(|entry| entry.to_status == Some(status))
            .map(|entry| entry.at)
            .max()
            .expect("terminal transition recorded")
    }

    fn collect(
        &self,
        collector: &TaskGcCollector<'_>,
        apply: bool,
        now: DateTime<Utc>,
        retention: Option<&str>,
    ) -> GcReport {
        let clock = FakeClock(now);
        execute_gc(
            collector,
            GcRequest {
                apply,
                scope: GcScope::Workspace {
                    workspace_id: Some("test".to_string()),
                    root: self.runtime.paths().orbit_dir.clone(),
                },
                retention_override: retention,
                global_state_dir: &self.state_dir,
                clock: &clock,
            },
        )
        .expect("gc report")
    }

    fn collector(&self) -> TaskGcCollector<'_> {
        TaskGcCollector::new(&self.runtime)
    }
}

fn eligible_ids(report: &GcReport) -> Vec<String> {
    report.targets[0]
        .items
        .iter()
        .map(|item| item.id.clone())
        .collect()
}

fn skip_code(report: &GcReport, id: &str) -> Option<String> {
    report.targets[0]
        .skipped
        .iter()
        .find(|skip| skip.id == id)
        .map(|skip| skip.code.clone())
}

#[test]
fn done_task_older_than_retention_is_archived() {
    let fixture = Fixture::new();
    let task = fixture.add_proposed("Old done task");
    fixture.drive_to_done(&task.id);
    let done_at = fixture.terminal_at(&task.id, TaskStatus::Done);
    // A live backlog task must never be pulled in solely by the clock.
    let backlog = fixture.add_proposed("Live backlog task");
    fixture.update_status(&backlog.id, TaskStatus::Backlog);

    let now = done_at + Duration::days(91);
    let plan = fixture.collect(&fixture.collector(), false, now, None);
    assert_eq!(eligible_ids(&plan), vec![task.id.clone()]);
    assert_eq!(plan.targets[0].counts.reclaimed, 0);
    assert_eq!(
        fixture.runtime.get_task(&task.id).expect("task").status,
        TaskStatus::Done,
        "plan mode must not mutate"
    );

    let apply = fixture.collect(&fixture.collector(), true, now, None);
    assert_eq!(apply.targets[0].counts.eligible, 1);
    assert_eq!(apply.targets[0].counts.reclaimed, 1);
    assert_eq!(apply.outcome, GcOutcome::Clean);
    assert_eq!(
        fixture.runtime.get_task(&task.id).expect("task").status,
        TaskStatus::Archived
    );
    assert_eq!(
        fixture.runtime.get_task(&backlog.id).expect("task").status,
        TaskStatus::Backlog
    );
}

#[test]
fn retention_boundary_is_strict() {
    let fixture = Fixture::new();
    let task = fixture.add_proposed("Boundary task");
    fixture.drive_to_done(&task.id);
    let done_at = fixture.terminal_at(&task.id, TaskStatus::Done);

    // Exactly at the retention age: age == retention, not strictly older.
    let at_boundary = fixture.collect(
        &fixture.collector(),
        false,
        done_at + Duration::days(90),
        None,
    );
    assert_eq!(at_boundary.targets[0].counts.eligible, 0);

    // One second past the boundary: eligible.
    let past_boundary = fixture.collect(
        &fixture.collector(),
        false,
        done_at + Duration::days(90) + Duration::seconds(1),
        None,
    );
    assert_eq!(eligible_ids(&past_boundary), vec![task.id]);
}

#[test]
fn nonterminal_statuses_are_never_selected_by_age() {
    let fixture = Fixture::new();

    let proposed = fixture.add_proposed("proposed");

    let backlog = fixture.add_proposed("backlog");
    fixture.update_status(&backlog.id, TaskStatus::Backlog);

    let someday = fixture.add_proposed("someday");
    fixture.update_status(&someday.id, TaskStatus::Someday);

    let blocked = fixture.add_proposed("blocked");
    fixture.update_status(&blocked.id, TaskStatus::Blocked);

    let in_progress = fixture.add_proposed("in-progress");
    fixture.update_status(&in_progress.id, TaskStatus::Backlog);
    fixture
        .runtime
        .update_task(
            &in_progress.id,
            TaskUpdateParams {
                plan: Some("1) do 2) verify".to_string()),
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .expect("to in-progress");

    let review = fixture.add_proposed("review");
    fixture.update_status(&review.id, TaskStatus::Backlog);
    fixture
        .runtime
        .update_task(
            &review.id,
            TaskUpdateParams {
                plan: Some("1) do 2) verify".to_string()),
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .expect("to in-progress");
    fixture
        .runtime
        .update_task(
            &review.id,
            TaskUpdateParams {
                execution_summary: Some("done; verified".to_string()),
                status: Some(TaskStatus::Review),
                ..Default::default()
            },
        )
        .expect("to review");

    // A single old done task confirms the clock is genuinely far in the future.
    let done = fixture.add_proposed("done");
    fixture.drive_to_done(&done.id);
    let done_at = fixture.terminal_at(&done.id, TaskStatus::Done);

    let report = fixture.collect(
        &fixture.collector(),
        false,
        done_at + Duration::days(365),
        None,
    );
    assert_eq!(
        eligible_ids(&report),
        vec![done.id],
        "only the terminal task is eligible"
    );
    assert_eq!(report.targets[0].counts.scanned, 7);
    // Non-terminal statuses are silently excluded, not reported as skips.
    for id in [
        &proposed.id,
        &backlog.id,
        &someday.id,
        &blocked.id,
        &in_progress.id,
        &review.id,
    ] {
        assert!(
            skip_code(&report, id).is_none(),
            "{id} should not be a skip"
        );
    }
}

#[test]
fn keep_tagged_terminal_task_is_retained_with_reason() {
    let fixture = Fixture::new();
    let task = fixture
        .runtime
        .add_task(TaskAddParams {
            title: "Keep me".to_string(),
            description: "Exempt from GC.".to_string(),
            acceptance_criteria: vec!["retained".to_string()],
            tags: vec![GC_KEEP_TAG.to_string()],
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("add tagged task");
    fixture.drive_to_done(&task.id);
    let done_at = fixture.terminal_at(&task.id, TaskStatus::Done);

    let report = fixture.collect(
        &fixture.collector(),
        false,
        done_at + Duration::days(120),
        None,
    );
    assert_eq!(report.targets[0].counts.eligible, 0);
    assert_eq!(skip_code(&report, &task.id).as_deref(), Some("keep_tag"));
}

#[test]
fn open_review_thread_retains_task() {
    let fixture = Fixture::new();
    let task = fixture.add_proposed("Task under review");
    fixture.update_status(&task.id, TaskStatus::Backlog);
    fixture
        .runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                plan: Some("1) do 2) verify".to_string()),
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .expect("to in-progress");
    fixture
        .runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                execution_summary: Some("done; verified".to_string()),
                status: Some(TaskStatus::Review),
                ..Default::default()
            },
        )
        .expect("to review");
    fixture
        .runtime
        .add_review_thread(&task.id, "needs a look".to_string(), None, None, None, None)
        .expect("open review thread");
    fixture.update_status(&task.id, TaskStatus::Done);
    let done_at = fixture.terminal_at(&task.id, TaskStatus::Done);

    // Sanity: the thread stayed open across the transition to done.
    let open = fixture
        .runtime
        .list_review_threads(&task.id, Some(ReviewThreadStatus::Open))
        .expect("threads");
    assert_eq!(open.len(), 1);

    let report = fixture.collect(
        &fixture.collector(),
        false,
        done_at + Duration::days(120),
        None,
    );
    assert_eq!(report.targets[0].counts.eligible, 0);
    assert_eq!(
        skip_code(&report, &task.id).as_deref(),
        Some("open_review_threads")
    );
}

#[test]
fn active_dependent_retains_task_until_dependent_closes() {
    let fixture = Fixture::new();
    let dependency = fixture.add_proposed("Shared dependency");
    fixture.drive_to_done(&dependency.id);
    let done_at = fixture.terminal_at(&dependency.id, TaskStatus::Done);

    // A live task still declares a blocked-by edge onto the done dependency.
    let dependent = fixture
        .runtime
        .add_task(TaskAddParams {
            title: "Depends on the shared task".to_string(),
            description: "Keeps the dependency coupled.".to_string(),
            acceptance_criteria: vec!["blocked".to_string()],
            dependencies: vec![dependency.id.clone()],
            workspace_path: Some(".".to_string()),
            ..Default::default()
        })
        .expect("add dependent");
    fixture.update_status(&dependent.id, TaskStatus::Backlog);

    let now = done_at + Duration::days(120);
    let retained = fixture.collect(&fixture.collector(), false, now, None);
    assert_eq!(retained.targets[0].counts.eligible, 0);
    assert_eq!(
        skip_code(&retained, &dependency.id).as_deref(),
        Some("active_dependency")
    );

    // Once the dependent is rejected (closed), the coupling is resolved.
    fixture.update_status(&dependent.id, TaskStatus::Rejected);
    let released = fixture.collect(&fixture.collector(), false, now, None);
    assert_eq!(eligible_ids(&released), vec![dependency.id]);
}

#[test]
fn rejected_tasks_only_selected_when_configured() {
    let fixture = Fixture::new();
    let task = fixture.add_proposed("Rejected task");
    fixture.update_status(&task.id, TaskStatus::Rejected);
    let rejected_at = fixture.terminal_at(&task.id, TaskStatus::Rejected);
    let now = rejected_at + Duration::days(120);

    let default_report = fixture.collect(&fixture.collector(), false, now, None);
    assert_eq!(
        default_report.targets[0].counts.eligible, 0,
        "done-only default must not select rejected tasks"
    );

    let collector = fixture.collector().include_rejected(true);
    let opt_in = fixture.collect(&collector, false, now, None);
    assert_eq!(eligible_ids(&opt_in), vec![task.id]);
}

#[test]
fn retention_override_changes_eligibility() {
    let fixture = Fixture::new();
    let task = fixture.add_proposed("Recently done");
    fixture.drive_to_done(&task.id);
    let done_at = fixture.terminal_at(&task.id, TaskStatus::Done);
    // Two days after completion: within the 90d default, but past a 1d override.
    let now = done_at + Duration::days(2);

    let default_report = fixture.collect(&fixture.collector(), false, now, None);
    assert_eq!(default_report.targets[0].counts.eligible, 0);

    let overridden = fixture.collect(&fixture.collector(), false, now, Some("1d"));
    assert_eq!(eligible_ids(&overridden), vec![task.id]);
}

#[test]
fn dry_run_selection_matches_apply() {
    let fixture = Fixture::new();
    let task = fixture.add_proposed("Parity task");
    fixture.drive_to_done(&task.id);
    let done_at = fixture.terminal_at(&task.id, TaskStatus::Done);
    let now = done_at + Duration::days(200);

    let plan = fixture.collect(&fixture.collector(), false, now, None);
    let apply = fixture.collect(&fixture.collector(), true, now, None);
    assert_eq!(eligible_ids(&plan), eligible_ids(&apply));
    assert_eq!(plan.targets[0].counts.reclaimed, 0);
    assert_eq!(apply.targets[0].counts.reclaimed, 1);
}

#[test]
fn second_apply_is_idempotent() {
    let fixture = Fixture::new();
    let task = fixture.add_proposed("Idempotent task");
    fixture.drive_to_done(&task.id);
    let done_at = fixture.terminal_at(&task.id, TaskStatus::Done);
    let now = done_at + Duration::days(200);

    let first = fixture.collect(&fixture.collector(), true, now, None);
    assert_eq!(first.targets[0].counts.reclaimed, 1);

    let second = fixture.collect(&fixture.collector(), true, now, None);
    assert_eq!(second.targets[0].counts.eligible, 0);
    assert_eq!(second.targets[0].counts.reclaimed, 0);
    assert_eq!(second.outcome, GcOutcome::Clean);
}

#[test]
fn archived_task_restores_to_backlog() {
    let fixture = Fixture::new();
    let task = fixture.add_proposed("Restore me");
    fixture.drive_to_done(&task.id);
    let done_at = fixture.terminal_at(&task.id, TaskStatus::Done);

    fixture.collect(
        &fixture.collector(),
        true,
        done_at + Duration::days(200),
        None,
    );
    assert_eq!(
        fixture.runtime.get_task(&task.id).expect("task").status,
        TaskStatus::Archived
    );

    fixture.update_status(&task.id, TaskStatus::Backlog);
    assert_eq!(
        fixture.runtime.get_task(&task.id).expect("task").status,
        TaskStatus::Backlog,
        "archival is reversible through the normal lifecycle"
    );
}

#[test]
fn empty_workspace_reports_tasks_target_with_no_candidates() {
    let fixture = Fixture::new();
    let collector = fixture.collector();
    let report = fixture.collect(&collector, false, Utc::now(), None);
    assert_eq!(report.targets[0].target, GcTarget::Tasks);
    assert_eq!(report.targets[0].counts.scanned, 0);
    assert_eq!(report.targets[0].counts.eligible, 0);
    assert_eq!(report.outcome, GcOutcome::Clean);
}
