use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use orbit_types::{OrbitError, Task, TaskHistoryEntry, TaskPriority, TaskStatus};

use super::contracts::{TaskCreateParams, TaskStoreBackend, TaskUpdateParams};

#[derive(Clone, Default)]
pub struct MemoryTaskStoreBackend {
    inner: Arc<Mutex<HashMap<String, Task>>>,
}

fn lock_err<T>(e: std::sync::PoisonError<T>) -> OrbitError {
    OrbitError::Store(format!("mutex poisoned: {e}"))
}

fn next_task_id(store: &HashMap<String, Task>, now: chrono::DateTime<Utc>) -> String {
    let base = format!("T{}", now.format("%Y%m%d-%H%M%S"));
    if !store.contains_key(&base) {
        return base;
    }
    for suffix in 2..1024_u32 {
        let candidate = format!("{base}-{suffix}");
        if !store.contains_key(&candidate) {
            return candidate;
        }
    }
    base
}

impl TaskStoreBackend for MemoryTaskStoreBackend {
    fn create_task(&self, params: TaskCreateParams) -> Result<Task, OrbitError> {
        if params.title.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task title must not be empty".to_string(),
            ));
        }
        if params.actor.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task actor must not be empty".to_string(),
            ));
        }
        let mut store = self.inner.lock().map_err(lock_err)?;
        let now = Utc::now();
        let id = next_task_id(&store, now);
        let task = Task {
            id: id.clone(),
            title: params.title,
            description: params.description,
            plan: params.plan,
            execution_summary: params.execution_summary,
            context_files: params.context_files,
            workspace_path: params.workspace_path,
            repo_root: None,
            assigned_to: params.assigned_to,
            created_by: params.created_by,
            status: params.status,
            priority: params.priority,
            complexity: params.complexity,
            task_type: params.task_type,
            branch: params.branch,
            commit_message: None,
            changed_files: None,
            pr_number: params.pr_number,
            proposed_by: params.proposed_by,
            comments: params.comments,
            history: vec![TaskHistoryEntry {
                at: now,
                by: params.actor,
                event: "created".to_string(),
                note: None,
                from_status: None,
                to_status: Some(params.status),
            }],
            created_at: now,
            updated_at: now,
        };
        store.insert(id, task.clone());
        Ok(task)
    }

    fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        let store = self.inner.lock().map_err(lock_err)?;
        let mut tasks: Vec<Task> = store.values().cloned().collect();
        tasks.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(tasks)
    }

    fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
    ) -> Result<Vec<Task>, OrbitError> {
        Ok(self
            .list_tasks()?
            .into_iter()
            .filter(|t| status.is_none_or(|s| t.status == s))
            .filter(|t| priority.is_none_or(|p| t.priority == p))
            .collect())
    }

    fn get_task(&self, id: &str) -> Result<Option<Task>, OrbitError> {
        let store = self.inner.lock().map_err(lock_err)?;
        Ok(store.get(id).cloned())
    }

    fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        let lowered = query.to_lowercase();
        Ok(self
            .list_tasks()?
            .into_iter()
            .filter(|t| {
                t.title.to_lowercase().contains(&lowered)
                    || t.description.to_lowercase().contains(&lowered)
            })
            .collect())
    }

    fn update_task(&self, id: &str, params: TaskUpdateParams) -> Result<Task, OrbitError> {
        if params.actor.trim().is_empty() {
            return Err(OrbitError::InvalidInput(
                "task actor must not be empty".to_string(),
            ));
        }
        let mut store = self.inner.lock().map_err(lock_err)?;
        let Some(task) = store.get_mut(id) else {
            return Err(OrbitError::TaskNotFound(id.to_string()));
        };

        let current_status = task.status;

        let title_changed = if let Some(ref title) = params.title {
            let changed = *title != task.title;
            task.title = title.clone();
            changed
        } else {
            false
        };
        if let Some(v) = params.description {
            task.description = v;
        }
        if let Some(v) = params.plan {
            task.plan = v;
        }
        if let Some(v) = params.execution_summary {
            task.execution_summary = v;
        }
        if let Some(v) = params.context_files {
            task.context_files = v;
        }
        if let Some(v) = params.workspace_path {
            task.workspace_path = v;
        }
        if let Some(v) = params.repo_root {
            task.repo_root = v;
        }
        if let Some(v) = params.assigned_to {
            task.assigned_to = v;
        }
        if let Some(v) = params.created_by {
            task.created_by = v;
        }
        if let Some(v) = params.priority {
            task.priority = v;
        }
        if let Some(v) = params.complexity {
            task.complexity = Some(v);
        }
        if let Some(v) = params.task_type {
            task.task_type = v;
        }
        if let Some(v) = params.branch {
            task.branch = v;
        }
        if let Some(v) = params.commit_message {
            task.commit_message = v;
        }
        if let Some(v) = params.changed_files {
            task.changed_files = v;
        }
        if let Some(v) = params.pr_number {
            task.pr_number = v;
        }
        if let Some(v) = params.proposed_by {
            task.proposed_by = v;
        }
        if !params.append_history.is_empty() {
            task.history.extend(params.append_history.clone());
        }
        if !params.append_comments.is_empty() {
            task.comments.extend(params.append_comments.clone());
        }

        let new_status = params.status.unwrap_or(current_status);
        let status_transition =
            (new_status != current_status).then_some((current_status, new_status));

        task.updated_at = Utc::now();

        let event = if let Some(event) = params.status_event.clone() {
            Some(event)
        } else if status_transition.is_some() {
            Some("status_changed".to_string())
        } else {
            None
        };

        if let Some(event) = event {
            task.history.push(TaskHistoryEntry {
                at: task.updated_at,
                by: params.actor.clone(),
                event,
                note: params.status_note,
                from_status: status_transition.map(|(from, _)| from),
                to_status: status_transition.map(|(_, to)| to),
            });
        }
        if title_changed {
            task.history.push(TaskHistoryEntry {
                at: task.updated_at,
                by: params.actor,
                event: "renamed".to_string(),
                note: None,
                from_status: None,
                to_status: None,
            });
        }
        if let Some(new_status) = params.status {
            task.status = new_status;
        }

        Ok(task.clone())
    }

    fn delete_task(&self, id: &str) -> Result<bool, OrbitError> {
        let mut store = self.inner.lock().map_err(lock_err)?;
        Ok(store.remove(id).is_some())
    }
}

#[cfg(test)]
mod tests {
    use orbit_types::{TaskComment, TaskPriority, TaskStatus, TaskType};

    use super::MemoryTaskStoreBackend;
    use crate::backend::contracts::{TaskCreateParams, TaskStoreBackend, TaskUpdateParams};

    fn sample_params(status: TaskStatus) -> TaskCreateParams {
        TaskCreateParams {
            actor: "test-agent".to_string(),
            title: "Test task".to_string(),
            description: "Test description".to_string(),
            plan: "Test plan".to_string(),
            execution_summary: String::new(),
            context_files: vec![],
            workspace_path: None,
            created_by: Some("human".to_string()),
            assigned_to: None,
            status,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Task,
            branch: None,
            pr_number: None,
            proposed_by: None,
            comments: vec![],
        }
    }

    #[test]
    fn create_and_get_task_roundtrip() {
        let store = MemoryTaskStoreBackend::default();
        let task = store
            .create_task(sample_params(TaskStatus::Backlog))
            .expect("create");
        assert_eq!(task.status, TaskStatus::Backlog);
        assert_eq!(task.history.len(), 1);
        assert_eq!(task.history[0].event, "created");

        let got = store.get_task(&task.id).expect("get").expect("exists");
        assert_eq!(got.id, task.id);
        assert_eq!(got.plan, "Test plan");
    }

    #[test]
    fn list_and_search_tasks() {
        let store = MemoryTaskStoreBackend::default();
        store
            .create_task(sample_params(TaskStatus::Backlog))
            .expect("create 1");
        store
            .create_task(TaskCreateParams {
                title: "Other task".to_string(),
                description: "Searchable phrase".to_string(),
                ..sample_params(TaskStatus::Done)
            })
            .expect("create 2");

        assert_eq!(store.list_tasks().expect("list").len(), 2);
        let matches = store.search_tasks("searchable").expect("search");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].description, "Searchable phrase");
    }

    #[test]
    fn update_task_status_appends_history() {
        let store = MemoryTaskStoreBackend::default();
        let task = store
            .create_task(sample_params(TaskStatus::Backlog))
            .expect("create");

        let updated = store
            .update_task(
                &task.id,
                TaskUpdateParams {
                    actor: "agent".to_string(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .expect("update");

        assert_eq!(updated.status, TaskStatus::InProgress);
        assert_eq!(updated.history.len(), 2);
        assert_eq!(updated.history[1].event, "status_changed");
        assert_eq!(updated.history[1].from_status, Some(TaskStatus::Backlog));
        assert_eq!(updated.history[1].to_status, Some(TaskStatus::InProgress));
    }

    #[test]
    fn update_task_appends_comments() {
        let store = MemoryTaskStoreBackend::default();
        let task = store
            .create_task(sample_params(TaskStatus::Backlog))
            .expect("create");

        let updated = store
            .update_task(
                &task.id,
                TaskUpdateParams {
                    actor: "agent".to_string(),
                    append_comments: vec![TaskComment {
                        at: chrono::Utc::now(),
                        by: "reviewer".to_string(),
                        message: "looks good".to_string(),
                    }],
                    ..Default::default()
                },
            )
            .expect("update");

        assert_eq!(updated.comments.len(), 1);
        assert_eq!(updated.comments[0].by, "reviewer");
    }

    #[test]
    fn delete_task_removes_it() {
        let store = MemoryTaskStoreBackend::default();
        let task = store
            .create_task(sample_params(TaskStatus::Backlog))
            .expect("create");

        assert!(store.delete_task(&task.id).expect("delete"));
        assert!(store.get_task(&task.id).expect("get").is_none());
        assert!(!store.delete_task(&task.id).expect("delete again"));
    }

    #[test]
    fn list_tasks_filtered_by_status() {
        let store = MemoryTaskStoreBackend::default();
        store
            .create_task(sample_params(TaskStatus::Backlog))
            .expect("create backlog");
        store
            .create_task(sample_params(TaskStatus::Done))
            .expect("create done");

        let backlog = store
            .list_tasks_filtered(Some(TaskStatus::Backlog), None)
            .expect("filter");
        assert_eq!(backlog.len(), 1);
        assert_eq!(backlog[0].status, TaskStatus::Backlog);
    }

    #[test]
    fn create_task_rejects_empty_title() {
        let store = MemoryTaskStoreBackend::default();
        let err = store
            .create_task(TaskCreateParams {
                title: "  ".to_string(),
                ..sample_params(TaskStatus::Backlog)
            })
            .unwrap_err();
        assert!(err.to_string().contains("title must not be empty"));
    }
}
