use chrono::Utc;
use orbit_store::{
    TaskCreateParams as StoreTaskCreateParams, TaskUpdateParams as StoreTaskUpdateParams,
    friction_bounty,
};
use orbit_types::{
    ActorIdentity, OrbitError, OrbitEvent, OrbitId, Task, TaskComment, TaskComplexity,
    TaskHistoryEntry, TaskPriority, TaskStatus, TaskType,
};
use std::path::{Path, PathBuf};

use crate::OrbitRuntime;
use crate::context::ActorKind;

pub struct TaskAddParams {
    pub parent_id: Option<OrbitId>,
    pub title: String,
    pub description: String,
    pub acceptance_criteria: Vec<String>,
    pub plan: String,
    pub comment: Option<String>,
    pub context_files: Vec<String>,
    pub workspace_path: Option<String>,
    pub priority: TaskPriority,
    pub complexity: Option<TaskComplexity>,
    pub task_type: TaskType,
    pub source_task_id: Option<String>,
}

impl Default for TaskAddParams {
    fn default() -> Self {
        Self {
            parent_id: None,
            title: String::new(),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            plan: String::new(),
            comment: None,
            context_files: Vec::new(),
            workspace_path: None,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Task,
            source_task_id: None,
        }
    }
}

#[derive(Default)]
pub struct TaskUpdateParams {
    pub title: Option<String>,
    pub description: Option<String>,
    pub acceptance_criteria: Option<Vec<String>>,
    pub plan: Option<String>,
    pub execution_summary: Option<String>,
    pub comment: Option<String>,
    pub status: Option<TaskStatus>,
    pub pr_number: Option<Option<String>>,
    pub pr_status: Option<Option<String>>,
    pub batch_id: Option<Option<String>>,
    pub context_files: Option<Vec<String>>,
    pub append_review_threads: Vec<orbit_types::ReviewThread>,
}

impl From<TaskUpdateParams> for StoreTaskUpdateParams {
    fn from(p: TaskUpdateParams) -> Self {
        Self {
            title: p.title,
            description: p.description,
            acceptance_criteria: p.acceptance_criteria,
            plan: p.plan,
            execution_summary: p.execution_summary,
            status: p.status,
            pr_number: p.pr_number,
            pr_status: p.pr_status,
            batch_id: p.batch_id,
            context_files: p.context_files,
            append_review_threads: p.append_review_threads,
            ..Default::default()
        }
    }
}

impl OrbitRuntime {
    pub fn add_task(&self, params: TaskAddParams) -> Result<Task, OrbitError> {
        self.add_task_with_identity(params, None, None)
    }

    pub fn add_task_with_identity(
        &self,
        params: TaskAddParams,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Task, OrbitError> {
        let actor = self.actor().clone();
        let effective_label =
            effective_actor_label(&actor.label, agent.as_deref(), model.as_deref());
        let initial_status =
            if actor.kind == ActorKind::Agent && self.task_approval_required_for_agent() {
                TaskStatus::Proposed
            } else {
                TaskStatus::Backlog
            };
        let is_friction = params.task_type.is_friction();
        let (create_actor, create_identity, create_label) = if is_friction {
            (
                "system".to_string(),
                ActorIdentity::System,
                "system".to_string(),
            )
        } else {
            (
                effective_label.clone(),
                ActorIdentity::from_legacy(agent.as_deref(), model.as_deref()),
                effective_label.clone(),
            )
        };
        let comments = build_task_comments(params.comment.clone(), effective_label.as_str())?;
        let workspace_path =
            normalize_workspace_path(&self.paths().repo_root, params.workspace_path.as_deref())?;

        let task = self.with_mutation(|| {
            let task = self.create_task_record(StoreTaskCreateParams {
                actor: create_actor.clone(),
                parent_id: params.parent_id.clone(),
                title: params.title.clone(),
                description: params.description.clone(),
                acceptance_criteria: params.acceptance_criteria.clone(),
                plan: params.plan.clone(),
                execution_summary: String::new(),
                context_files: params.context_files.clone(),
                workspace_path: workspace_path.clone(),
                repo_root: None,
                created_by: Some(create_label.clone()),
                actor_identity: create_identity.clone(),
                assigned_to: Some(create_label.clone()),
                status: initial_status,
                priority: params.priority,
                complexity: params.complexity,
                task_type: params.task_type,
                pr_number: None,
                proposed_by: Some(create_label.clone()),
                source_task_id: params.source_task_id.clone(),
                comments: comments.clone(),
            })?;
            Ok((
                task.clone(),
                OrbitEvent::TaskAdded {
                    id: task.id.clone(),
                },
            ))
        })?;

        // Friction bounty: record issues-reported on creation when agent+model present.
        if self.scoring_enabled()
            && params.task_type.is_friction()
            && let (Some(a), Some(m)) = (&agent, &model)
        {
            let _ = friction_bounty::record_friction_reported(&self.paths().scoreboard_dir, a, m);
        }

        Ok(task)
    }

    pub fn get_task(&self, id: &str) -> Result<Task, OrbitError> {
        self.get_task_record(id)?
            .ok_or_else(|| OrbitError::TaskNotFound(id.to_string()))
    }

    pub fn list_tasks(&self) -> Result<Vec<Task>, OrbitError> {
        self.list_task_records()
    }

    pub fn list_tasks_filtered(
        &self,
        status: Option<TaskStatus>,
        priority: Option<TaskPriority>,
        parent_id: Option<&str>,
        batch_id: Option<&str>,
    ) -> Result<Vec<Task>, OrbitError> {
        self.list_task_records_filtered(status, priority, parent_id, batch_id)
    }

    pub fn update_task(&self, id: &str, params: TaskUpdateParams) -> Result<Task, OrbitError> {
        self.update_task_with_identity(id, params, None, None)
    }

    pub fn update_task_with_identity(
        &self,
        id: &str,
        params: TaskUpdateParams,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.update_task_with_status_note_and_identity(id, params, None, agent, model)
    }

    pub fn update_task_from_activity(
        &self,
        id: &str,
        status: TaskStatus,
        execution_summary: Option<String>,
        comment: Option<String>,
        note: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.update_task_with_status_note(
            id,
            TaskUpdateParams {
                execution_summary,
                comment,
                status: Some(status),
                ..Default::default()
            },
            note,
        )
    }

    fn update_task_with_status_note(
        &self,
        id: &str,
        params: TaskUpdateParams,
        status_note: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.update_task_with_status_note_and_identity(id, params, status_note, None, None)
    }

    fn update_task_with_status_note_and_identity(
        &self,
        id: &str,
        params: TaskUpdateParams,
        status_note: Option<String>,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Task, OrbitError> {
        let task = self.get_task(id)?;
        let locked_field_update = params.plan.is_some()
            || params.execution_summary.is_some()
            || params.comment.is_some()
            || params.pr_number.is_some()
            || params.pr_status.is_some()
            || params.batch_id.is_some();

        if locked_field_update && matches!(task.status, TaskStatus::Done | TaskStatus::Archived) {
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
            task.status
                .validate_transition(target_status)
                .map_err(OrbitError::TaskStatusTransition)?;
            if target_status == TaskStatus::InProgress && task.status != TaskStatus::InProgress {
                let effective_plan = params.plan.as_deref().unwrap_or(task.plan.as_str());
                ensure_task_has_execution_plan(id, effective_plan)?;
            }
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

        let actor = self.actor().clone();
        let effective_label =
            effective_actor_label(&actor.label, agent.as_deref(), model.as_deref());
        let status_note = status_note
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let append_comments =
            build_task_comments(params.comment.clone(), effective_label.as_str())?;
        let assigned_to = params.status.and_then(|status| {
            if status == TaskStatus::InProgress {
                Some(Some(effective_label.clone()))
            } else {
                None
            }
        });

        let old_status = task.status;
        let target_status = params.status;
        let updated = self.with_mutation(|| {
            let task = self.update_task_record(
                id,
                StoreTaskUpdateParams {
                    actor: effective_label.clone(),
                    assigned_to,
                    status_note,
                    actor_identity: agent
                        .as_ref()
                        .map(|_| ActorIdentity::from_legacy(agent.as_deref(), model.as_deref())),
                    append_comments: append_comments.clone(),
                    ..StoreTaskUpdateParams::from(params)
                },
            )?;
            Ok((task.clone(), OrbitEvent::TaskUpdated { id: id.to_string() }))
        })?;

        if let Some(new_status) = target_status
            && new_status != old_status
        {
            if should_release_file_locks(old_status, new_status) {
                let _ = self.file_lock_store().release_locks_for_task(id);
            }
            self.try_record_friction_transition(&task, old_status, new_status);
        }

        Ok(updated)
    }

    pub fn approve_task(
        &self,
        id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.approve_task_with_identity(id, note, comment, None, None)
    }

    pub fn approve_task_with_identity(
        &self,
        id: &str,
        note: Option<String>,
        comment: Option<String>,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Task, OrbitError> {
        let task = self.get_task(id)?;
        let actor = self.actor().clone();
        let effective_label =
            effective_actor_label(&actor.label, agent.as_deref(), model.as_deref());
        let append_comments = build_task_comments(comment, effective_label.as_str())?;

        let result = match task.status {
            TaskStatus::Proposed => self.with_mutation(|| {
                let task = self.update_task_record(
                    id,
                    StoreTaskUpdateParams {
                        actor: effective_label.clone(),
                        status: Some(TaskStatus::Backlog),
                        status_event: Some("proposal_approved".to_string()),
                        status_note: note.clone(),
                        assigned_to: Some(Some(effective_label.clone())),
                        actor_identity: agent.as_ref().map(|_| {
                            ActorIdentity::from_legacy(agent.as_deref(), model.as_deref())
                        }),
                        append_comments: append_comments.clone(),
                        ..Default::default()
                    },
                )?;
                Ok((
                    task.clone(),
                    OrbitEvent::TaskProposalApproved {
                        id: id.to_string(),
                        approved_by: effective_label.clone(),
                    },
                ))
            }),
            TaskStatus::Review => self.with_mutation(|| {
                let task = self.update_task_record(
                    id,
                    StoreTaskUpdateParams {
                        actor: effective_label.clone(),
                        status: Some(TaskStatus::Done),
                        status_event: Some("review_approved".to_string()),
                        status_note: note.clone(),
                        actor_identity: agent.as_ref().map(|_| {
                            ActorIdentity::from_legacy(agent.as_deref(), model.as_deref())
                        }),
                        append_comments: append_comments.clone(),
                        ..Default::default()
                    },
                )?;
                Ok((
                    task.clone(),
                    OrbitEvent::TaskReviewApproved {
                        id: id.to_string(),
                        approved_by: effective_label.clone(),
                    },
                ))
            }),
            other => Err(OrbitError::InvalidInput(format!(
                "task '{id}' is in status '{other}'; approve requires 'proposed' or 'review'"
            ))),
        }?;

        if task.status == TaskStatus::Review {
            let _ = self.file_lock_store().release_locks_for_task(id);
        }

        self.try_record_friction_transition(
            &task,
            task.status,
            if task.status == TaskStatus::Proposed {
                TaskStatus::Backlog
            } else {
                TaskStatus::Done
            },
        );

        Ok(result)
    }

    pub fn start_task(
        &self,
        id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.start_task_with_identity(id, note, comment, None, None)
    }

    pub fn start_task_with_identity(
        &self,
        id: &str,
        note: Option<String>,
        comment: Option<String>,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Task, OrbitError> {
        let task = self.get_task(id)?;
        ensure_task_has_execution_plan(id, task.plan.as_str())?;
        let actor = self.actor().clone();
        let effective_label =
            effective_actor_label(&actor.label, agent.as_deref(), model.as_deref());
        let append_comments = build_task_comments(comment, effective_label.as_str())?;

        match task.status {
            TaskStatus::Proposed => {
                let result = self.with_mutation(|| {
                    let at = Utc::now();
                    let task = self.update_task_record(
                        id,
                        StoreTaskUpdateParams {
                            actor: effective_label.clone(),
                            status: Some(TaskStatus::InProgress),
                            status_event: Some("started".to_string()),
                            assigned_to: Some(Some(effective_label.clone())),
                            actor_identity: agent.as_ref().map(|_| {
                                ActorIdentity::from_legacy(agent.as_deref(), model.as_deref())
                            }),
                            append_history: vec![TaskHistoryEntry {
                                at,
                                by: effective_label.clone(),
                                event: "proposal_approved".to_string(),
                                note: note.clone(),
                                from_status: Some(TaskStatus::Proposed),
                                to_status: Some(TaskStatus::Backlog),
                            }],
                            append_comments: append_comments.clone(),
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        task.clone(),
                        OrbitEvent::TaskStarted {
                            id: id.to_string(),
                            started_by: effective_label.clone(),
                            approved_from_proposed: true,
                        },
                    ))
                })?;
                self.try_record_friction_transition(
                    &task,
                    TaskStatus::Proposed,
                    TaskStatus::InProgress,
                );
                Ok(result)
            }
            TaskStatus::Backlog | TaskStatus::Someday | TaskStatus::Blocked => {
                let task = self.with_mutation(|| {
                    let task = self.update_task_record(
                        id,
                        StoreTaskUpdateParams {
                            actor: effective_label.clone(),
                            status: Some(TaskStatus::InProgress),
                            status_event: Some("started".to_string()),
                            status_note: note.clone(),
                            assigned_to: Some(Some(effective_label.clone())),
                            actor_identity: agent.as_ref().map(|_| {
                                ActorIdentity::from_legacy(agent.as_deref(), model.as_deref())
                            }),
                            append_comments: append_comments.clone(),
                            ..Default::default()
                        },
                    )?;
                    Ok((
                        task.clone(),
                        OrbitEvent::TaskStarted {
                            id: id.to_string(),
                            started_by: effective_label.clone(),
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
                "task '{id}' is in status '{other}'; start requires 'proposed', 'backlog', 'someday', or 'blocked'"
            ))),
        }
    }

    pub fn reject_task(
        &self,
        id: &str,
        note: String,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.reject_task_with_identity(id, note, comment, None, None)
    }

    pub fn reject_task_with_identity(
        &self,
        id: &str,
        note: String,
        comment: Option<String>,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Task, OrbitError> {
        let task = self.get_task(id)?;
        let actor = self.actor().clone();
        let effective_label =
            effective_actor_label(&actor.label, agent.as_deref(), model.as_deref());
        let reason = note.trim();
        if reason.is_empty() {
            return Err(OrbitError::InvalidInput(
                "rejection note must not be empty".to_string(),
            ));
        }
        let reason = reason.to_string();
        let append_comments = build_task_comments(comment, effective_label.as_str())?;

        let result = match task.status {
            TaskStatus::Proposed => self.with_mutation(|| {
                let task = self.update_task_record(
                    id,
                    StoreTaskUpdateParams {
                        actor: effective_label.clone(),
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
                        rejected_by: effective_label.clone(),
                    },
                ))
            }),
            TaskStatus::Review => self.with_mutation(|| {
                let task = self.update_task_record(
                    id,
                    StoreTaskUpdateParams {
                        actor: effective_label.clone(),
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
                        rejected_by: effective_label.clone(),
                    },
                ))
            }),
            TaskStatus::Backlog => self.with_mutation(|| {
                let task = self.update_task_record(
                    id,
                    StoreTaskUpdateParams {
                        actor: effective_label.clone(),
                        status: Some(TaskStatus::Rejected),
                        status_event: Some("backlog_rejected".to_string()),
                        status_note: Some(reason.clone()),
                        append_comments: append_comments.clone(),
                        ..Default::default()
                    },
                )?;
                Ok((
                    task.clone(),
                    OrbitEvent::TaskProposalRejected {
                        id: id.to_string(),
                        rejected_by: effective_label.clone(),
                    },
                ))
            }),
            TaskStatus::InProgress => self.with_mutation(|| {
                let task = self.update_task_record(
                    id,
                    StoreTaskUpdateParams {
                        actor: effective_label.clone(),
                        status: Some(TaskStatus::Rejected),
                        status_event: Some("in_progress_rejected".to_string()),
                        status_note: Some(reason.clone()),
                        append_comments: append_comments.clone(),
                        ..Default::default()
                    },
                )?;
                Ok((
                    task.clone(),
                    OrbitEvent::TaskProposalRejected {
                        id: id.to_string(),
                        rejected_by: effective_label.clone(),
                    },
                ))
            }),
            other => Err(OrbitError::InvalidInput(format!(
                "task '{id}' is in status '{other}'; reject requires 'proposed', 'review', 'backlog', or 'in-progress'"
            ))),
        }?;

        if matches!(task.status, TaskStatus::Review | TaskStatus::InProgress) {
            let _ = self.file_lock_store().release_locks_for_task(id);
        }

        self.try_record_friction_transition(&task, task.status, TaskStatus::Rejected);

        Ok(result)
    }

    pub fn archive_task(&self, id: &str) -> Result<(), OrbitError> {
        let task = self.get_task(id)?;

        if task.status == TaskStatus::Archived {
            return Err(OrbitError::InvalidInput(format!(
                "task '{id}' is already archived"
            )));
        }

        self.with_mutation(|| {
            let _ = self.update_task_record(
                id,
                StoreTaskUpdateParams {
                    actor: self.actor_label().to_string(),
                    status: Some(TaskStatus::Archived),
                    ..Default::default()
                },
            )?;
            Ok(((), OrbitEvent::TaskArchived { id: id.to_string() }))
        })?;

        let _ = self.file_lock_store().release_locks_for_task(id);
        Ok(())
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
            let _ = self.update_task_record(
                id,
                StoreTaskUpdateParams {
                    actor: self.actor_label().to_string(),
                    status: Some(TaskStatus::Backlog),
                    ..Default::default()
                },
            )?;
            Ok(((), OrbitEvent::TaskUnarchived { id: id.to_string() }))
        })
    }

    pub fn delete_task(&self, id: &str) -> Result<(), OrbitError> {
        self.with_mutation(|| {
            let deleted = self.delete_task_record(id)?;
            if !deleted {
                return Err(OrbitError::TaskNotFound(id.to_string()));
            }
            Ok(((), OrbitEvent::TaskDeleted { id: id.to_string() }))
        })
    }

    pub fn search_tasks(&self, query: &str) -> Result<Vec<Task>, OrbitError> {
        self.search_task_records(query)
    }

    // ---- Review thread operations ----

    pub fn add_review_thread(
        &self,
        task_id: &str,
        body: String,
        path: Option<String>,
        line: Option<u64>,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<orbit_types::ReviewThread, OrbitError> {
        let actor = self.actor().clone();
        let effective_label =
            effective_actor_label(&actor.label, agent.as_deref(), model.as_deref());

        let now = Utc::now();
        let nanos_suffix = now.timestamp_subsec_nanos() % 10000;
        let thread_id = format!("rt-{}-{:04}", now.format("%Y%m%d-%H%M%S"), nanos_suffix);
        let message_id = format!("rm-{}-{:04}", now.format("%Y%m%d-%H%M%S"), nanos_suffix);

        let thread = orbit_types::ReviewThread {
            thread_id: thread_id.clone(),
            path,
            line,
            status: orbit_types::ReviewThreadStatus::Open,
            messages: vec![orbit_types::ReviewMessage {
                message_id,
                at: now,
                by: effective_label.clone(),
                body,
                github_comment_id: None,
            }],
            github_thread_id: None,
        };

        self.update_task_with_identity(
            task_id,
            TaskUpdateParams {
                append_review_threads: vec![thread.clone()],
                ..Default::default()
            },
            agent,
            model,
        )?;

        Ok(thread)
    }

    pub fn list_review_threads(
        &self,
        task_id: &str,
        status_filter: Option<orbit_types::ReviewThreadStatus>,
    ) -> Result<Vec<orbit_types::ReviewThread>, OrbitError> {
        let task = self.get_task(task_id)?;
        let threads = if let Some(status) = status_filter {
            task.review_threads
                .into_iter()
                .filter(|t| t.status == status)
                .collect()
        } else {
            task.review_threads
        };
        Ok(threads)
    }

    pub fn reply_review_thread(
        &self,
        task_id: &str,
        thread_id: &str,
        body: String,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<orbit_types::ReviewThread, OrbitError> {
        let task = self.get_task(task_id)?;
        let existing = task
            .review_threads
            .iter()
            .find(|t| t.thread_id == thread_id)
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "review thread '{thread_id}' not found on task '{task_id}'"
                ))
            })?;

        let actor = self.actor().clone();
        let effective_label =
            effective_actor_label(&actor.label, agent.as_deref(), model.as_deref());

        let now = Utc::now();
        let nanos_suffix = now.timestamp_subsec_nanos() % 10000;
        let message_id = format!("rm-{}-{:04}", now.format("%Y%m%d-%H%M%S"), nanos_suffix);

        let reply_thread = orbit_types::ReviewThread {
            thread_id: thread_id.to_string(),
            path: None,
            line: None,
            status: existing.status,
            messages: vec![orbit_types::ReviewMessage {
                message_id,
                at: now,
                by: effective_label.clone(),
                body,
                github_comment_id: None,
            }],
            github_thread_id: None,
        };

        self.update_task_with_identity(
            task_id,
            TaskUpdateParams {
                append_review_threads: vec![reply_thread],
                ..Default::default()
            },
            agent,
            model,
        )?;

        // Reload to get the merged thread
        let updated_task = self.get_task(task_id)?;
        updated_task
            .review_threads
            .into_iter()
            .find(|t| t.thread_id == thread_id)
            .ok_or_else(|| {
                OrbitError::Execution("review thread disappeared after reply".to_string())
            })
    }

    pub fn resolve_review_thread(
        &self,
        task_id: &str,
        thread_id: &str,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<orbit_types::ReviewThread, OrbitError> {
        let task = self.get_task(task_id)?;
        let _existing = task
            .review_threads
            .iter()
            .find(|t| t.thread_id == thread_id)
            .ok_or_else(|| {
                OrbitError::InvalidInput(format!(
                    "review thread '{thread_id}' not found on task '{task_id}'"
                ))
            })?;

        let resolve_thread = orbit_types::ReviewThread {
            thread_id: thread_id.to_string(),
            path: None,
            line: None,
            status: orbit_types::ReviewThreadStatus::Resolved,
            messages: vec![],
            github_thread_id: None,
        };

        self.update_task_with_identity(
            task_id,
            TaskUpdateParams {
                append_review_threads: vec![resolve_thread],
                ..Default::default()
            },
            agent,
            model,
        )?;

        let updated_task = self.get_task(task_id)?;
        updated_task
            .review_threads
            .into_iter()
            .find(|t| t.thread_id == thread_id)
            .ok_or_else(|| {
                OrbitError::Execution("review thread disappeared after resolve".to_string())
            })
    }

    /// Best-effort friction bounty scoreboard update after a status transition.
    fn try_record_friction_transition(&self, task: &Task, from: TaskStatus, to: TaskStatus) {
        if !self.scoring_enabled() || !task.task_type.is_friction() {
            return;
        }
        let (Some(agent), Some(model)) = (
            task.actor_identity.agent_name(),
            task.actor_identity.agent_model(),
        ) else {
            return;
        };
        let scoreboard_dir = &self.paths().scoreboard_dir;

        let is_approval = matches!(
            (from, to),
            (TaskStatus::Proposed, TaskStatus::Backlog)
                | (TaskStatus::Proposed, TaskStatus::InProgress)
                | (TaskStatus::Review, TaskStatus::Done)
        );

        if is_approval {
            let _ = friction_bounty::record_friction_accepted(scoreboard_dir, agent, model);
        } else if to == TaskStatus::Rejected {
            let _ = friction_bounty::record_friction_rejected(scoreboard_dir, agent, model);
        }
    }

    #[cfg(test)]
    pub(crate) fn add_task_with_status(
        &self,
        title: &str,
        status: TaskStatus,
    ) -> Result<Task, OrbitError> {
        let actor = if status == TaskStatus::Proposed {
            "agent".to_string()
        } else {
            self.actor_label().to_string()
        };
        let execution_summary = if matches!(
            status,
            TaskStatus::Review | TaskStatus::Done | TaskStatus::Archived
        ) {
            "seeded by runtime test helper".to_string()
        } else {
            String::new()
        };

        self.create_task_record(StoreTaskCreateParams {
            actor: actor.clone(),
            parent_id: None,
            title: title.to_string(),
            description: String::new(),
            acceptance_criteria: Vec::new(),
            plan: String::new(),
            execution_summary,
            context_files: Vec::new(),
            workspace_path: None,
            repo_root: None,
            created_by: Some(actor.clone()),
            actor_identity: ActorIdentity::System,
            assigned_to: Some(actor.clone()),
            status,
            priority: TaskPriority::Medium,
            complexity: None,
            task_type: TaskType::Task,
            pr_number: None,
            proposed_by: (status == TaskStatus::Proposed).then_some(actor),
            source_task_id: None,
            comments: Vec::new(),
        })
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

fn normalize_workspace_path(
    repo_root: &Path,
    workspace: Option<&str>,
) -> Result<Option<String>, OrbitError> {
    let Some(workspace) = workspace.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let canonical_repo_root = repo_root.canonicalize().map_err(|error| {
        OrbitError::InvalidInput(format!(
            "failed to resolve repository root '{}': {error}",
            repo_root.display()
        ))
    })?;
    let candidate = if Path::new(workspace).is_absolute() {
        PathBuf::from(workspace)
    } else {
        canonical_repo_root.join(workspace)
    };
    let canonical_workspace = candidate.canonicalize().map_err(|error| {
        OrbitError::InvalidInput(format!(
            "workspace_path '{}' must reference an existing directory inside the repository: {error}",
            candidate.display()
        ))
    })?;
    if !canonical_workspace.is_dir() {
        return Err(OrbitError::InvalidInput(format!(
            "workspace_path '{}' must reference a directory inside the repository",
            canonical_workspace.display()
        )));
    }

    if !canonical_workspace.starts_with(&canonical_repo_root) {
        return Err(OrbitError::InvalidInput(format!(
            "workspace_path '{}' must stay within repository '{}'",
            canonical_workspace.display(),
            canonical_repo_root.display()
        )));
    }

    Ok(Some(canonical_workspace.to_string_lossy().into_owned()))
}

const UNAUTHORED_TASK_PLAN_PLACEHOLDER: &str = "To be authored by executing agent at start time.";

pub(crate) fn ensure_task_has_execution_plan(id: &str, plan: &str) -> Result<(), OrbitError> {
    let normalized = plan.trim();
    if normalized.is_empty() || normalized == UNAUTHORED_TASK_PLAN_PLACEHOLDER {
        return Err(OrbitError::InvalidInput(format!(
            "task '{id}' requires a non-empty execution plan before transitioning to in-progress"
        )));
    }
    Ok(())
}

fn effective_actor_label(default_label: &str, agent: Option<&str>, model: Option<&str>) -> String {
    match (agent, model) {
        (Some(agent), Some(model)) => format!("{agent} / {model}"),
        (Some(agent), None) => agent.to_string(),
        (None, Some(model)) => model.to_string(),
        (None, None) => default_label.to_string(),
    }
}

fn should_release_file_locks(old_status: TaskStatus, new_status: TaskStatus) -> bool {
    matches!(old_status, TaskStatus::InProgress | TaskStatus::Review)
        && !matches!(new_status, TaskStatus::InProgress | TaskStatus::Review)
}

#[cfg(test)]
mod tests {
    use super::{
        TaskAddParams, TaskUpdateParams, UNAUTHORED_TASK_PLAN_PLACEHOLDER,
        ensure_task_has_execution_plan,
    };
    use crate::{OrbitError, OrbitRuntime, TaskStatus, TaskType};
    use orbit_engine::{TaskAutomationUpdate, TaskHost};
    use std::fs;
    use std::path::Path;

    fn canonical_string(path: &Path) -> String {
        path.canonicalize()
            .expect("canonical path")
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn blank_or_placeholder_plan_is_rejected() {
        let blank = ensure_task_has_execution_plan("T1", "");
        assert!(matches!(blank, Err(OrbitError::InvalidInput(_))));

        let placeholder = ensure_task_has_execution_plan("T1", UNAUTHORED_TASK_PLAN_PLACEHOLDER);
        assert!(matches!(placeholder, Err(OrbitError::InvalidInput(_))));
    }

    #[test]
    fn starting_task_requires_plan() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task_with_status("needs plan", TaskStatus::Backlog)
            .expect("task");

        let err = runtime
            .start_task(&task.id, None, None)
            .expect_err("start should fail");
        assert!(matches!(err, OrbitError::InvalidInput(_)));
        assert!(
            err.to_string()
                .contains("requires a non-empty execution plan")
        );
    }

    #[test]
    fn transition_to_in_progress_allows_plan_in_same_update() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task_with_status("needs plan", TaskStatus::Backlog)
            .expect("task");

        let updated = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    plan: Some("## Plan\n- Ship it".to_string()),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .expect("update succeeds");

        assert_eq!(updated.status, TaskStatus::InProgress);
        assert_eq!(updated.plan, "## Plan\n- Ship it");
    }

    #[test]
    fn automation_transition_to_in_progress_requires_plan() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task_with_status("needs plan", TaskStatus::Backlog)
            .expect("task");

        let err = <OrbitRuntime as TaskHost>::apply_task_automation_update(
            &runtime,
            &task.id,
            TaskAutomationUpdate {
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .expect_err("automation update should fail");

        assert!(matches!(err, OrbitError::InvalidInput(_)));
    }

    #[test]
    fn acceptance_criteria_round_trip_through_tasks() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "criteria".to_string(),
                description: "desc".to_string(),
                acceptance_criteria: vec![
                    "first outcome".to_string(),
                    "second outcome".to_string(),
                ],
                ..Default::default()
            })
            .expect("task");

        assert_eq!(
            task.acceptance_criteria,
            vec!["first outcome".to_string(), "second outcome".to_string()]
        );

        let loaded = runtime.get_task(&task.id).expect("load");
        assert_eq!(loaded.acceptance_criteria, task.acceptance_criteria);
    }

    #[test]
    fn update_task_updates_acceptance_criteria() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "criteria".to_string(),
                description: "desc".to_string(),
                acceptance_criteria: vec!["first outcome".to_string()],
                ..Default::default()
            })
            .expect("task");

        let updated = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    acceptance_criteria: Some(vec![
                        "updated outcome".to_string(),
                        "another outcome".to_string(),
                    ]),
                    ..Default::default()
                },
            )
            .expect("update succeeds");

        assert_eq!(
            updated.acceptance_criteria,
            vec!["updated outcome".to_string(), "another outcome".to_string()]
        );
    }

    #[test]
    fn done_tasks_allow_metadata_updates() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        for status in [TaskStatus::Done, TaskStatus::Archived] {
            let task = runtime
                .add_task_with_status("completed", status)
                .expect("task");

            let updated = runtime
                .update_task(
                    &task.id,
                    TaskUpdateParams {
                        title: Some("completed v2".to_string()),
                        description: Some("clarified description".to_string()),
                        acceptance_criteria: Some(vec!["documented".to_string()]),
                        ..Default::default()
                    },
                )
                .expect("metadata update succeeds");

            assert_eq!(updated.title, "completed v2");
            assert_eq!(updated.description, "clarified description");
            assert_eq!(updated.acceptance_criteria, vec!["documented".to_string()]);
        }
    }

    #[test]
    fn add_task_rejects_absolute_ancestor_workspace_path() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let repo_root = runtime.paths().repo_root.clone();
        let filesystem_root = repo_root
            .ancestors()
            .last()
            .expect("filesystem root")
            .to_path_buf();

        let err = runtime
            .add_task(TaskAddParams {
                title: "bad workspace".to_string(),
                description: "desc".to_string(),
                workspace_path: Some(filesystem_root.to_string_lossy().into_owned()),
                ..Default::default()
            })
            .expect_err("workspace should be rejected");

        assert!(matches!(err, OrbitError::InvalidInput(_)));
        assert!(err.to_string().contains("must stay within repository"));
    }

    #[test]
    fn add_task_normalizes_workspace_path_to_canonical_repo_child() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let repo_root = runtime.paths().repo_root.clone();
        let nested = repo_root.join("nested");
        fs::create_dir_all(&nested).expect("nested dir");

        let task = runtime
            .add_task(TaskAddParams {
                title: "normalized workspace".to_string(),
                description: "desc".to_string(),
                workspace_path: Some("nested/.".to_string()),
                ..Default::default()
            })
            .expect("task");

        assert_eq!(task.workspace_path, Some(canonical_string(&nested)));
    }

    #[test]
    fn add_task_allows_missing_workspace_path() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");

        let task = runtime
            .add_task(TaskAddParams {
                title: "no workspace".to_string(),
                description: "desc".to_string(),
                ..Default::default()
            })
            .expect("task");

        assert_eq!(task.workspace_path, None);
    }

    #[test]
    fn reject_from_backlog_succeeds() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task_with_status("backlog task", TaskStatus::Backlog)
            .expect("task");

        let rejected = runtime
            .reject_task(&task.id, "duplicate".to_string(), None)
            .expect("reject from backlog should succeed");

        assert_eq!(rejected.status, TaskStatus::Rejected);
    }

    #[test]
    fn reject_from_in_progress_succeeds() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task_with_status("wip task", TaskStatus::InProgress)
            .expect("task");

        let rejected = runtime
            .reject_task(&task.id, "no longer needed".to_string(), None)
            .expect("reject from in_progress should succeed");

        assert_eq!(rejected.status, TaskStatus::Rejected);
    }

    #[test]
    fn reject_from_done_is_disallowed() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task_with_status("done task", TaskStatus::Done)
            .expect("task");

        let err = runtime
            .reject_task(&task.id, "oops".to_string(), None)
            .expect_err("reject from done should fail");

        assert!(matches!(err, OrbitError::InvalidInput(_)));
    }

    #[test]
    fn friction_task_attributes_to_system() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task_with_identity(
                TaskAddParams {
                    title: "friction report".to_string(),
                    description: "something is broken".to_string(),
                    task_type: TaskType::Friction,
                    ..Default::default()
                },
                Some("codex".to_string()),
                Some("gpt-5.4".to_string()),
            )
            .expect("friction task");

        assert_eq!(task.created_by, Some("system".to_string()));
        assert_eq!(task.proposed_by, Some("system".to_string()));
        assert_eq!(task.assigned_to, Some("system".to_string()));
        assert_eq!(
            task.history.first().map(|h| h.by.as_str()),
            Some("system"),
            "history actor should be system for friction tasks"
        );
    }

    #[test]
    fn issue_task_attributes_to_system() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task_with_identity(
                TaskAddParams {
                    title: "issue report".to_string(),
                    description: "also friction".to_string(),
                    task_type: TaskType::Issue,
                    ..Default::default()
                },
                Some("claude".to_string()),
                Some("opus".to_string()),
            )
            .expect("issue task");

        assert_eq!(task.created_by, Some("system".to_string()));
        assert_eq!(task.proposed_by, Some("system".to_string()));
        assert_eq!(task.assigned_to, Some("system".to_string()));
    }

    #[test]
    fn non_friction_task_attributes_to_agent() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task_with_identity(
                TaskAddParams {
                    title: "normal task".to_string(),
                    description: "not friction".to_string(),
                    task_type: TaskType::Feature,
                    ..Default::default()
                },
                Some("codex".to_string()),
                Some("gpt-5.4".to_string()),
            )
            .expect("non-friction task");

        // Non-friction tasks should use the agent identity
        assert_ne!(task.created_by, Some("system".to_string()));
        assert_ne!(task.proposed_by, Some("system".to_string()));
        assert_ne!(task.assigned_to, Some("system".to_string()));
        assert_eq!(
            task.created_by,
            Some("codex / gpt-5.4".to_string()),
            "non-friction tasks should use agent label"
        );
    }

    #[test]
    fn delete_task_removes_it() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task_with_status("to delete", TaskStatus::Proposed)
            .expect("task");

        runtime
            .delete_task(&task.id)
            .expect("delete should succeed");

        let err = runtime.get_task(&task.id).expect_err("task should be gone");
        assert!(matches!(err, OrbitError::TaskNotFound(_)));
    }
}
