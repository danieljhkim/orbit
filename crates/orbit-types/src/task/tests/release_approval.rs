use chrono::Utc;

use crate::task::{
    AWAITING_RELEASE_APPROVAL_TAG, RELEASE_TASK_TAG, Task, TaskPriority, TaskStatus, TaskType,
};

fn task(description: &str, criteria: &[&str], tags: &[&str]) -> Task {
    let now = Utc::now();
    Task {
        id: "ORB-10987".to_string(),
        title: "Prepare v0.14.0 release".to_string(),
        description: description.to_string(),
        acceptance_criteria: criteria.iter().map(|value| (*value).to_string()).collect(),
        tags: tags.iter().map(|value| (*value).to_string()).collect(),
        plan: String::new(),
        execution_summary: String::new(),
        context_files: Vec::new(),
        created_by: None,
        planned_by: None,
        implemented_by: None,
        status: TaskStatus::Proposed,
        priority: TaskPriority::High,
        complexity: None,
        task_type: TaskType::Chore,
        pr_status: None,
        external_refs: Vec::new(),
        relations: Vec::new(),
        job_run_id: None,
        crew: None,
        orchestrator: None,
        created_at: now,
        updated_at: now,
    }
}

const REPORT_ONLY_MANDATE: &str = "\
Prepare the candidate v0.14.0 release after the report-first probe.

This handoff is intentionally bounded: do not commit, tag, push, publish, \
promote, merge, bump versions, edit CHANGELOG.md, or record human confirmation \
that a breaking-change candidate is accepted. The implementing release agent \
must stop at the human approval boundary and report evidence before any \
release-state action.
";

const AUTHORIZED_MANDATE: &str = "\
Prepare the candidate v0.15.0 release as a reviewable PR targeting agent-main.

The report-first survey is complete. Implement the approved release-preparation \
diff according to RELEASING.md: add the CHANGELOG section, bump versions, \
commit, push the task branch, and open the task PR.

This task stops at an open PR. Do not create or push v0.15.0, publish \
Cargo/npm/GitHub/Homebrew artifacts, promote agent-main to main, merge any PR, \
or mark the release complete.
";

#[test]
fn awaiting_tag_keeps_a_release_task_non_dispatchable() {
    let task = task(
        AUTHORIZED_MANDATE,
        &["Follow RELEASING.md."],
        &[RELEASE_TASK_TAG, AWAITING_RELEASE_APPROVAL_TAG],
    );
    assert!(task.awaits_release_approval());
    assert!(
        task.release_approval_block_reason()
            .contains(AWAITING_RELEASE_APPROVAL_TAG)
    );
}

#[test]
fn historical_report_only_mandate_matches_orb_10987() {
    let task = task(
        REPORT_ONLY_MANDATE,
        &[
            "Follow RELEASING.md and its release checklist; preserve the human approval boundary for tag, publish, promotion, and merge.",
            "The PR workflow may create its task commit, push its task branch, and open/update the PR. Do not tag, publish, promote, or merge without a separate explicit human approval.",
        ],
        &[RELEASE_TASK_TAG],
    );
    assert!(
        task.awaits_release_approval(),
        "ORB-10987/ORB-11004 report-only mandate must stay non-dispatchable even without the new tag"
    );
}

#[test]
fn authorized_implementation_mandate_is_dispatchable() {
    let task = task(
        AUTHORIZED_MANDATE,
        &[
            "Cargo.toml, Cargo.lock, and npm/package.json report version 0.15.0.",
            "A task-scoped PR is opened against agent-main. Stop before tag creation/push, package or release publication, agent-main-to-main promotion, PR merge, or final release completion.",
        ],
        &[RELEASE_TASK_TAG],
    );
    assert!(!task.awaits_release_approval());
}

#[test]
fn probe_and_unrelated_tasks_are_not_release_approval_pauses() {
    let probe = task(
        "Report-first gate. It must never edit repository files.",
        &["A blocked or no-release pass changes no repository or release state."],
        &["release-prep", "no-diff-expected"],
    );
    assert!(!probe.awaits_release_approval());

    let ordinary = task("Implement a bug fix.", &["The failing test passes."], &[]);
    assert!(!ordinary.awaits_release_approval());
}
