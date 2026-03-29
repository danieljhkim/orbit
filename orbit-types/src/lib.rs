//! Shared domain types, error definitions, and ID generation for the Orbit workspace.
//!
//! This is the leaf crate in the dependency graph — it has no internal Orbit
//! dependencies and is imported by every other crate in the workspace.
//!
//! # Role
//! Acts as the single source of truth for all cross-crate data structures.
//! All other crates depend on this crate; it depends on none of them.
//!
//! # Key exports
//! - [`OrbitError`] — workspace-wide error enum; all crates use this exclusively
//! - [`OrbitId`] — deterministic, human-readable ID generation
//! - [`Activity`], [`Job`], [`JobRun`], [`Task`], [`Skill`] — core domain types
//! - [`ExecutionResult`] — process execution output shared between orbit-exec and callers
//! - [`AuditEvent`], [`OrbitEvent`] — event types for the audit trail and event bus
//! - [`Role`], [`PolicyDecision`] — RBAC primitives consumed by orbit-policy
//!
//! # Dependency direction
//! `orbit-types` ← orbit-policy, orbit-exec, orbit-tools, orbit-store,
//!                  orbit-agent, orbit-engine, orbit-core, orbit-cli

pub mod activity;
pub mod actor;
pub mod audit;
pub mod audit_event;
pub mod error;
pub mod event;
pub mod friction;
pub mod id;
pub mod job;
pub mod metrics;
pub mod policy_decision;
pub mod redaction;
pub mod role;
pub mod skill;
pub mod task;
pub mod tool;
pub mod workspace;

pub use activity::Activity;
pub use actor::ActorIdentity;
pub use audit::Audit;
pub use audit_event::{AuditEvent, AuditEventStatus, AuditStats};
pub use error::OrbitError;
pub use event::OrbitEvent;
pub use friction::FrictionEntry;
pub use id::OrbitId;
pub use job::{
    AgentCommitRequest, AgentResponseEnvelope, AgentRunError, Job, JobRun, JobRunState, JobRunStep,
    JobScheduleState, JobStep, JobTargetType, StepCondition, default_job_max_active_runs,
    default_max_iterations, default_retry_backoff_seconds,
};
pub use metrics::MetricsEntry;
pub use policy_decision::PolicyDecision;
pub use redaction::{
    is_sensitive_env_name, redact_sensitive_env_error, redact_sensitive_env_json,
    redact_sensitive_env_option, redact_sensitive_env_text,
};
pub use role::Role;
pub use skill::Skill;
pub use task::{
    Task, TaskComment, TaskComplexity, TaskHistoryEntry, TaskPriority, TaskStatus, TaskType,
};
pub use tool::{ExecutionResult, StoredTool, ToolParam, ToolSchema};
pub use workspace::{Workspace, WorkspacePaths, WorkspaceRegistry, WorkspaceStatus};

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use std::str::FromStr;

    use crate::{
        Activity, ActorIdentity, AgentCommitRequest, AgentResponseEnvelope, ExecutionResult,
        FrictionEntry, Job, JobRun, JobRunState, JobScheduleState, JobStep, MetricsEntry,
        OrbitEvent, Role, Skill, StepCondition, TaskStatus,
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
            default_input: Some(serde_json::json!({"base": "main"})),
            max_active_runs: 2,
            max_iterations: 1,
            steps: vec![JobStep {
                target_id: "exec-1".to_string(),
                agent_cli: "claude".to_string(),
                timeout_seconds: 300,
                retry_max_attempts: 3,
                ..Default::default()
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let job_value = serde_json::to_value(job).expect("serialize job");
        assert_eq!(job_value["state"], "enabled");
        assert_eq!(job_value["default_input"]["base"], "main");
        assert_eq!(job_value["max_active_runs"], 2);
        assert_eq!(job_value["steps"][0]["target_type"], "activity");
        assert_eq!(job_value["steps"][0]["retry_max_attempts"], 3);
        assert_eq!(job_value["steps"][0]["retry_backoff_seconds"], 10);
        assert_eq!(job_value["steps"][0]["condition"], "always");

        let run = JobRun {
            run_id: "run-1".to_string(),
            job_id: "job-1".to_string(),
            attempt: 1,
            state: JobRunState::Running,
            scheduled_at: Utc::now(),
            started_at: None,
            finished_at: None,
            duration_ms: None,
            pid: Some(12345),
            pid_start_time: Some("Sat Mar 28 17:05:00 2026".to_string()),
            steps: vec![],
            created_at: Utc::now(),
        };
        let run_value = serde_json::to_value(run).expect("serialize run");
        assert_eq!(run_value["state"], "running");
        assert_eq!(run_value["attempt"], 1);
        assert_eq!(run_value["pid"], 12345);
        assert_eq!(run_value["pid_start_time"], "Sat Mar 28 17:05:00 2026");

        // Old jrun.yaml without pid must deserialize cleanly (serde default)
        let no_pid_json = r#"{"run_id":"r","job_id":"j","attempt":1,"state":"pending","scheduled_at":"2026-01-01T00:00:00Z","created_at":"2026-01-01T00:00:00Z"}"#;
        let deserialized: JobRun =
            serde_json::from_str(no_pid_json).expect("deserialize run without pid");
        assert_eq!(deserialized.pid, None);
        assert_eq!(deserialized.pid_start_time, None);

        // Retrying variant serializes and parses correctly
        assert_eq!(
            serde_json::to_value(JobRunState::Retrying).expect("serialize retrying"),
            "retrying"
        );
        assert_eq!(
            serde_json::to_value(JobRunState::Skipped).expect("serialize skipped"),
            "skipped"
        );
        assert_eq!(
            "retrying".parse::<JobRunState>().expect("parse retrying"),
            JobRunState::Retrying
        );
        assert_eq!(
            "skipped".parse::<JobRunState>().expect("parse skipped"),
            JobRunState::Skipped
        );
    }

    #[test]
    fn job_step_condition_defaults_to_always_when_missing() {
        let step: JobStep = serde_json::from_value(serde_json::json!({
            "target_type": "activity",
            "target_id": "exec-1",
            "agent_cli": "codex",
            "timeout_seconds": 30
        }))
        .expect("deserialize step");

        assert_eq!(step.condition, StepCondition::Always);
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
            tools: vec!["fs.read".to_string(), "fs.write".to_string()],
            proc_allowed_programs: vec![],
            workspace_path: None,
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

    #[test]
    fn friction_entry_round_trips() {
        let entry = FrictionEntry {
            ts: Utc::now(),
            job_run: "JR-123".to_string(),
            step: "commit_changes".to_string(),
            task_id: Some("T20260322-022125".to_string()),
            command: "commit_task_changes".to_string(),
            input: "{\"task_id\":\"T20260322-022125\"}".to_string(),
            exit_code: Some(1),
            stderr: "boom".to_string(),
            actor_identity: ActorIdentity::agent("codex", "gpt-5.4"),
        };

        let json = serde_json::to_string(&entry).expect("serialize friction entry");
        let decoded: FrictionEntry =
            serde_json::from_str(&json).expect("deserialize friction entry");

        assert_eq!(decoded, entry);
    }

    #[test]
    fn metrics_entry_round_trips() {
        let entry = MetricsEntry {
            ts: Utc::now(),
            job_run: "JR-456".to_string(),
            step: "execute_task".to_string(),
            task_id: Some("T20260322-024246".to_string()),
            actor_identity: ActorIdentity::agent("claude", "opus-4.6"),
            tool_invocations: 12,
            token_usage: Some(25000),
            step_duration_ms: Some(60000),
            retry_count: 1,
        };

        let json = serde_json::to_string(&entry).expect("serialize metrics entry");
        let decoded: MetricsEntry = serde_json::from_str(&json).expect("deserialize metrics entry");

        assert_eq!(decoded, entry);
    }
}
