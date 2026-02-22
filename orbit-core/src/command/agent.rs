use chrono::Utc;
use orbit_types::{
    AgentSession, AgentSessionStatus, AgentToolCall, AuthorType, EntityType, EntryType, OrbitError,
    OrbitEvent, Role,
};
use serde_json::json;

use crate::OrbitRuntime;
use crate::agent::context::{compose_agent_context, parse_planned_tool_calls};
use crate::command::entry::EntryAddParams;

const AGENT_ENTRY_AUTHOR_ID: &str = "orbit-agent";
const AGENT_ENTRY_AUTHOR_MODEL: &str = "planner-v1";
const SYSTEM_ENTRY_AUTHOR_ID: &str = "runtime";

#[derive(Debug, Clone)]
pub struct AgentRunResult {
    pub session_id: String,
    pub task_id: String,
    pub tool_calls_executed: usize,
    pub status: AgentSessionStatus,
}

impl OrbitRuntime {
    pub fn get_agent_session(&self, session_id: &str) -> Result<Option<AgentSession>, OrbitError> {
        self.context.store.get_agent_session(session_id)
    }

    pub fn run_agent_task(&self, task_id: &str) -> Result<AgentRunResult, OrbitError> {
        let task = self.get_task(task_id)?;
        let skills = self.list_task_skills(task_id)?;
        let session_id = format!(
            "session-{}",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );

        let composed = match compose_agent_context(self, &task, &skills, Role::Agent) {
            Ok(composed) => composed,
            Err(err) => {
                let _ = self.with_mutation(|_| {
                    Ok((
                        (),
                        OrbitEvent::AgentSessionCompleted {
                            session_id: session_id.clone(),
                            task_id: task.id.clone(),
                            status: "failed".to_string(),
                        },
                    ))
                });
                return Err(err);
            }
        };

        let planned_calls = match parse_planned_tool_calls(&task.instructions) {
            Ok(calls) => calls,
            Err(err) => {
                let _ = self.with_mutation(|_| {
                    Ok((
                        (),
                        OrbitEvent::AgentSessionCompleted {
                            session_id: session_id.clone(),
                            task_id: task.id.clone(),
                            status: "failed".to_string(),
                        },
                    ))
                });
                return Err(err);
            }
        };

        let now = Utc::now();
        let mut executed_calls: Vec<AgentToolCall> = Vec::new();
        let skill_names = composed.skill_names.clone();
        let mut session_outcome = "running".to_string();

        let session = AgentSession {
            session_id: session_id.clone(),
            task_id: task.id.clone(),
            skill_names: skill_names.clone(),
            composed_context_hash: composed.composed_context_hash.clone(),
            effective_allowed_tools: composed.effective_allowed_tools.clone(),
            tool_calls: vec![],
            outcome: session_outcome.clone(),
            status: AgentSessionStatus::Running,
            created_at: now,
            updated_at: now,
        };

        self.with_mutation(|tx| {
            tx.insert_agent_session(&session)?;
            Ok((
                (),
                OrbitEvent::AgentSessionStarted {
                    session_id: session_id.clone(),
                    task_id: task.id.clone(),
                    skill_names: skill_names.clone(),
                    composed_context_hash: composed.composed_context_hash.clone(),
                    effective_allowed_tools: composed.effective_allowed_tools.clone(),
                },
            ))
        })?;
        self.append_system_session_entry(
            &session_id,
            format!("agent session started for task {}", task.id),
        )?;
        self.append_agent_reasoning_entry(
            &session_id,
            format!("planner parsed {} tool call(s)", planned_calls.len()),
        )?;

        for (call_index, mut planned) in planned_calls.into_iter().enumerate() {
            if !composed.effective_allowed_tools.contains(&planned.name) {
                planned.success = false;
                planned.output = Some(json!({
                    "error": format!("tool '{}' not permitted by effective allowlist", planned.name)
                }));
                let failure_message = format!(
                    "tool call #{} denied by allowlist: {}",
                    call_index + 1,
                    planned.name
                );
                executed_calls.push(planned.clone());
                let session_status = AgentSessionStatus::Failed;
                session_outcome = "tool not permitted".to_string();
                self.append_agent_reasoning_entry(&session_id, failure_message)?;

                self.record_agent_tool_call(
                    &AgentSessionUpdate {
                        session_id: &session_id,
                        task_id: &task.id,
                        skill_names: &skill_names,
                        all_calls: &executed_calls,
                        outcome: &session_outcome,
                        status: session_status.clone(),
                    },
                    &planned,
                )?;
                self.finish_agent_session(
                    &session_id,
                    &task.id,
                    &executed_calls,
                    &session_outcome,
                    session_status.clone(),
                )?;
                self.append_system_session_entry(
                    &session_id,
                    format!(
                        "agent session completed with status={}",
                        status_to_text(&session_status)
                    ),
                )?;

                return Err(OrbitError::PolicyDenied(format!(
                    "tool '{}' not permitted by effective allowlist",
                    planned.name
                )));
            }

            match self.run_tool_with_role(&planned.name, planned.input.clone(), composed.role) {
                Ok(output) => {
                    planned.success = true;
                    planned.output = Some(output);
                    self.append_agent_reasoning_entry(
                        &session_id,
                        format!("tool call #{} succeeded: {}", call_index + 1, planned.name),
                    )?;
                    executed_calls.push(planned.clone());
                    self.record_agent_tool_call(
                        &AgentSessionUpdate {
                            session_id: &session_id,
                            task_id: &task.id,
                            skill_names: &skill_names,
                            all_calls: &executed_calls,
                            outcome: "running",
                            status: AgentSessionStatus::Running,
                        },
                        &planned,
                    )?;
                }
                Err(err) => {
                    planned.success = false;
                    planned.output = Some(json!({ "error": err.to_string() }));
                    self.append_agent_reasoning_entry(
                        &session_id,
                        format!(
                            "tool call #{} failed: {} ({})",
                            call_index + 1,
                            planned.name,
                            err
                        ),
                    )?;
                    executed_calls.push(planned.clone());
                    let session_status = AgentSessionStatus::Failed;
                    session_outcome = err.to_string();
                    self.record_agent_tool_call(
                        &AgentSessionUpdate {
                            session_id: &session_id,
                            task_id: &task.id,
                            skill_names: &skill_names,
                            all_calls: &executed_calls,
                            outcome: &session_outcome,
                            status: session_status.clone(),
                        },
                        &planned,
                    )?;
                    self.finish_agent_session(
                        &session_id,
                        &task.id,
                        &executed_calls,
                        &session_outcome,
                        session_status.clone(),
                    )?;
                    self.append_system_session_entry(
                        &session_id,
                        format!(
                            "agent session completed with status={}",
                            status_to_text(&session_status)
                        ),
                    )?;
                    return Err(err);
                }
            }
        }

        let session_status = AgentSessionStatus::Completed;
        session_outcome = "completed".to_string();
        self.finish_agent_session(
            &session_id,
            &task.id,
            &executed_calls,
            &session_outcome,
            session_status.clone(),
        )?;
        self.append_system_session_entry(
            &session_id,
            format!(
                "agent session completed with status={}",
                status_to_text(&session_status)
            ),
        )?;

        Ok(AgentRunResult {
            session_id,
            task_id: task.id,
            tool_calls_executed: executed_calls.len(),
            status: session_status,
        })
    }

    fn record_agent_tool_call(
        &self,
        update: &AgentSessionUpdate<'_>,
        call: &AgentToolCall,
    ) -> Result<(), OrbitError> {
        self.with_mutation(|tx| {
            tx.update_agent_session(
                update.session_id,
                update.all_calls,
                update.outcome,
                update.status.clone(),
            )?;
            Ok((
                (),
                OrbitEvent::AgentToolCall {
                    session_id: update.session_id.to_string(),
                    task_id: update.task_id.to_string(),
                    skill_names: update.skill_names.to_vec(),
                    tool_name: call.name.clone(),
                    input: call.input.clone(),
                    output: call.output.clone(),
                    success: call.success,
                },
            ))
        })
    }

    fn finish_agent_session(
        &self,
        session_id: &str,
        task_id: &str,
        all_calls: &[AgentToolCall],
        outcome: &str,
        status: AgentSessionStatus,
    ) -> Result<(), OrbitError> {
        self.with_mutation(|tx| {
            tx.update_agent_session(session_id, all_calls, outcome, status.clone())?;
            Ok((
                (),
                OrbitEvent::AgentSessionCompleted {
                    session_id: session_id.to_string(),
                    task_id: task_id.to_string(),
                    status: status_to_text(&status).to_string(),
                },
            ))
        })
    }

    fn append_agent_reasoning_entry(
        &self,
        session_id: &str,
        body: String,
    ) -> Result<(), OrbitError> {
        let _entry = self.add_entry(EntryAddParams {
            entity_type: EntityType::Session,
            entity_id: session_id.to_string(),
            session_id: Some(session_id.to_string()),
            entry_type: EntryType::Reasoning,
            author_type: AuthorType::Agent,
            author_id: AGENT_ENTRY_AUTHOR_ID.to_string(),
            author_model: Some(AGENT_ENTRY_AUTHOR_MODEL.to_string()),
            body,
        })?;
        Ok(())
    }

    fn append_system_session_entry(
        &self,
        session_id: &str,
        body: String,
    ) -> Result<(), OrbitError> {
        let _entry = self.add_entry(EntryAddParams {
            entity_type: EntityType::Session,
            entity_id: session_id.to_string(),
            session_id: Some(session_id.to_string()),
            entry_type: EntryType::System,
            author_type: AuthorType::System,
            author_id: SYSTEM_ENTRY_AUTHOR_ID.to_string(),
            author_model: None,
            body,
        })?;
        Ok(())
    }
}

fn status_to_text(status: &AgentSessionStatus) -> &'static str {
    match status {
        AgentSessionStatus::Running => "running",
        AgentSessionStatus::Completed => "completed",
        AgentSessionStatus::Failed => "failed",
    }
}

struct AgentSessionUpdate<'a> {
    session_id: &'a str,
    task_id: &'a str,
    skill_names: &'a [String],
    all_calls: &'a [AgentToolCall],
    outcome: &'a str,
    status: AgentSessionStatus,
}
