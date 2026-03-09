use orbit_types::{Audit, OrbitError, OrbitEvent};
use rusqlite::params;
use serde_json::Value;

use crate::{Store, StoreTx, now_string, parse_timestamp};

impl Store {
    pub fn list_audits(&self, limit: usize) -> Result<Vec<Audit>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, event_type, payload, message, created_at FROM audits ORDER BY id DESC LIMIT ?1",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        let rows = stmt
            .query_map([limit as i64], |row| {
                let payload_raw: String = row.get(2)?;
                let created_at_raw: String = row.get(4)?;

                let payload: Value = serde_json::from_str(&payload_raw).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        payload_raw.len(),
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;

                Ok(Audit {
                    id: row.get(0)?,
                    event_type: row.get(1)?,
                    payload,
                    message: row.get(3)?,
                    created_at: parse_timestamp(&created_at_raw)?,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }
}

impl<'a> StoreTx<'a> {
    pub fn insert_audit_event(&mut self, event: &OrbitEvent) -> Result<(), OrbitError> {
        let payload = serde_json::to_string(event).map_err(|e| OrbitError::Store(e.to_string()))?;
        let event_type = event_type(event);
        let message = event_message(event);
        self.tx
            .execute(
                "INSERT INTO audits(event_type, payload, message, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![event_type, payload, message, now_string()],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        Ok(())
    }
}

fn event_type(event: &OrbitEvent) -> &'static str {
    match event {
        OrbitEvent::JobAdded { .. } => "JobAdded",
        OrbitEvent::JobPaused { .. } => "JobPaused",
        OrbitEvent::JobResumed { .. } => "JobResumed",
        OrbitEvent::JobDeleted { .. } => "JobDeleted",
        OrbitEvent::JobTriggered { .. } => "JobTriggered",
        OrbitEvent::JobRunStarted { .. } => "JobRunStarted",
        OrbitEvent::JobRunCompleted { .. } => "JobRunCompleted",
        OrbitEvent::JobRetryScheduled { .. } => "JobRetryScheduled",
        OrbitEvent::JobProtocolViolation { .. } => "JobProtocolViolation",
        OrbitEvent::JobSkipped { .. } => "JobSkipped",
        OrbitEvent::JobRunArchived { .. } => "JobRunArchived",
        OrbitEvent::ToolExecuted { .. } => "ToolExecuted",
        OrbitEvent::PolicyDenied { .. } => "PolicyDenied",
        OrbitEvent::TaskAdded { .. } => "TaskAdded",
        OrbitEvent::TaskUpdated { .. } => "TaskUpdated",
        OrbitEvent::TaskArchived { .. } => "TaskArchived",
        OrbitEvent::TaskUnarchived { .. } => "TaskUnarchived",
        OrbitEvent::TaskProposalApproved { .. } => "TaskProposalApproved",
        OrbitEvent::TaskProposalRejected { .. } => "TaskProposalRejected",
        OrbitEvent::TaskReviewApproved { .. } => "TaskReviewApproved",
        OrbitEvent::TaskReviewRejected { .. } => "TaskReviewRejected",
        OrbitEvent::TaskDeleted { .. } => "TaskDeleted",
        OrbitEvent::ToolAdded { .. } => "ToolAdded",
        OrbitEvent::ToolRemoved { .. } => "ToolRemoved",
        OrbitEvent::ToolEnabled { .. } => "ToolEnabled",
        OrbitEvent::ToolDisabled { .. } => "ToolDisabled",
        OrbitEvent::SkillAdded { .. } => "SkillAdded",
        OrbitEvent::SkillUpdated { .. } => "SkillUpdated",
        OrbitEvent::SkillDeleted { .. } => "SkillDeleted",
        OrbitEvent::SkillAttached { .. } => "SkillAttached",
        OrbitEvent::SkillDetached { .. } => "SkillDetached",
        OrbitEvent::ActivityAdded { .. } => "ActivityAdded",
        OrbitEvent::ActivityDisabled { .. } => "ActivityDisabled",
        OrbitEvent::ActivityRunStarted { .. } => "ActivityRunStarted",
        OrbitEvent::ActivityRunCompleted { .. } => "ActivityRunCompleted",
        OrbitEvent::ActivityProtocolViolation { .. } => "ActivityProtocolViolation",
        OrbitEvent::AgentSessionStarted { .. } => "AgentSessionStarted",
        OrbitEvent::AgentToolCall { .. } => "AgentToolCall",
        OrbitEvent::AgentSessionCompleted { .. } => "AgentSessionCompleted",
    }
}

fn event_message(event: &OrbitEvent) -> String {
    match event {
        OrbitEvent::JobAdded { job_id } => format!("job added: {job_id}"),
        OrbitEvent::JobPaused { job_id } => format!("job paused: {job_id}"),
        OrbitEvent::JobResumed { job_id } => {
            format!("job resumed: {job_id}")
        }
        OrbitEvent::JobDeleted { job_id } => {
            format!("job deleted: {job_id}")
        }
        OrbitEvent::JobTriggered { job_id } => {
            format!("job triggered: {job_id}")
        }
        OrbitEvent::JobRunStarted {
            job_id,
            run_id,
            attempt,
        } => format!("job run started: job={job_id} run={run_id} attempt={attempt}"),
        OrbitEvent::JobRunCompleted {
            job_id,
            run_id,
            state,
        } => {
            format!("job run completed: job={job_id} run={run_id} state={state}")
        }
        OrbitEvent::JobRetryScheduled {
            job_id,
            run_id,
            next_run_at,
        } => format!("job retry scheduled: job={job_id} run={run_id} next_run_at={next_run_at}"),
        OrbitEvent::JobProtocolViolation {
            job_id,
            run_id,
            message,
        } => format!("job protocol violation: job={job_id} run={run_id} message={message}"),
        OrbitEvent::JobSkipped { job_id, reason } => {
            format!("job skipped: job={job_id} reason={reason}")
        }
        OrbitEvent::JobRunArchived { job_id, run_id } => {
            format!("job run archived: job={job_id} run={run_id}")
        }
        OrbitEvent::ToolExecuted { name } => format!("tool executed: {name}"),
        OrbitEvent::PolicyDenied { tool } => format!("policy denied: {tool}"),
        OrbitEvent::TaskAdded { id } => format!("task added: {id}"),
        OrbitEvent::TaskUpdated { id } => format!("task updated: {id}"),
        OrbitEvent::TaskArchived { id } => format!("task archived: {id}"),
        OrbitEvent::TaskUnarchived { id } => format!("task unarchived: {id}"),
        OrbitEvent::TaskProposalApproved { id, approved_by } => {
            format!("task proposal approved: {id} by {approved_by}")
        }
        OrbitEvent::TaskProposalRejected { id, rejected_by } => {
            format!("task proposal rejected: {id} by {rejected_by}")
        }
        OrbitEvent::TaskReviewApproved { id, approved_by } => {
            format!("task review approved: {id} by {approved_by}")
        }
        OrbitEvent::TaskReviewRejected { id, rejected_by } => {
            format!("task review rejected: {id} by {rejected_by}")
        }
        OrbitEvent::TaskDeleted { id } => format!("task deleted: {id}"),
        OrbitEvent::ToolAdded { name } => format!("tool added: {name}"),
        OrbitEvent::ToolRemoved { name } => format!("tool removed: {name}"),
        OrbitEvent::ToolEnabled { name } => format!("tool enabled: {name}"),
        OrbitEvent::ToolDisabled { name } => format!("tool disabled: {name}"),
        OrbitEvent::SkillAdded { name } => format!("skill added: {name}"),
        OrbitEvent::SkillUpdated { name } => format!("skill updated: {name}"),
        OrbitEvent::SkillDeleted { name } => format!("skill deleted: {name}"),
        OrbitEvent::SkillAttached {
            task_id,
            skill_name,
        } => format!("skill attached: {skill_name} -> {task_id}"),
        OrbitEvent::SkillDetached {
            task_id,
            skill_name,
        } => format!("skill detached: {skill_name} -> {task_id}"),
        OrbitEvent::ActivityAdded { id } => format!("activity added: {id}"),
        OrbitEvent::ActivityDisabled { id } => format!("activity disabled: {id}"),
        OrbitEvent::ActivityRunStarted { id } => format!("activity run started: {id}"),
        OrbitEvent::ActivityRunCompleted { id, state } => {
            format!("activity run completed: {id} state={state}")
        }
        OrbitEvent::ActivityProtocolViolation { id, message } => {
            format!("activity protocol violation: {id} message={message}")
        }
        OrbitEvent::AgentSessionStarted {
            session_id,
            task_id,
            ..
        } => {
            format!("agent session started: {session_id} task={task_id}")
        }
        OrbitEvent::AgentToolCall {
            session_id,
            tool_name,
            success,
            ..
        } => format!("agent tool call: session={session_id} tool={tool_name} success={success}"),
        OrbitEvent::AgentSessionCompleted {
            session_id,
            task_id,
            status,
        } => format!("agent session completed: {session_id} task={task_id} status={status}"),
    }
}
