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
        OrbitEvent::ToolExecuted { .. } => "ToolExecuted",
        OrbitEvent::JobStarted { .. } => "JobStarted",
        OrbitEvent::JobCompleted { .. } => "JobCompleted",
        OrbitEvent::WatchTriggered { .. } => "WatchTriggered",
        OrbitEvent::PolicyDenied { .. } => "PolicyDenied",
        OrbitEvent::TaskAdded { .. } => "TaskAdded",
        OrbitEvent::TaskUpdated { .. } => "TaskUpdated",
        OrbitEvent::TaskClosed { .. } => "TaskClosed",
        OrbitEvent::TaskReopened { .. } => "TaskReopened",
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
        OrbitEvent::AgentSessionStarted { .. } => "AgentSessionStarted",
        OrbitEvent::AgentToolCall { .. } => "AgentToolCall",
        OrbitEvent::AgentSessionCompleted { .. } => "AgentSessionCompleted",
        OrbitEvent::EntryCreated { .. } => "EntryCreated",
    }
}

fn event_message(event: &OrbitEvent) -> String {
    match event {
        OrbitEvent::ToolExecuted { name } => format!("tool executed: {name}"),
        OrbitEvent::JobStarted { id } => format!("job started: {id}"),
        OrbitEvent::JobCompleted { id, success } => {
            format!("job completed: {id} (success={success})")
        }
        OrbitEvent::WatchTriggered { path } => format!("watch triggered: {path}"),
        OrbitEvent::PolicyDenied { tool } => format!("policy denied: {tool}"),
        OrbitEvent::TaskAdded { id } => format!("task added: {id}"),
        OrbitEvent::TaskUpdated { id } => format!("task updated: {id}"),
        OrbitEvent::TaskClosed { id } => format!("task closed: {id}"),
        OrbitEvent::TaskReopened { id } => format!("task reopened: {id}"),
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
        OrbitEvent::EntryCreated {
            id,
            entity_type,
            entity_id,
            sequence_number,
        } => {
            format!(
                "entry created: {id} entity={entity_type}:{entity_id} sequence={sequence_number}"
            )
        }
    }
}
