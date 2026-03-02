use orbit_store::{
    TaskCreateParams as StoreTaskCreateParams, TaskUpdateParams as StoreTaskUpdateParams,
};
use orbit_types::{OrbitError, OrbitEvent, Task, TaskPriority, TaskStatus, TaskType};

use crate::OrbitRuntime;
use crate::paths::normalize_path;

pub struct TaskAddParams {
    pub title: String,
    pub description: String,
    pub instructions: String,
    pub context_files: Vec<String>,
    pub workspace_path: Option<String>,
    pub assigned_to: Option<String>,
    pub created_by: Option<String>,
    pub priority: TaskPriority,
    pub task_type: TaskType,
    pub proposed_by: Option<String>,
}

impl Default for TaskAddParams {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            instructions: String::new(),
            context_files: Vec::new(),
            workspace_path: None,
            assigned_to: None,
            created_by: None,
            priority: TaskPriority::Medium,
            task_type: TaskType::Task,
            proposed_by: None,
        }
    }
}

pub struct TaskUpdateParams {
    pub execution_summary: Option<String>,
    pub assigned_to: Option<Option<String>>,
    pub status: Option<TaskStatus>,
    pub branch: Option<Option<String>>,
    pub pr_number: Option<Option<String>>,
}

impl OrbitRuntime {
    pub fn add_task(&self, params: TaskAddParams) -> Result<Task, OrbitError> {
        let workspace_path = normalize_path(params.workspace_path)?;
        let initial_status = if self.context.task_approval_required_for_agent {
            TaskStatus::Proposed
        } else {
            TaskStatus::Backlog
        };
        let proposed_by = params
            .proposed_by
            .clone()
            .or_else(|| params.created_by.clone());

        self.with_mutation(|| {
            let task = self.context.task_store.create_task(StoreTaskCreateParams {
                title: params.title.clone(),
                description: params.description.clone(),
                instructions: params.instructions.clone(),
                execution_summary: String::new(),
                context_files: params.context_files.clone(),
                workspace_path: workspace_path.clone(),
                assigned_to: params.assigned_to.clone(),
                created_by: params.created_by.clone(),
                status: initial_status,
                priority: params.priority,
                task_type: params.task_type,
                branch: None,
                pr_number: None,
                proposed_by: proposed_by.clone(),
            })?;
            Ok((
                task.clone(),
                OrbitEvent::TaskAdded {
                    id: task.id.clone(),
                },
            ))
        })
    }

    pub fn get_task(&self, id: &str) -> Result<Task, OrbitError> {
        self.context
            .task_store
            .get_task(id)?
            .ok_or_else(|| OrbitError::TaskNotFound(id.to_string()))
    }

    pub fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        self.context.task_store.list_tasks()
    }

    pub fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.context
            .task_store
            .list_tasks_filtered(status, priority)
    }

    pub fn update_task(&self, id: &str, params: TaskUpdateParams) -> Result<Task, OrbitError> {
        let task = self.get_task(id)?;

        if let Some(target_status) = params.status {
            if target_status == TaskStatus::Archived {
                return Err(OrbitError::InvalidInput(
                    "use `orbit task archive <id>` instead of setting status to archived"
                        .to_string(),
                ));
            }
            task.status
                .validate_transition(target_status)
                .map_err(OrbitError::TaskStatusTransition)?;
        }

        if task.status == TaskStatus::InProgress && params.status == Some(TaskStatus::Review) {
            let effective_execution_summary = params
                .execution_summary
                .as_deref()
                .unwrap_or(task.execution_summary.as_str());
            if effective_execution_summary.trim().is_empty() {
                return Err(OrbitError::InvalidInput(format!(
                    "task '{id}' requires non-empty execution_summary before transitioning in_progress -> review"
                )));
            }
        }

        let task = self.with_mutation(|| {
            let task = self.context.task_store.update_task(
                id,
                StoreTaskUpdateParams {
                    execution_summary: params.execution_summary,
                    assigned_to: params.assigned_to,
                    status: params.status,
                    branch: params.branch,
                    pr_number: params.pr_number,
                    ..Default::default()
                },
            )?;
            Ok((task.clone(), OrbitEvent::TaskUpdated { id: id.to_string() }))
        })?;

        Ok(task)
    }

    pub fn approve_task(
        &self,
        id: &str,
        approved_by: &str,
        note: Option<String>,
    ) -> Result<Task, OrbitError> {
        let task = self.get_task(id)?;
        let approver = approved_by.trim();
        if approver.is_empty() {
            return Err(OrbitError::InvalidInput(
                "approved_by must not be empty".to_string(),
            ));
        }

        match task.status {
            TaskStatus::Proposed => {
                let task = self.with_mutation(|| {
                    let task = self.context.task_store.update_task(
                        id,
                        StoreTaskUpdateParams {
                            status: Some(TaskStatus::Backlog),
                            proposal_approved_by: Some(Some(approver.to_string())),
                            proposal_decision_note: Some(note.clone()),
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        task.clone(),
                        OrbitEvent::TaskProposalApproved {
                            id: id.to_string(),
                            approved_by: approver.to_string(),
                        },
                    ))
                })?;
                Ok(task)
            }
            TaskStatus::Review => {
                let task = self.with_mutation(|| {
                    let task = self.context.task_store.update_task(
                        id,
                        StoreTaskUpdateParams {
                            status: Some(TaskStatus::Done),
                            review_approved_by: Some(Some(approver.to_string())),
                            review_decision_note: Some(note.clone()),
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        task.clone(),
                        OrbitEvent::TaskReviewApproved {
                            id: id.to_string(),
                            approved_by: approver.to_string(),
                        },
                    ))
                })?;
                Ok(task)
            }
            other => Err(OrbitError::InvalidInput(format!(
                "task '{id}' is in status '{other}'; approve requires 'proposed' or 'review'"
            ))),
        }
    }

    pub fn archive_task(&self, id: &str) -> Result<(), OrbitError> {
        let task = self.get_task(id)?;

        if task.status == TaskStatus::Archived {
            return Err(OrbitError::InvalidInput(format!(
                "task '{id}' is already archived"
            )));
        }

        self.with_mutation(|| {
            let _ = self.context.task_store.update_task(
                id,
                StoreTaskUpdateParams {
                    status: Some(TaskStatus::Archived),
                    ..Default::default()
                },
            )?;
            Ok(((), OrbitEvent::TaskArchived { id: id.to_string() }))
        })
    }

    pub fn unarchive_task(&self, id: &str) -> Result<(), OrbitError> {
        let task = self.get_task(id)?;

        if task.status != TaskStatus::Archived {
            return Err(OrbitError::InvalidInput(format!(
                "task '{id}' is not archived (status: {})",
                task.status
            )));
        }

        self.with_mutation(|| {
            let _ = self.context.task_store.update_task(
                id,
                StoreTaskUpdateParams {
                    status: Some(TaskStatus::Backlog),
                    ..Default::default()
                },
            )?;
            Ok(((), OrbitEvent::TaskUnarchived { id: id.to_string() }))
        })
    }

    pub fn delete_task(&self, id: &str) -> Result<(), OrbitError> {
        self.with_mutation(|| {
            let deleted = self.context.task_store.delete_task(id)?;
            if !deleted {
                return Err(OrbitError::TaskNotFound(id.to_string()));
            }
            Ok(((), OrbitEvent::TaskDeleted { id: id.to_string() }))
        })
    }

    pub fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.context.task_store.search_tasks(query)
    }
}
