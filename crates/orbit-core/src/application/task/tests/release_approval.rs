//! ORB-11081: report-only release tasks stay out of commit-required delivery.

use orbit_types::task::{
    AWAITING_RELEASE_APPROVAL_TAG, RELEASE_TASK_TAG, Task, TaskStatus, TaskType,
};

use super::test_runtime;
use crate::application::task::{TaskAddParams, TaskUpdateParams};

const REPORT_ONLY: &str = "\
This handoff is intentionally bounded: do not commit, tag, push, publish, \
promote, merge, bump versions, edit CHANGELOG.md, or record human confirmation \
that a breaking-change candidate is accepted. Stop at the human approval \
boundary.
";

const AUTHORIZED: &str = "\
Implement the approved release-preparation diff according to RELEASING.md: \
add the CHANGELOG section, bump versions, commit, push, and open the task PR. \
Do not tag, publish, promote, or merge without a separate explicit human approval.
";

fn add_release_task(
    runtime: &crate::OrbitRuntime,
    title: &str,
    description: &str,
    tags: Vec<String>,
    status: Option<TaskStatus>,
) -> Task {
    runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            description: description.to_string(),
            acceptance_criteria: vec![
                "Preserve the human approval boundary for tag, publish, promotion, and merge."
                    .to_string(),
            ],
            tags,
            plan: "Follow RELEASING.md.".to_string(),
            workspace_path: Some(".".to_string()),
            task_type: Some(TaskType::Chore),
            status,
            ..Default::default()
        })
        .expect("add release task")
}

#[test]
fn report_only_release_task_cannot_enter_backlog_or_workflow() {
    let (_root, runtime) = test_runtime();
    let task = add_release_task(
        &runtime,
        "Prepare v0.14.0 release",
        REPORT_ONLY,
        vec![
            RELEASE_TASK_TAG.to_string(),
            AWAITING_RELEASE_APPROVAL_TAG.to_string(),
        ],
        Some(TaskStatus::Proposed),
    );

    let approve_err = runtime
        .approve_task(&task.id, Some("approve classification".to_string()), None)
        .expect_err("report-only release task must not approve into backlog");
    assert!(
        approve_err
            .to_string()
            .contains(AWAITING_RELEASE_APPROVAL_TAG),
        "{approve_err}"
    );

    let start_err = runtime
        .start_task(&task.id, Some("start report-only".to_string()), None)
        .expect_err("report-only release task must not start");
    assert!(start_err.to_string().contains("report-only"), "{start_err}");

    let update_err = runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                status: Some(TaskStatus::Backlog),
                ..Default::default()
            },
        )
        .expect_err("status-only update must not skip the mandate rewrite");
    assert!(
        update_err.to_string().contains("bounded diff"),
        "{update_err}"
    );

    let admit_err = runtime
        .ensure_task_can_enter_workflow_as_system(&task.id, "task_pr_pipeline")
        .expect_err("report-only phase cannot reach git_commit through workflow admission");
    assert!(
        admit_err
            .to_string()
            .contains("awaiting human classification"),
        "{admit_err}"
    );
    assert_eq!(
        runtime.get_task(&task.id).expect("reload").status,
        TaskStatus::Proposed
    );
}

#[test]
fn approval_handoff_rewrites_mandate_before_admission() {
    let (_root, runtime) = test_runtime();
    let task = add_release_task(
        &runtime,
        "Prepare v0.15.0 release",
        REPORT_ONLY,
        vec![
            RELEASE_TASK_TAG.to_string(),
            AWAITING_RELEASE_APPROVAL_TAG.to_string(),
        ],
        Some(TaskStatus::Proposed),
    );

    let updated = runtime
        .update_task(
            &task.id,
            TaskUpdateParams {
                description: Some(AUTHORIZED.to_string()),
                acceptance_criteria: Some(vec![
                    "Open a task-scoped PR. Stop before tag, publish, promotion, or merge."
                        .to_string(),
                ]),
                tags: Some(vec![RELEASE_TASK_TAG.to_string()]),
                status: Some(TaskStatus::Backlog),
                ..Default::default()
            },
        )
        .expect("approval handoff updates mandate and admits to backlog in one write");
    assert_eq!(updated.status, TaskStatus::Backlog);
    assert!(!updated.awaits_release_approval());

    let admitted = runtime
        .ensure_task_can_enter_workflow_as_system(&updated.id, "task_pr_pipeline")
        .expect("authorized implementation phase can enter the workflow that reaches git_commit");
    assert_eq!(admitted.status, TaskStatus::Backlog);
}
