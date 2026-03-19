use chrono::Utc;
use orbit_store::{
    TaskCreateParams as StoreTaskCreateParams, TaskUpdateParams as StoreTaskUpdateParams,
};
use orbit_types::{
    OrbitError, OrbitEvent, Task, TaskComment, TaskHistoryEntry, TaskPriority, TaskStatus, TaskType,
};

use crate::OrbitRuntime;
use crate::paths::normalize_path;

pub struct TaskAddParams {
    pub title: String,
    pub description: String,
    pub plan: String,
    pub comment: Option<String>,
    pub context_files: Vec<String>,
    pub workspace_path: Option<String>,
    pub priority: TaskPriority,
    pub task_type: TaskType,
}

impl Default for TaskAddParams {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            plan: String::new(),
            comment: None,
            context_files: Vec::new(),
            workspace_path: None,
            priority: TaskPriority::Medium,
            task_type: TaskType::Task,
        }
    }
}

pub struct TaskUpdateParams {
    pub title: Option<String>,
    pub description: Option<String>,
    pub plan: Option<String>,
    pub execution_summary: Option<String>,
    pub comment: Option<String>,
    pub status: Option<TaskStatus>,
    pub branch: Option<Option<String>>,
    pub pr_number: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskActorKind {
    Human,
    Agent,
}

#[derive(Debug, Clone)]
struct EffectiveTaskActor {
    kind: TaskActorKind,
    label: String,
}

const ORBIT_TASK_ACTOR_KIND: &str = "ORBIT_TASK_ACTOR_KIND";
const ORBIT_TASK_ACTOR_IDENTITY_ID: &str = "ORBIT_TASK_ACTOR_IDENTITY_ID";

fn effective_task_actor() -> EffectiveTaskActor {
    let kind_raw = std::env::var(ORBIT_TASK_ACTOR_KIND).ok();
    let identity_id = std::env::var(ORBIT_TASK_ACTOR_IDENTITY_ID)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let kind = match kind_raw.as_deref() {
        Some("agent") => TaskActorKind::Agent,
        _ if identity_id.is_some() => TaskActorKind::Agent,
        _ => TaskActorKind::Human,
    };
    let label = identity_id.unwrap_or_else(|| match kind {
        TaskActorKind::Human => "human".to_string(),
        TaskActorKind::Agent => "agent".to_string(),
    });

    EffectiveTaskActor { kind, label }
}

impl OrbitRuntime {
    pub fn add_task(&self, params: TaskAddParams) -> Result<Task, OrbitError> {
        let workspace_path = normalize_path(params.workspace_path)?;
        let actor = effective_task_actor();
        let initial_status = if actor.kind == TaskActorKind::Agent
            && self.context.task_approval_required_for_agent
        {
            TaskStatus::Proposed
        } else {
            TaskStatus::Backlog
        };
        let comments = build_task_comments(params.comment.clone(), actor.label.as_str())?;

        self.with_mutation(|| {
            let task = self.context.task_store.create_task(StoreTaskCreateParams {
                actor: actor.label.clone(),
                title: params.title.clone(),
                description: params.description.clone(),
                plan: params.plan.clone(),
                execution_summary: String::new(),
                context_files: params.context_files.clone(),
                workspace_path: workspace_path.clone(),
                created_by: Some(actor.label.clone()),
                assigned_to: Some(actor.label.clone()),
                status: initial_status,
                priority: params.priority,
                task_type: params.task_type,
                branch: None,
                pr_number: None,
                proposed_by: Some(actor.label.clone()),
                comments: comments.clone(),
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
        let is_field_update = params.title.is_some()
            || params.description.is_some()
            || params.plan.is_some()
            || params.execution_summary.is_some()
            || params.comment.is_some()
            || params.branch.is_some()
            || params.pr_number.is_some();

        if is_field_update && matches!(task.status, TaskStatus::Done | TaskStatus::Archived) {
            return Err(OrbitError::InvalidInput(format!(
                "task {id} is {} and cannot be modified; unarchive or reopen it first",
                task.status
            )));
        }

        if let Some(target_status) = params.status {
            if target_status == TaskStatus::Archived {
                return Err(OrbitError::InvalidInput(
                    "use `orbit task archive <id>` instead of setting status to archived"
                        .to_string(),
                ));
            }
            if target_status == TaskStatus::Rejected {
                return Err(OrbitError::InvalidInput(
                    "use `orbit task reject <id>` instead of setting status to rejected"
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
                    "task '{id}' requires non-empty execution_summary before transitioning in-progress -> review"
                )));
            }
        }

        let actor = effective_task_actor();
        let append_comments = build_task_comments(params.comment.clone(), actor.label.as_str())?;
        let assigned_to = params.status.and_then(|status| {
            if status == TaskStatus::InProgress {
                Some(Some(actor.label.clone()))
            } else {
                None
            }
        });

        let task = self.with_mutation(|| {
            let task = self.context.task_store.update_task(
                id,
                StoreTaskUpdateParams {
                    actor: actor.label.clone(),
                    title: params.title,
                    description: params.description,
                    plan: params.plan,
                    execution_summary: params.execution_summary,
                    assigned_to,
                    status: params.status,
                    branch: params.branch,
                    pr_number: params.pr_number,
                    append_comments: append_comments.clone(),
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
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        let task = self.get_task(id)?;
        let actor = effective_task_actor();
        let append_comments = build_task_comments(comment, actor.label.as_str())?;

        match task.status {
            TaskStatus::Proposed => {
                let task = self.with_mutation(|| {
                    let task = self.context.task_store.update_task(
                        id,
                        StoreTaskUpdateParams {
                            actor: actor.label.clone(),
                            status: Some(TaskStatus::Backlog),
                            status_event: Some("proposal_approved".to_string()),
                            status_note: note.clone(),
                            assigned_to: Some(Some(actor.label.clone())),
                            append_comments: append_comments.clone(),
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        task.clone(),
                        OrbitEvent::TaskProposalApproved {
                            id: id.to_string(),
                            approved_by: actor.label.clone(),
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
                            actor: actor.label.clone(),
                            status: Some(TaskStatus::Done),
                            status_event: Some("review_approved".to_string()),
                            status_note: note.clone(),
                            append_comments: append_comments.clone(),
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        task.clone(),
                        OrbitEvent::TaskReviewApproved {
                            id: id.to_string(),
                            approved_by: actor.label.clone(),
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

    pub fn start_task(
        &self,
        id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        let task = self.get_task(id)?;
        let actor = effective_task_actor();
        let append_comments = build_task_comments(comment, actor.label.as_str())?;

        match task.status {
            TaskStatus::Proposed => {
                let task = self.with_mutation(|| {
                    let at = Utc::now();
                    let task = self.context.task_store.update_task(
                        id,
                        StoreTaskUpdateParams {
                            actor: actor.label.clone(),
                            status: Some(TaskStatus::InProgress),
                            status_event: Some("started".to_string()),
                            assigned_to: Some(Some(actor.label.clone())),
                            append_history: vec![TaskHistoryEntry {
                                at,
                                by: actor.label.clone(),
                                event: "proposal_approved".to_string(),
                                note: note.clone(),
                            }],
                            append_comments: append_comments.clone(),
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        task.clone(),
                        OrbitEvent::TaskStarted {
                            id: id.to_string(),
                            started_by: actor.label.clone(),
                            approved_from_proposed: true,
                        },
                    ))
                })?;
                Ok(task)
            }
            TaskStatus::Backlog | TaskStatus::Blocked => {
                let task = self.with_mutation(|| {
                    let task = self.context.task_store.update_task(
                        id,
                        StoreTaskUpdateParams {
                            actor: actor.label.clone(),
                            status: Some(TaskStatus::InProgress),
                            status_event: Some("started".to_string()),
                            status_note: note.clone(),
                            assigned_to: Some(Some(actor.label.clone())),
                            append_comments: append_comments.clone(),
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        task.clone(),
                        OrbitEvent::TaskStarted {
                            id: id.to_string(),
                            started_by: actor.label.clone(),
                            approved_from_proposed: false,
                        },
                    ))
                })?;
                Ok(task)
            }
            TaskStatus::InProgress => Err(OrbitError::InvalidInput(format!(
                "task '{id}' is already in-progress"
            ))),
            other => Err(OrbitError::InvalidInput(format!(
                "task '{id}' is in status '{other}'; start requires 'proposed', 'backlog', or 'blocked'"
            ))),
        }
    }

    pub fn reject_task(
        &self,
        id: &str,
        note: String,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        let task = self.get_task(id)?;
        let actor = effective_task_actor();
        let reason = note.trim();
        if reason.is_empty() {
            return Err(OrbitError::InvalidInput(
                "rejection note must not be empty".to_string(),
            ));
        }
        let reason = reason.to_string();
        let append_comments = build_task_comments(comment, actor.label.as_str())?;

        match task.status {
            TaskStatus::Proposed => {
                let task = self.with_mutation(|| {
                    let task = self.context.task_store.update_task(
                        id,
                        StoreTaskUpdateParams {
                            actor: actor.label.clone(),
                            status: Some(TaskStatus::Rejected),
                            status_event: Some("proposal_rejected".to_string()),
                            status_note: Some(reason.clone()),
                            append_comments: append_comments.clone(),
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        task.clone(),
                        OrbitEvent::TaskProposalRejected {
                            id: id.to_string(),
                            rejected_by: actor.label.clone(),
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
                            actor: actor.label.clone(),
                            status: Some(TaskStatus::Rejected),
                            status_event: Some("review_rejected".to_string()),
                            status_note: Some(reason.clone()),
                            append_comments: append_comments.clone(),
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        task.clone(),
                        OrbitEvent::TaskReviewRejected {
                            id: id.to_string(),
                            rejected_by: actor.label.clone(),
                        },
                    ))
                })?;
                Ok(task)
            }
            other => Err(OrbitError::InvalidInput(format!(
                "task '{id}' is in status '{other}'; reject requires 'proposed' or 'review'"
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
                    actor: effective_task_actor().label,
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
                    actor: effective_task_actor().label,
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

fn build_task_comments(message: Option<String>, by: &str) -> Result<Vec<TaskComment>, OrbitError> {
    let Some(message) = message else {
        return Ok(Vec::new());
    };
    let message = message.trim();
    if message.is_empty() {
        return Err(OrbitError::InvalidInput(
            "task comment must not be empty".to_string(),
        ));
    }
    let by = by.trim();
    if by.is_empty() {
        return Err(OrbitError::InvalidInput(
            "task comment author must not be empty".to_string(),
        ));
    }

    Ok(vec![TaskComment {
        at: Utc::now(),
        by: by.to_string(),
        message: message.to_string(),
    }])
}
