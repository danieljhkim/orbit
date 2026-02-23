use chrono::Utc;
use orbit_types::{
    AgentSession, AgentSessionStatus, AgentToolCall, OrbitError, Skill, TaskSkillAttachment,
};
use rusqlite::{OptionalExtension, params};

use crate::{Store, StoreTx};

fn parse_skill(row: &rusqlite::Row<'_>) -> rusqlite::Result<Skill> {
    let context_files_raw: String = row.get(4)?;
    let allowed_tools_raw: String = row.get(5)?;
    let role_raw: String = row.get(6)?;
    let created_at_raw: String = row.get(7)?;
    let updated_at_raw: String = row.get(8)?;

    let context_files: Vec<String> = serde_json::from_str(&context_files_raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            context_files_raw.len(),
            rusqlite::types::Type::Text,
            Box::new(e),
        )
    })?;
    let allowed_tools: Vec<String> = serde_json::from_str(&allowed_tools_raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            allowed_tools_raw.len(),
            rusqlite::types::Type::Text,
            Box::new(e),
        )
    })?;

    let role = role_raw.parse().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            role_raw.len(),
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })?;

    Ok(Skill {
        schema_version: row.get::<_, i64>(0)? as u8,
        name: row.get(1)?,
        description: row.get(2)?,
        instructions: row.get(3)?,
        context_files,
        allowed_tools,
        role,
        created_at: crate::parse_timestamp(&created_at_raw)?,
        updated_at: crate::parse_timestamp(&updated_at_raw)?,
    })
}

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
    pub fn list_skills(&self) -> Result<Vec<Skill>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT schema_version, name, description, instructions, context_files, allowed_tools, role, created_at, updated_at FROM skills ORDER BY name",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map([], parse_skill)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_skill(&self, name: &str) -> Result<Option<Skill>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        conn.query_row(
            "SELECT schema_version, name, description, instructions, context_files, allowed_tools, role, created_at, updated_at FROM skills WHERE name = ?1",
            [name],
            parse_skill,
        )
        .optional()
        .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn list_task_skills(&self, task_id: &str) -> Result<Vec<Skill>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT s.schema_version, s.name, s.description, s.instructions, s.context_files, s.allowed_tools, s.role, s.created_at, s.updated_at
                 FROM task_skills ts
                 JOIN skills s ON s.name = ts.skill_name
                 WHERE ts.task_id = ?1
                 ORDER BY ts.attachment_order ASC, ts.skill_name ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map([task_id], parse_skill)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn list_task_skill_attachments(
        &self,
        task_id: &str,
    ) -> Result<Vec<TaskSkillAttachment>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;
        let mut stmt = conn
            .prepare(
                "SELECT task_id, skill_name, attachment_order FROM task_skills WHERE task_id = ?1 ORDER BY attachment_order ASC, skill_name ASC",
            )
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let rows = stmt
            .query_map([task_id], |row| {
                Ok(TaskSkillAttachment {
                    task_id: row.get(0)?,
                    skill_name: row.get(1)?,
                    attachment_order: row.get(2)?,
                })
            })
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| OrbitError::Store(e.to_string()))
    }

    pub fn get_agent_session(&self, session_id: &str) -> Result<Option<AgentSession>, OrbitError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| OrbitError::Store(format!("mutex poisoned: {e}")))?;

        conn.query_row(
            "SELECT session_id, task_id, skill_names, composed_context_hash, effective_allowed_tools, tool_calls, outcome, status, created_at, updated_at
             FROM agent_sessions WHERE session_id = ?1",
            [session_id],
            |row| {
                let skill_names_raw: String = row.get(2)?;
                let effective_allowed_tools_raw: String = row.get(4)?;
                let tool_calls_raw: String = row.get(5)?;
                let status_raw: String = row.get(7)?;
                let created_at_raw: String = row.get(8)?;
                let updated_at_raw: String = row.get(9)?;

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
                    skill_names,
                    composed_context_hash: row.get(3)?,
                    effective_allowed_tools,
                    tool_calls,
                    outcome: row.get(6)?,
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
    pub fn insert_skill(&mut self, _skill: &Skill) -> Result<(), OrbitError> {
        Err(OrbitError::Store(
            "legacy sqlite skill mutation is disabled; use file-based skills".to_string(),
        ))
    }

    pub fn update_skill(&mut self, _skill: &Skill) -> Result<bool, OrbitError> {
        Err(OrbitError::Store(
            "legacy sqlite skill mutation is disabled; use file-based skills".to_string(),
        ))
    }

    pub fn delete_skill(&mut self, _name: &str) -> Result<bool, OrbitError> {
        Err(OrbitError::Store(
            "legacy sqlite skill mutation is disabled; use file-based skills".to_string(),
        ))
    }

    pub fn attach_skill_to_task(
        &mut self,
        _task_id: &str,
        _skill_name: &str,
    ) -> Result<bool, OrbitError> {
        Err(OrbitError::Store(
            "task-skill attachment is disabled; use work.skill_refs".to_string(),
        ))
    }

    pub fn detach_skill_from_task(
        &mut self,
        _task_id: &str,
        _skill_name: &str,
    ) -> Result<bool, OrbitError> {
        Err(OrbitError::Store(
            "task-skill attachment is disabled; use work.skill_refs".to_string(),
        ))
    }

    pub fn insert_agent_session(&mut self, session: &AgentSession) -> Result<(), OrbitError> {
        let skill_names = serde_json::to_string(&session.skill_names)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let effective_allowed_tools = serde_json::to_string(&session.effective_allowed_tools)
            .map_err(|e| OrbitError::Store(e.to_string()))?;
        let tool_calls = serde_json::to_string(&session.tool_calls)
            .map_err(|e| OrbitError::Store(e.to_string()))?;

        self.tx
            .execute(
                "INSERT INTO agent_sessions(session_id, task_id, skill_names, composed_context_hash, effective_allowed_tools, tool_calls, outcome, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    session.session_id,
                    session.task_id,
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

    pub fn skill_exists(&mut self, name: &str) -> Result<bool, OrbitError> {
        let exists = self
            .tx
            .query_row(
                "SELECT 1 FROM skills WHERE name = ?1 LIMIT 1",
                [name],
                |_| Ok(()),
            )
            .optional()
            .map_err(|e| OrbitError::Store(e.to_string()))?
            .is_some();
        Ok(exists)
    }
}

#[cfg(test)]
mod tests {
    use orbit_types::{AgentSession, AgentSessionStatus, AgentToolCall, Role, Skill};
    use serde_json::json;

    use crate::Store;
    use crate::task_store::TaskInsertParams;

    fn sample_skill(name: &str) -> Skill {
        let now = chrono::Utc::now();
        Skill {
            schema_version: 1,
            name: name.to_string(),
            description: Some("desc".to_string()),
            instructions: "Do thing".to_string(),
            context_files: vec!["ARCHITECTURE.md".to_string()],
            allowed_tools: vec!["fs.read".to_string()],
            role: Role::Agent,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn legacy_skill_mutation_is_disabled() {
        let store = Store::open_in_memory().expect("store");
        let err = store
            .with_transaction(|tx| {
                tx.insert_skill(&sample_skill("alpha"))?;
                Ok(())
            })
            .expect_err("insert should fail");
        assert!(
            err.to_string()
                .contains("legacy sqlite skill mutation is disabled")
        );
    }

    #[test]
    fn task_skill_attachment_is_disabled() {
        let store = Store::open_in_memory().expect("store");
        let task = store
            .with_transaction(|tx| {
                tx.insert_task(&TaskInsertParams {
                    title: "task".to_string(),
                    ..Default::default()
                })
            })
            .expect("task");

        let err = store
            .with_transaction(|tx| {
                tx.attach_skill_to_task(&task.id, "b")?;
                Ok(())
            })
            .expect_err("attach should fail");
        assert!(
            err.to_string()
                .contains("task-skill attachment is disabled")
        );
    }

    #[test]
    fn insert_and_update_agent_session() {
        let store = Store::open_in_memory().expect("store");
        let task = store
            .with_transaction(|tx| {
                tx.insert_task(&TaskInsertParams {
                    title: "task".to_string(),
                    ..Default::default()
                })
            })
            .expect("task");

        let now = chrono::Utc::now();
        let session = AgentSession {
            session_id: "session-1".to_string(),
            task_id: task.id.clone(),
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
