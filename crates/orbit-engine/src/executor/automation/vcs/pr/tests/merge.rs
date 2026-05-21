use super::super::merge::merge_batch_pr;
use super::test_support::*;

use crate::context::TaskReadHost;
use orbit_common::types::TaskStatus;
use serde_json::json;

#[test]
fn merge_batch_pr_preserves_task_attribution_per_task() {
    let workspace = pr_workspace();
    let cases = [
        ("T-CLAUDE", "claude"),
        ("T-CODEX", "codex"),
        ("T-GEMINI", "gemini"),
        ("T-GROK", "grok"),
    ];
    let tasks = cases
        .iter()
        .map(|(id, implemented_by)| review_batch_task(id, Some(implemented_by), None))
        .collect::<Vec<_>>();
    let host = PrOpenTestHost::new(tasks, workspace.repo.clone());

    let result =
        merge_batch_pr(&host, &merge_batch_pr_input(&workspace.repo)).expect("merge batch pr");

    assert_eq!(result["merged"], json!(true));
    for (task_id, expected) in cases {
        let task = host.get_task(task_id).expect("updated task");
        assert_eq!(task.status, TaskStatus::Done, "{task_id}");
        assert_eq!(task.implemented_by.as_deref(), Some(expected), "{task_id}");

        let (_, update) = host
            .automation_updates()
            .into_iter()
            .find(|(updated_task_id, _)| updated_task_id == task_id)
            .expect("task automation update");
        assert_eq!(update.model.as_deref(), Some(expected), "{task_id}");
    }
}

#[test]
fn merge_batch_pr_uses_created_by_when_implemented_by_missing() {
    let workspace = pr_workspace();
    let host = PrOpenTestHost::new(
        vec![review_batch_task("T-CREATED-BY", None, Some("codex"))],
        workspace.repo.clone(),
    );

    merge_batch_pr(&host, &merge_batch_pr_input(&workspace.repo)).expect("merge batch pr");

    let task = host.get_task("T-CREATED-BY").expect("updated task");
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.implemented_by.as_deref(), Some("codex"));
    let (_, update) = host
        .automation_updates()
        .into_iter()
        .find(|(updated_task_id, _)| updated_task_id == "T-CREATED-BY")
        .expect("task automation update");
    assert_eq!(update.model.as_deref(), Some("codex"));
}

#[test]
fn merge_batch_pr_actorless_task_falls_back_to_system() {
    let workspace = pr_workspace();
    let host = PrOpenTestHost::new(
        vec![review_batch_task("T-SYSTEM", None, None)],
        workspace.repo.clone(),
    );

    merge_batch_pr(&host, &merge_batch_pr_input(&workspace.repo)).expect("merge batch pr");

    let task = host.get_task("T-SYSTEM").expect("updated task");
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.implemented_by.as_deref(), Some("system"));
    let (_, update) = host
        .automation_updates()
        .into_iter()
        .find(|(updated_task_id, _)| updated_task_id == "T-SYSTEM")
        .expect("task automation update");
    assert_eq!(update.model, None);
}
