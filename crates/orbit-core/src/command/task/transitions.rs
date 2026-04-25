use orbit_common::types::{
    OrbitError, OrbitEvent, Task, TaskHistoryEntry, TaskStatus, build_task_status_index,
    normalize_optional_attribution_label, unmet_task_dependencies,
};
use orbit_store::friction_bounty;

use crate::OrbitRuntime;
use crate::runtime::TaskRecordUpdateParams as StoreTaskUpdateParams;

use super::helpers::{build_task_comments, effective_actor_label, implementation_label};

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
        let task = self.get_task(id)?;
        let actor = self.actor().clone();
        let effective_label =
            effective_actor_label(&actor.label, agent.as_deref(), model.as_deref());
        let implemented_by =
            implementation_label(&task, effective_label.as_str(), model.as_deref());
        let append_comments = build_task_comments(comment, effective_label.as_str())?;

        let result = match task.status {
            TaskStatus::Proposed => self.with_mutation(|| {
                let task = self.stores().tasks().update(
                    id,
                    StoreTaskUpdateParams {
                        actor: effective_label.clone(),
                        status: Some(TaskStatus::Backlog),
                        status_event: Some("proposal_approved".to_string()),
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
                "task '{id}' is in status '{other}'; approve requires 'proposed' or 'review'"
            ))),
        }?;

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
        let (canonical_agent, canonical_model) =
            self.canonical_agent_model_identity(agent.as_deref(), model.as_deref());
        let task = self.get_task(id)?;
        let dependency_status_index = build_task_status_index(&self.list_tasks()?);
        let unmet_dependencies = unmet_task_dependencies(&task, &dependency_status_index);
        if in_progress_transition_requires_plan(task.status) {
            ensure_task_has_execution_plan(id, task.plan.as_str())?;
        }
        let actor = self.actor().clone();
        let effective_label = effective_actor_label(
            &actor.label,
            canonical_agent.as_deref(),
            canonical_model.as_deref(),
        );
        let append_comments = build_task_comments(comment, effective_label.as_str())?;
        let dependency_warning = (!unmet_dependencies.is_empty()).then(|| {
            format!(
                "warning: task '{id}' has unmet dependencies: {}",
                unmet_dependencies
                    .iter()
                    .map(|dependency| dependency.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });

        match task.status {
            TaskStatus::Proposed => {
                if let Some(warning) = &dependency_warning {
                    eprintln!("{warning}");
                }
                let result = self.with_mutation(|| {
                    let at = chrono::Utc::now();
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
                if let Some(warning) = &dependency_warning {
                    eprintln!("{warning}");
                }
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
                let task = self.stores().tasks().update(
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
                "task '{id}' is in status '{other}'; reject requires 'proposed', 'review', 'backlog', or 'in-progress'"
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
            (TaskStatus::Proposed, TaskStatus::Backlog)
                | (TaskStatus::Proposed, TaskStatus::InProgress)
                | (TaskStatus::Review, TaskStatus::Done)
        );

        if is_approval {
            let _ = friction_bounty::record_friction_accepted(scoreboard_dir, &model);
        } else if to == TaskStatus::Rejected {
            let _ = friction_bounty::record_friction_rejected(scoreboard_dir, &model);
        }
    }
}
