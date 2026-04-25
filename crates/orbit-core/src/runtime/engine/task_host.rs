use orbit_common::types::{
    OrbitError, OrbitEvent, Task, TaskPriority, TaskStatus, normalize_optional_attribution_label,
};
use orbit_engine::{TaskAutomationUpdate, TaskReadHost, TaskWriteHost};

use crate::OrbitRuntime;
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
        OrbitRuntime::start_task(self, task_id, note, comment)
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
            let actor_label = normalize_optional_attribution_label(
                update
                    .model
                    .as_deref()
                    .or(update.agent.as_deref())
                    .or(Some("agent")),
                existing_task.model.as_deref().or(model.as_deref()),
            )
            .unwrap_or_else(|| "agent".to_string());
            let implemented_by = normalize_optional_attribution_label(
                existing_task
                    .model
                    .as_deref()
                    .or(model.as_deref())
                    .or(existing_task.implemented_by.as_deref())
                    .or(Some(actor_label.as_str())),
                existing_task.model.as_deref().or(model.as_deref()),
            );
            let task = self.stores().tasks().update(
                task_id,
                StoreTaskUpdateParams {
                    actor: actor_label.clone(),
                    execution_summary: update.execution_summary.clone(),
                    plan: update.plan.clone(),
                    planned_by: update.plan.as_ref().map(|_| Some(actor_label.clone())),
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

    use crate::command::task::TaskAddParams;
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
}
