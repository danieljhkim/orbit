use orbit_common::types::{
    OrbitError, OrbitEvent, Task, TaskPriority, TaskStatus, normalize_optional_attribution_label,
};
use orbit_engine::{TaskAutomationUpdate, TaskReadHost, TaskWriteHost};

use crate::OrbitRuntime;
use crate::command::task::SYSTEM_ACTOR_LABEL;
use crate::runtime::TaskRecordUpdateParams as StoreTaskUpdateParams;

impl TaskReadHost for OrbitRuntime {
    fn get_task(&self, task_id: &str) -> Result<Task, OrbitError> {
        OrbitRuntime::get_task(self, task_id)
    }

    fn get_task_artifacts(
        &self,
        task_id: &str,
    ) -> Result<Vec<orbit_common::types::TaskArtifact>, OrbitError> {
        OrbitRuntime::get_task_artifacts(self, task_id)
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        OrbitRuntime::list_tasks_filtered(self, status, priority, parent_id, batch_id)
    }
}

impl TaskWriteHost for OrbitRuntime {
    fn start_task(
        &self,
        task_id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        OrbitRuntime::start_task_as_system(self, task_id, note, comment)
    }

    fn update_task_from_activity(
        &self,
        task_id: &str,
        status: TaskStatus,
        execution_summary: Option<String>,
        comment: Option<String>,
        note: Option<String>,
    ) -> Result<Task, OrbitError> {
        OrbitRuntime::update_task_from_activity(
            self,
            task_id,
            status,
            execution_summary,
            comment,
            note,
        )
    }

    fn apply_task_automation_update(
        &self,
        task_id: &str,
        update: TaskAutomationUpdate,
    ) -> Result<(), OrbitError> {
        let existing_task = self.get_task(task_id)?;
        if update.status == Some(TaskStatus::InProgress)
            && crate::command::task::in_progress_transition_requires_plan(existing_task.status)
        {
            crate::command::task::ensure_task_has_execution_plan(
                task_id,
                existing_task.plan.as_str(),
            )?;
        }
        let _ = self.with_mutation(|| {
            let (agent, model) = self
                .canonical_agent_model_identity(update.agent.as_deref(), update.model.as_deref());
            let actor_label = SYSTEM_ACTOR_LABEL.to_string();
            let explicit_attribution_label = normalize_optional_attribution_label(
                update
                    .model
                    .as_deref()
                    .or(model.as_deref())
                    .or(update.agent.as_deref())
                    .or(agent.as_deref()),
                model.as_deref(),
            );
            let planned_by = update.plan.as_ref().map(|_| {
                Some(
                    explicit_attribution_label
                        .clone()
                        .unwrap_or_else(|| actor_label.clone()),
                )
            });
            let implemented_by = normalize_optional_attribution_label(
                existing_task
                    .model
                    .as_deref()
                    .or(model.as_deref())
                    .or(existing_task.implemented_by.as_deref())
                    .or(explicit_attribution_label.as_deref())
                    .or(Some(actor_label.as_str())),
                existing_task.model.as_deref().or(model.as_deref()),
            );
            let task = self.stores().tasks().update(
                task_id,
                StoreTaskUpdateParams {
                    actor: actor_label.clone(),
                    execution_summary: update.execution_summary.clone(),
                    plan: update.plan.clone(),
                    planned_by,
                    implemented_by: if matches!(
                        update.status,
                        Some(TaskStatus::Review | TaskStatus::Done)
                    ) {
                        implemented_by.clone().map(Some)
                    } else {
                        None
                    },
                    agent: agent.clone().map(Some),
                    model: model.clone().map(Some),
                    status: update.status,
                    workspace_path: update.workspace_path.clone(),
                    repo_root: update.repo_root.clone().map(Some),
                    pr_number: update.pr_number.clone().map(Some),
                    batch_id: update.batch_id.clone().map(Some),
                    status_event: update.status_event.clone(),
                    status_note: update.status_note.clone(),
                    append_comments: update.append_comments.clone(),
                    replace_review_threads: update.review_threads.clone(),
                    ..Default::default()
                },
            )?;
            Ok((
                task.clone(),
                OrbitEvent::TaskUpdated {
                    id: task_id.to_string(),
                },
            ))
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use crate::command::task::{TaskAddParams, TaskUpdateParams};
    use serde_json::json;
    use tempfile::tempdir;

    fn test_runtime() -> (tempfile::TempDir, OrbitRuntime) {
        let root = tempdir().expect("create tempdir");
        let global_root = root.path().join("global");
        let repo_root = root.path().join("repo");
        let workspace_root = repo_root.join(".orbit");
        std::fs::create_dir_all(&global_root).expect("create global root");
        std::fs::create_dir_all(&workspace_root).expect("create workspace root");
        let runtime =
            OrbitRuntime::from_roots(&global_root, &workspace_root).expect("build test runtime");
        (root, runtime)
    }

    #[test]
    fn automation_can_restamp_in_progress_task_without_plan() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Restamp task metadata".to_string(),
                description: "Exercise idempotent in-progress automation updates.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");

        assert!(task.plan.is_empty());
        let started = runtime
            .start_task(&task.id, Some("start from backlog".to_string()), None)
            .expect("start backlog task without plan");
        assert_eq!(started.status, TaskStatus::InProgress);

        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    batch_id: Some("jrun-test".to_string()),
                    workspace_path: Some(Some("/tmp/orbit-worktree".to_string())),
                    status: Some(TaskStatus::InProgress),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("restamp in-progress task metadata");

        let updated = runtime.get_task(&task.id).expect("reload task");
        assert_eq!(updated.status, TaskStatus::InProgress);
        assert_eq!(updated.batch_id.as_deref(), Some("jrun-test"));
        assert_eq!(
            updated.workspace_path.as_deref(),
            Some("/tmp/orbit-worktree")
        );
    }

    #[test]
    fn update_task_automation_records_status_history_as_system() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Review automated update".to_string(),
                description: "Exercise update_task automation attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");
        runtime
            .start_task(&task.id, Some("human starts work".to_string()), None)
            .expect("start task");
        runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    execution_summary: Some("Implemented and validated.".to_string()),
                    ..Default::default()
                },
            )
            .expect("set execution summary");

        orbit_engine::execute_deterministic_action(
            &runtime,
            "update_task",
            &json!({
                "task_id": task.id.clone(),
                "status": "review"
            }),
            false,
            &HashMap::new(),
            None,
        )
        .expect("run update_task automation");

        let updated = runtime.get_task(&task.id).expect("reload task");
        let status_entry = updated
            .history
            .iter()
            .rev()
            .find(|entry| {
                entry.event == "status_changed" && entry.to_status == Some(TaskStatus::Review)
            })
            .expect("review transition history");
        assert_eq!(status_entry.by, SYSTEM_ACTOR_LABEL);
        assert_eq!(
            status_entry.note.as_deref(),
            Some("automation: update_task \u{2192} review")
        );
    }

    #[test]
    fn activity_update_comment_records_comment_as_system() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Activity comment".to_string(),
                description: "Exercise activity comment attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");

        runtime
            .update_task_from_activity(
                &task.id,
                TaskStatus::InProgress,
                None,
                Some("Automation left a note.".to_string()),
                Some("automation start".to_string()),
            )
            .expect("activity update");

        let updated = runtime.get_task(&task.id).expect("reload task");
        let comment = updated.comments.last().expect("activity comment");
        assert_eq!(comment.by, SYSTEM_ACTOR_LABEL);
        let comment_history = updated
            .history
            .iter()
            .find(|entry| entry.event == "commented")
            .expect("comment history");
        assert_eq!(comment_history.by, SYSTEM_ACTOR_LABEL);
    }

    #[test]
    fn generic_automation_status_update_uses_system_history_and_preserves_implementer() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Generic automation".to_string(),
                description: "Exercise TaskAutomationUpdate attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");
        runtime
            .start_task(&task.id, Some("human starts work".to_string()), None)
            .expect("start task");

        runtime
            .apply_task_automation_update(
                &task.id,
                TaskAutomationUpdate {
                    status: Some(TaskStatus::Review),
                    execution_summary: Some("Automated handoff complete.".to_string()),
                    agent: Some("codex".to_string()),
                    model: Some("gpt-test".to_string()),
                    ..TaskAutomationUpdate::default()
                },
            )
            .expect("automation update");

        let updated = runtime.get_task(&task.id).expect("reload task");
        let status_entry = updated
            .history
            .iter()
            .rev()
            .find(|entry| {
                entry.event == "status_changed" && entry.to_status == Some(TaskStatus::Review)
            })
            .expect("review transition history");
        assert_eq!(status_entry.by, SYSTEM_ACTOR_LABEL);
        assert_eq!(updated.implemented_by.as_deref(), Some("gpt-test"));
    }

    #[test]
    fn direct_update_task_keeps_default_human_attribution() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Human comment".to_string(),
                description: "Exercise direct update attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");

        runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    comment: Some("Human-visible note.".to_string()),
                    ..Default::default()
                },
            )
            .expect("update task");

        let updated = runtime.get_task(&task.id).expect("reload task");
        let comment = updated.comments.last().expect("human comment");
        assert_eq!(comment.by, "human");
        let comment_history = updated
            .history
            .iter()
            .find(|entry| entry.event == "commented")
            .expect("comment history");
        assert_eq!(comment_history.by, "human");
    }

    #[test]
    fn dispatch_batch_claim_records_start_and_comment_as_system() {
        let (_root, runtime) = test_runtime();
        let task = runtime
            .add_task(TaskAddParams {
                title: "Claim in batch".to_string(),
                description: "Exercise dispatch_batch attribution.".to_string(),
                workspace_path: Some(".".to_string()),
                ..Default::default()
            })
            .expect("add task");

        orbit_engine::execute_deterministic_action(
            &runtime,
            "dispatch_batch",
            &json!({
                "run_id": "jrun-test",
                "parallelism": 1,
                "task_ids": [task.id.clone()]
            }),
            false,
            &HashMap::new(),
            None,
        )
        .expect("dispatch batch");

        let updated = runtime.get_task(&task.id).expect("reload task");
        let start_entry = updated
            .history
            .iter()
            .find(|entry| entry.event == "started")
            .expect("start history");
        assert_eq!(start_entry.by, SYSTEM_ACTOR_LABEL);
        let batch_comment = updated
            .comments
            .iter()
            .find(|comment| comment.message.starts_with("Batch dispatched:"))
            .expect("batch dispatch comment");
        assert_eq!(batch_comment.by, SYSTEM_ACTOR_LABEL);
    }
}
