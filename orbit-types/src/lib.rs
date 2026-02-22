pub mod audit;
pub mod entry;
pub mod error;
pub mod event;
pub mod id;
pub mod job;
pub mod memo;
pub mod role;
pub mod skill;
pub mod task;
pub mod tool;
pub mod watch;

pub use audit::Audit;
pub use entry::{AuthorType, EntityType, Entry, EntryType};
pub use error::OrbitError;
pub use event::OrbitEvent;
pub use id::OrbitId;
pub use job::{Job, JobScheduleState, JobSession, JobSessionStatus, JobTrigger};
pub use memo::Memo;
pub use role::Role;
pub use skill::{AgentSession, AgentSessionStatus, AgentToolCall, Skill, TaskSkillAttachment};
pub use task::{Task, TaskPriority, TaskStatus, TaskType};
pub use tool::{ExecutionResult, PolicyDecision, StoredTool, ToolParam, ToolSchema};
pub use watch::Watch;

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::{
        ExecutionResult, Job, JobScheduleState, JobSession, JobSessionStatus, JobTrigger,
        OrbitEvent, Role, Skill,
    };

    #[test]
    fn orbit_event_shape_is_stable() {
        let event = OrbitEvent::ToolExecuted {
            name: "fs.read".to_string(),
        };
        let json = serde_json::to_value(event).expect("serialize event");

        assert_eq!(json["type"], "ToolExecuted");
        assert_eq!(json["data"]["name"], "fs.read");
    }

    #[test]
    fn execution_result_round_trips() {
        let result = ExecutionResult {
            success: true,
            stdout: "ok".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 12,
            output: Some(serde_json::json!({"k": "v"})),
        };

        let json = serde_json::to_string(&result).expect("serialize result");
        let decoded: ExecutionResult = serde_json::from_str(&json).expect("deserialize result");

        assert_eq!(decoded, result);
    }

    #[test]
    fn role_round_trips() {
        let role = Role::Agent;
        let json = serde_json::to_string(&role).expect("serialize role");
        assert_eq!(json, "\"agent\"");
        let decoded: Role = serde_json::from_str(&json).expect("deserialize role");
        assert_eq!(decoded, Role::Agent);
    }

    #[test]
    fn skill_shape_is_stable() {
        let skill = Skill {
            schema_version: 1,
            name: "refactor-rust-module".to_string(),
            description: Some("test".to_string()),
            instructions: "Do things".to_string(),
            context_files: vec!["ARCHITECTURE.md".to_string()],
            allowed_tools: vec!["fs.read".to_string()],
            role: Role::Agent,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let value = serde_json::to_value(skill).expect("serialize skill");
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["name"], "refactor-rust-module");
        assert_eq!(value["role"], "agent");
    }

    #[test]
    fn entry_shape_is_stable() {
        let value = serde_json::json!({
            "id": "entry-1",
            "entity_type": "task",
            "entity_id": "task-1",
            "session_id": null,
            "sequence_number": 1,
            "entry_type": "comment",
            "author_type": "human",
            "author_id": "daniel",
            "author_model": null,
            "body": "hello",
            "created_at": "2026-02-22T00:00:00Z"
        });

        let decoded: crate::Entry = serde_json::from_value(value.clone()).expect("decode entry");
        let reencoded = serde_json::to_value(decoded).expect("encode entry");
        assert_eq!(reencoded["entity_type"], "task");
        assert_eq!(reencoded["entry_type"], "comment");
        assert_eq!(reencoded["author_type"], "human");
    }

    #[test]
    fn job_shapes_are_stable() {
        let job = Job {
            job_id: "job-1".to_string(),
            name: "Hourly Check".to_string(),
            task_id: "task-1".to_string(),
            schedule_spec: "0 * * * *".to_string(),
            timezone: "UTC".to_string(),
            state: JobScheduleState::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            paused_at: None,
            deleted_at: None,
            last_run_session_id: None,
            last_run_at: None,
            next_run_at: None,
            last_error: None,
        };
        let job_value = serde_json::to_value(job).expect("serialize job");
        assert_eq!(job_value["state"], "active");

        let session = JobSession {
            session_id: "session-1".to_string(),
            job_id: "job-1".to_string(),
            task_id: "task-1".to_string(),
            trigger: JobTrigger::Manual,
            trigger_time: Utc::now(),
            started_at: None,
            finished_at: None,
            status: JobSessionStatus::Running,
            exit_code: None,
            error: None,
            composed_context_hash: None,
            effective_allowlist_hash: None,
            created_by_role: Role::Admin,
            created_at: Utc::now(),
            cancel_requested_at: None,
        };
        let session_value = serde_json::to_value(session).expect("serialize session");
        assert_eq!(session_value["trigger"], "manual");
        assert_eq!(session_value["status"], "running");
    }
}
