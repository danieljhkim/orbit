pub mod activity;
pub mod agent_session;
pub mod audit;
pub mod audit_event;
pub mod error;
pub mod event;
pub mod id;
pub mod identity;
pub mod job;
pub mod memo;
pub mod role;
pub mod skill;
pub mod task;
pub mod tool;

pub use activity::Activity;
pub use agent_session::{AgentSession, AgentSessionStatus, AgentToolCall};
pub use audit::Audit;
pub use audit_event::{AuditEvent, AuditEventStatus, AuditStats};
pub use error::OrbitError;
pub use event::OrbitEvent;
pub use id::OrbitId;
pub use identity::{IdentityRole, ResolvedIdentity};
pub use job::{
    AgentCommitRequest, AgentResponseEnvelope, AgentRunError, Job, JobRun, JobRunState, JobRunStep,
    JobScheduleState, JobStep, JobTargetType,
};
pub use memo::Memo;
pub use role::Role;
pub use skill::Skill;
pub use task::{Task, TaskComment, TaskPriority, TaskStatus, TaskType};
pub use tool::{ExecutionResult, PolicyDecision, StoredTool, ToolParam, ToolSchema};

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use std::str::FromStr;

    use crate::{
        Activity, AgentCommitRequest, AgentResponseEnvelope, ExecutionResult, Job, JobRun,
        JobRunState, JobScheduleState, JobStep, JobTargetType, OrbitEvent, Role, Skill, TaskStatus,
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
    fn job_shapes_are_stable() {
        let job = Job {
            job_id: "job-1".to_string(),
            state: JobScheduleState::Enabled,
            steps: vec![JobStep {
                target_type: JobTargetType::Activity,
                target_id: "exec-1".to_string(),
                agent_cli: "claude".to_string(),
                timeout_seconds: 300,
                env_extra: vec![],
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let job_value = serde_json::to_value(job).expect("serialize job");
        assert_eq!(job_value["state"], "enabled");
        assert_eq!(job_value["steps"][0]["target_type"], "activity");

        let run = JobRun {
            run_id: "run-1".to_string(),
            job_id: "job-1".to_string(),
            attempt: 1,
            state: JobRunState::Running,
            scheduled_at: Utc::now(),
            started_at: None,
            finished_at: None,
            duration_ms: None,
            steps: vec![],
            created_at: Utc::now(),
        };
        let run_value = serde_json::to_value(run).expect("serialize run");
        assert_eq!(run_value["state"], "running");
        assert_eq!(run_value["attempt"], 1);
    }

    #[test]
    fn activity_shape_is_stable() {
        let spec = Activity {
            id: "exec-1".to_string(),
            spec_type: "agent_invoke".to_string(),
            description: "Analyze repository".to_string(),
            input_schema_json: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } }
            }),
            output_schema_json: serde_json::json!({
                "type": "object",
                "properties": { "score": { "type": "number" } }
            }),
            spec_config: serde_json::json!({
                "instruction": "Summarize the repository health.",
                "skill_refs": ["orbit-assess-codebase"],
                "tools": ["fs.read", "fs.write"]
            }),
            workspace_path: None,
            identity_id: Some("prii".to_string()),
            created_by: Some("human".to_string()),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let spec_json = serde_json::to_value(spec).expect("serialize spec");
        assert_eq!(spec_json["spec_type"], "agent_invoke");
        assert_eq!(
            spec_json["spec_config"]["instruction"],
            "Summarize the repository health."
        );
        assert_eq!(spec_json["spec_config"]["tools"][0], "fs.read");
        assert_eq!(spec_json["is_active"], true);
    }

    #[test]
    fn agent_response_envelope_shape_is_stable() {
        let envelope = AgentResponseEnvelope {
            schema_version: 1,
            status: "success".to_string(),
            result: Some(serde_json::json!({ "k": "v" })),
            error: None,
            duration_ms: 1234,
        };
        let value = serde_json::to_value(envelope).expect("serialize envelope");
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["status"], "success");
        assert_eq!(value["durationMs"], 1234);
    }

    #[test]
    fn agent_commit_request_round_trips() {
        let request = AgentCommitRequest {
            message: "feat: orbit-owned commit".to_string(),
            files: vec!["orbit-core/src/command/job.rs".to_string()],
        };
        let value = serde_json::to_value(&request).expect("serialize commit request");
        assert_eq!(value["message"], "feat: orbit-owned commit");
        assert_eq!(value["files"][0], "orbit-core/src/command/job.rs");
        let decoded: AgentCommitRequest =
            serde_json::from_value(value).expect("deserialize commit request");
        assert_eq!(decoded, request);
    }

    #[test]
    fn task_status_accepts_snake_case_alias_and_formats_for_cli() {
        assert_eq!(
            TaskStatus::from_str("in_progress").expect("snake_case alias"),
            TaskStatus::InProgress
        );
        assert_eq!(
            TaskStatus::from_str("in-progress").expect("canonical cli spelling"),
            TaskStatus::InProgress
        );
        assert_eq!(TaskStatus::InProgress.to_string(), "in-progress");
    }
}
