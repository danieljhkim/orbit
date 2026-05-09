use orbit_common::types::{
    OrbitError, OrbitEvent, Task, TaskHistoryEntry, TaskStatus, build_task_status_index,
    normalize_optional_attribution_label, unmet_task_dependencies,
};
use orbit_store::friction_bounty;

use crate::OrbitRuntime;
use crate::runtime::TaskRecordUpdateParams as StoreTaskUpdateParams;

use super::helpers::{
    SYSTEM_ACTOR_LABEL, build_task_comments, effective_actor_label, implementation_label,
};

const UNAUTHORED_TASK_PLAN_PLACEHOLDER: &str = "To be authored by executing agent at start time.";

impl OrbitRuntime {
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
        let (canonical_agent, canonical_model) =
            self.try_canonical_agent_model_identity(agent.as_deref(), model.as_deref())?;
        let task = self.get_task(id)?;
        let actor = self.actor().clone();
        let effective_label = effective_actor_label(
            &actor.label,
            canonical_agent.as_deref(),
            canonical_model.as_deref(),
        );
        let implemented_by =
            implementation_label(&task, effective_label.as_str(), canonical_model.as_deref());
        let append_comments = build_task_comments(comment, effective_label.as_str())?;

        let result = match task.status {
            TaskStatus::Proposed | TaskStatus::Friction => self.with_mutation(|| {
                let status_event = if task.status == TaskStatus::Friction {
                    "friction_accepted"
                } else {
                    "proposal_approved"
                };
                let task = self.stores().tasks().update(
                    id,
                    StoreTaskUpdateParams {
                        actor: effective_label.clone(),
                        status: Some(TaskStatus::Backlog),
                        status_event: Some(status_event.to_string()),
                        status_note: note.clone(),
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
                let task = self.stores().tasks().update(
                    id,
                    StoreTaskUpdateParams {
                        actor: effective_label.clone(),
                        status: Some(TaskStatus::Done),
                        status_event: Some("review_approved".to_string()),
                        status_note: note.clone(),
                        implemented_by: implemented_by.clone().map(Some),
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
                "task '{id}' is in status '{other}'; approve requires 'proposed', 'friction', or 'review'"
            ))),
        }?;

        let approved_to = if matches!(task.status, TaskStatus::Proposed | TaskStatus::Friction) {
            TaskStatus::Backlog
        } else {
            TaskStatus::Done
        };
        self.try_record_friction_transition(&task, task.status, approved_to);

        Ok(result)
    }

    pub fn start_task(
        &self,
        id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.start_task_with_actor_label_override(id, note, comment, None, None, None)
    }

    pub fn start_task_with_identity(
        &self,
        id: &str,
        note: Option<String>,
        comment: Option<String>,
        agent: Option<String>,
        model: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.start_task_with_actor_label_override(id, note, comment, agent, model, None)
    }

    pub(crate) fn start_task_as_system(
        &self,
        id: &str,
        note: Option<String>,
        comment: Option<String>,
    ) -> Result<Task, OrbitError> {
        self.start_task_with_actor_label_override(
            id,
            note,
            comment,
            None,
            None,
            Some(SYSTEM_ACTOR_LABEL.to_string()),
        )
    }

    fn start_task_with_actor_label_override(
        &self,
        id: &str,
        note: Option<String>,
        comment: Option<String>,
        agent: Option<String>,
        model: Option<String>,
        actor_label_override: Option<String>,
    ) -> Result<Task, OrbitError> {
        let (canonical_agent, canonical_model) =
            self.try_canonical_agent_model_identity(agent.as_deref(), model.as_deref())?;
        let task = self.get_task(id)?;
        let dependency_status_index = build_task_status_index(&self.list_tasks()?);
        let unmet_dependencies = unmet_task_dependencies(&task, &dependency_status_index);
        if in_progress_transition_requires_plan(task.status) {
            ensure_task_has_execution_plan(id, task.plan.as_str())?;
        }
        let actor = self.actor().clone();
        let effective_label = actor_label_override.unwrap_or_else(|| {
            effective_actor_label(
                &actor.label,
                canonical_agent.as_deref(),
                canonical_model.as_deref(),
            )
        });
        let append_comments = build_task_comments(comment, effective_label.as_str())?;
        let unmet_dependency_labels: Vec<String> = unmet_dependencies
            .iter()
            .map(|dependency| dependency.label())
            .collect();
        let warn_unmet_dependencies = || {
            if !unmet_dependency_labels.is_empty() {
                orbit_common::tracing::warn!(
                    target: "orbit.task.dependencies",
                    task_id = id,
                    unmet = unmet_dependency_labels.join(",").as_str(),
                    "task has unmet dependencies",
                );
            }
        };

        match task.status {
            TaskStatus::Proposed | TaskStatus::Friction => {
                warn_unmet_dependencies();
                let result = self.with_mutation(|| {
                    let at = chrono::Utc::now();
                    let acceptance_event = if task.status == TaskStatus::Friction {
                        "friction_accepted"
                    } else {
                        "proposal_approved"
                    };
                    let task = self.stores().tasks().update(
                        id,
                        StoreTaskUpdateParams {
                            actor: effective_label.clone(),
                            status: Some(TaskStatus::InProgress),
                            status_event: Some("started".to_string()),
                            agent: canonical_agent.clone().map(Some),
                            model: canonical_model.clone().map(Some),
                            append_history: vec![TaskHistoryEntry {
                                at,
                                by: effective_label.clone(),
                                event: acceptance_event.to_string(),
                                note: note.clone(),
                                from_status: Some(task.status),
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
                            approved_from_proposed: task.status == TaskStatus::Proposed,
                        },
                    ))
                })?;
                self.try_record_friction_transition(&task, task.status, TaskStatus::InProgress);
                Ok(result)
            }
            TaskStatus::Backlog | TaskStatus::Someday | TaskStatus::Blocked => {
                warn_unmet_dependencies();
                let task = self.with_mutation(|| {
                    let task = self.stores().tasks().update(
                        id,
                        StoreTaskUpdateParams {
                            actor: effective_label.clone(),
                            status: Some(TaskStatus::InProgress),
                            status_event: Some("started".to_string()),
                            status_note: note.clone(),
                            agent: canonical_agent.clone().map(Some),
                            model: canonical_model.clone().map(Some),
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
                "task '{id}' is in status '{other}'; start requires 'proposed', 'friction', 'backlog', 'someday', or 'blocked'"
            ))),
        }
    }

    pub(crate) fn admit_task_for_workflow_as_system(
        &self,
        id: &str,
        workflow: &str,
    ) -> Result<Task, OrbitError> {
        let task = self.get_task(id)?;
        let workflow = workflow.trim();
        let workflow = if workflow.is_empty() {
            "workflow"
        } else {
            workflow
        };

        if task.status == TaskStatus::InProgress {
            return Ok(task);
        }

        if !matches!(
            task.status,
            TaskStatus::Proposed
                | TaskStatus::Friction
                | TaskStatus::Backlog
                | TaskStatus::Rejected
                | TaskStatus::Archived
        ) {
            return Err(OrbitError::InvalidInput(format!(
                "task '{id}' is in status '{}'; workflow admission for '{workflow}' requires 'proposed', 'friction', 'backlog', 'rejected', 'archived', or 'in-progress'",
                task.status
            )));
        }

        let note = Some(format!("workflow admission: {workflow}"));
        let append_history = if matches!(task.status, TaskStatus::Proposed | TaskStatus::Friction) {
            let acceptance_event = if task.status == TaskStatus::Friction {
                "friction_accepted"
            } else {
                "proposal_approved"
            };
            vec![TaskHistoryEntry {
                at: chrono::Utc::now(),
                by: SYSTEM_ACTOR_LABEL.to_string(),
                event: acceptance_event.to_string(),
                note: note.clone(),
                from_status: Some(task.status),
                to_status: Some(TaskStatus::Backlog),
            }]
        } else {
            Vec::new()
        };

        let approved_from_proposed = task.status == TaskStatus::Proposed;
        let updated = self.with_mutation(|| {
            let task = self.stores().tasks().update(
                id,
                StoreTaskUpdateParams {
                    actor: SYSTEM_ACTOR_LABEL.to_string(),
                    status: Some(TaskStatus::InProgress),
                    status_event: Some("started".to_string()),
                    status_note: note.clone(),
                    append_history: append_history.clone(),
                    ..Default::default()
                },
            )?;
            Ok((
                task.clone(),
                OrbitEvent::TaskStarted {
                    id: id.to_string(),
                    started_by: SYSTEM_ACTOR_LABEL.to_string(),
                    approved_from_proposed,
                },
            ))
        })?;

        self.try_record_friction_transition(&task, task.status, TaskStatus::InProgress);

        Ok(updated)
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
        let (canonical_agent, canonical_model) =
            self.try_canonical_agent_model_identity(agent.as_deref(), model.as_deref())?;
        let task = self.get_task(id)?;
        let actor = self.actor().clone();
        let effective_label = effective_actor_label(
            &actor.label,
            canonical_agent.as_deref(),
            canonical_model.as_deref(),
        );
        let reason = note.trim();
        if reason.is_empty() {
            return Err(OrbitError::InvalidInput(
                "rejection note must not be empty".to_string(),
            ));
        }
        let reason = reason.to_string();
        let append_comments = build_task_comments(comment, effective_label.as_str())?;

        let result = match task.status {
            TaskStatus::Proposed | TaskStatus::Friction => self.with_mutation(|| {
                let status_event = if task.status == TaskStatus::Friction {
                    "friction_rejected"
                } else {
                    "proposal_rejected"
                };
                let task = self.stores().tasks().update(
                    id,
                    StoreTaskUpdateParams {
                        actor: effective_label.clone(),
                        status: Some(TaskStatus::Rejected),
                        status_event: Some(status_event.to_string()),
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
                let task = self.stores().tasks().update(
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
                let task = self.stores().tasks().update(
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
                let task = self.stores().tasks().update(
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
                "task '{id}' is in status '{other}'; reject requires 'proposed', 'friction', 'review', 'backlog', or 'in-progress'"
            ))),
        }?;

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
            let _ = self.stores().tasks().update(
                id,
                StoreTaskUpdateParams {
                    actor: self.actor_label().to_string(),
                    status: Some(TaskStatus::Archived),
                    ..Default::default()
                },
            )?;
            Ok(((), OrbitEvent::TaskArchived { id: id.to_string() }))
        })?;

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
            let _ = self.stores().tasks().update(
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
            let deleted = self.stores().tasks().delete(id)?;
            if !deleted {
                return Err(OrbitError::TaskNotFound(id.to_string()));
            }
            Ok(((), OrbitEvent::TaskDeleted { id: id.to_string() }))
        })
    }

    pub fn delete_task_guarded(&self, id: &str, force: bool) -> Result<(), OrbitError> {
        let task = self.get_task(id)?;
        ensure_task_delete_allowed(&task.id, task.status, force)?;
        self.delete_task(id)
    }
}

fn ensure_task_delete_allowed(id: &str, status: TaskStatus, force: bool) -> Result<(), OrbitError> {
    if force
        || matches!(
            status,
            TaskStatus::Proposed | TaskStatus::Friction | TaskStatus::Rejected
        )
    {
        return Ok(());
    }

    Err(OrbitError::InvalidInput(format!(
        "task '{id}' is in status '{status}'; use --force to delete tasks not in proposed, friction, or rejected status"
    )))
}

pub(crate) fn ensure_task_has_execution_plan(id: &str, plan: &str) -> Result<(), OrbitError> {
    let normalized = plan.trim();
    if normalized.is_empty() || normalized == UNAUTHORED_TASK_PLAN_PLACEHOLDER {
        return Err(OrbitError::InvalidInput(format!(
            "task '{id}' requires a non-empty execution plan before transitioning to in-progress"
        )));
    }
    Ok(())
}

pub(crate) fn in_progress_transition_requires_plan(from_status: TaskStatus) -> bool {
    !matches!(from_status, TaskStatus::Backlog | TaskStatus::InProgress)
}

impl OrbitRuntime {
    /// Best-effort friction bounty scoreboard update after a status transition.
    pub(crate) fn try_record_friction_transition(
        &self,
        task: &Task,
        from: TaskStatus,
        to: TaskStatus,
    ) {
        if !self.scoring_enabled() || !task.task_type.counts_toward_friction_bounty() {
            return;
        }
        let Some(model) =
            normalize_optional_attribution_label(task.created_by.as_deref(), task.model.as_deref())
        else {
            return;
        };
        let scoreboard_dir = &self.paths().scoreboard_dir;

        let is_approval = matches!(
            (from, to),
            (TaskStatus::Friction, TaskStatus::Backlog)
                | (TaskStatus::Friction, TaskStatus::InProgress)
                | (TaskStatus::Friction, TaskStatus::Done)
        );

        if is_approval {
            let _ = friction_bounty::record_friction_accepted(scoreboard_dir, &model);
        } else if from == TaskStatus::Friction && to == TaskStatus::Rejected {
            let _ = friction_bounty::record_friction_rejected(scoreboard_dir, &model);
        }
    }
}
