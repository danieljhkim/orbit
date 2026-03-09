use chrono::Utc;
use orbit_types::{AgentSession, AgentSessionStatus, AgentToolCall, IdentityRole, OrbitError};
use rusqlite::{OptionalExtension, params};

use crate::{Store, StoreTx};

fn parse_agent_status(raw: &str) -> AgentSessionStatus {
    match raw {
        "running" => AgentSessionStatus::Running,
        "completed" => AgentSessionStatus::Completed,
        "failed" => AgentSessionStatus::Failed,
        _ => AgentSessionStatus::Failed,
    }
}

fn status_to_str(status: &AgentSessionStatus) -> &'static str {
    match status {
        AgentSessionStatus::Running => "running",
        AgentSessionStatus::Completed => "completed",
        AgentSessionStatus::Failed => "failed",
    }
}

impl Store {
    pub fn get_agent_session(&self, session_id: &str) -> Result<Option<AgentSession>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        conn.query_row(
            "SELECT session_id, task_id, identity_id, identity_name, identity_role, identity_block, skill_names, composed_context_hash, effective_allowed_tools, tool_calls, outcome, status, created_at, updated_at
             FROM agent_sessions WHERE session_id = ?1",
            [session_id],
            |row| {
                let identity_role_raw: Option<String> = row.get(4)?;
                let skill_names_raw: String = row.get(6)?;
                let effective_allowed_tools_raw: String = row.get(8)?;
                let tool_calls_raw: String = row.get(9)?;
                let status_raw: String = row.get(11)?;
                let created_at_raw: String = row.get(12)?;
                let updated_at_raw: String = row.get(13)?;
                let identity_role = identity_role_raw
                    .as_deref()
                    .map(|v| v.parse::<IdentityRole>())
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            4,
                            rusqlite::types::Type::Text,
                            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
                        )
                    })?;

                let skill_names = serde_json::from_str(&skill_names_raw).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        skill_names_raw.len(),
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                let effective_allowed_tools =
                    serde_json::from_str(&effective_allowed_tools_raw).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            effective_allowed_tools_raw.len(),
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                let tool_calls = serde_json::from_str(&tool_calls_raw).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        tool_calls_raw.len(),
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;

                Ok(AgentSession {
                    session_id: row.get(0)?,
                    task_id: row.get(1)?,
                    identity_id: row.get(2)?,
                    identity_name: row.get(3)?,
                    identity_role,
                    identity_block: row.get(5)?,
                    skill_names,
                    composed_context_hash: row.get(7)?,
                    effective_allowed_tools,
                    tool_calls,
                    outcome: row.get(10)?,
                    status: parse_agent_status(&status_raw),
                    created_at: crate::parse_timestamp(&created_at_raw)?,
                    updated_at: crate::parse_timestamp(&updated_at_raw)?,
                })
            },
        )
        .optional()
        .map_err(|e| OrbitError::Store(e.to_string()))
    }
}

impl<'a> StoreTx<'a> {
    pub fn insert_agent_session(&mut self, session: &AgentSession) -> Result<(), OrbitError> {
        let skill_names = serde_json::to_string(&session.skill_names)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let effective_allowed_tools = serde_json::to_string(&session.effective_allowed_tools)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let tool_calls = serde_json::to_string(&session.tool_calls)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        self.tx
            .execute(
                "INSERT INTO agent_sessions(session_id, task_id, identity_id, identity_name, identity_role, identity_block, skill_names, composed_context_hash, effective_allowed_tools, tool_calls, outcome, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    session.session_id,
                    session.task_id,
                    session.identity_id,
                    session.identity_name,
                    session.identity_role.map(|v| v.to_string()),
                    session.identity_block,
                    skill_names,
                    session.composed_context_hash,
                    effective_allowed_tools,
                    tool_calls,
                    session.outcome,
                    status_to_str(&session.status),
                    session.created_at.to_rfc3339(),
                    session.updated_at.to_rfc3339(),
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(())
    }

    pub fn update_agent_session(
        &mut self,
        session_id: &str,
        tool_calls: &[AgentToolCall],
        outcome: &str,
        status: AgentSessionStatus,
    ) -> Result<bool, OrbitError> {
        let tool_calls_json =
            serde_json::to_string(tool_calls).map_err(|e| OrbitError::Store(e.to_string()))?;

        let changed = self
            .tx
            .execute(
                "UPDATE agent_sessions
                 SET tool_calls = ?1, outcome = ?2, status = ?3, updated_at = ?4
                 WHERE session_id = ?5",
                params![
                    tool_calls_json,
                    outcome,
                    status_to_str(&status),
                    Utc::now().to_rfc3339(),
                    session_id
                ],
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        Ok(changed == 1)
    }
}

#[cfg(test)]
mod tests {
    use orbit_types::{AgentSession, AgentSessionStatus, AgentToolCall};
    use serde_json::json;

    use crate::Store;

    #[test]
    fn insert_and_update_agent_session() {
        let store = Store::open_in_memory().expect("store");

        let now = chrono::Utc::now();
        let session = AgentSession {
            session_id: "session-1".to_string(),
            task_id: "task-test-1".to_string(),
            identity_id: Some("linus".to_string()),
            identity_name: Some("Linus".to_string()),
            identity_role: Some(orbit_types::IdentityRole::Leader),
            identity_block: Some(
                "<agent_identity>\nName: Linus\nRole: leader\n</agent_identity>".to_string(),
            ),
            skill_names: vec!["alpha".to_string()],
            composed_context_hash: "hash".to_string(),
            effective_allowed_tools: vec!["fs.read".to_string()],
            tool_calls: vec![],
            outcome: "running".to_string(),
            status: AgentSessionStatus::Running,
            created_at: now,
            updated_at: now,
        };

        store
            .with_transaction(|tx| tx.insert_agent_session(&session))
            .expect("insert session");

        store
            .with_transaction(|tx| {
                tx.update_agent_session(
                    "session-1",
                    &[AgentToolCall {
                        name: "fs.read".to_string(),
                        input: json!({"path": "README.md"}),
                        output: Some(json!({"content": "ok"})),
                        success: true,
                    }],
                    "completed",
                    AgentSessionStatus::Completed,
                )?;
                Ok(())
            })
            .expect("update session");

        let loaded = store
            .get_agent_session("session-1")
            .expect("get")
            .expect("session");
        assert_eq!(loaded.status, AgentSessionStatus::Completed);
        assert_eq!(loaded.tool_calls.len(), 1);
    }
}
