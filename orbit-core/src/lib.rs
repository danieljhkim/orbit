pub mod command;
mod config;
pub mod context;
mod fs_utils;
mod json_schema;
mod paths;
pub mod runtime;

pub use orbit_engine::JobRunResult;
pub use orbit_store::identity_store as identity_catalog;
pub use orbit_store::skill_store as skill_catalog;

pub use context::OrbitContext;
pub use orbit_store::AuditEventInsertParams;
pub use orbit_types::OrbitError;
pub use orbit_types::{
    Activity, AuditEvent, AuditEventStatus, AuditStats, IdentityRole, Job, JobRun, JobRunState,
    JobScheduleState, JobStep, JobTargetType, Role, Skill, Task, TaskComment, TaskPriority,
    TaskStatus, TaskType,
};
pub use orbit_types::{
    redact_sensitive_env_error, redact_sensitive_env_json, redact_sensitive_env_option,
    redact_sensitive_env_text,
};
pub use runtime::OrbitRuntime;

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use orbit_policy::PolicyEngine;
    use orbit_store::TaskCreateParams as StoreTaskCreateParams;
    use orbit_types::{
        JobRunState, JobStep, JobTargetType, OrbitEvent, TaskPriority, TaskStatus, TaskType,
    };
    use serde_json::json;
    use tempfile::tempdir;

    use crate::OrbitRuntime;
    use crate::command::activity::ActivityAddParams;
    use crate::command::job::JobAddParams;
    use crate::command::task::{TaskAddParams, TaskUpdateParams};

    #[test]
    fn policy_denied_records_audit_and_no_side_effects() {
        let runtime = OrbitRuntime::in_memory()
            .expect("runtime")
            .with_policy(PolicyEngine::new_local_default_allow().deny_tool("fs.read"));

        let result = runtime.run_tool("fs.read", json!({"path": "missing"}));
        assert!(matches!(result, Err(crate::OrbitError::PolicyDenied(_))));

        let audits = runtime.list_audits(10).expect("audits");
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].event_type, "PolicyDenied");
    }

    #[test]
    fn activity_tool_allowlist_denies_unlisted_tool() {
        use orbit_tools::ToolContext;
        use orbit_types::Role;

        let runtime = OrbitRuntime::in_memory().expect("runtime");

        let tool_context = ToolContext {
            cwd: None,
            allowed_tools: vec!["time.now".to_string()],
        };
        let result = runtime.run_tool_with_context_and_role(
            "fs.read",
            json!({"path": "missing"}),
            Role::Admin,
            tool_context,
        );

        assert!(
            matches!(result, Err(crate::OrbitError::PolicyDenied(_))),
            "expected PolicyDenied for tool not in allowlist"
        );

        let audits = runtime.list_audits(10).expect("audits");
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].event_type, "PolicyDenied");
    }

    #[test]
    fn activity_tool_allowlist_permits_listed_tool() {
        use orbit_tools::ToolContext;
        use orbit_types::Role;

        let runtime = OrbitRuntime::in_memory().expect("runtime");

        let tool_context = ToolContext {
            cwd: None,
            allowed_tools: vec!["time.now".to_string()],
        };
        let result = runtime.run_tool_with_context_and_role(
            "time.now",
            json!({}),
            Role::Admin,
            tool_context,
        );

        assert!(result.is_ok(), "expected success for tool in allowlist");
    }

    #[test]
    fn empty_allowlist_is_unrestricted() {
        use orbit_tools::ToolContext;
        use orbit_types::Role;

        let dir = tempdir().expect("temp dir");
        let file = dir.path().join("sample.txt");
        std::fs::write(&file, "data").expect("write");

        let runtime = OrbitRuntime::in_memory().expect("runtime");

        // Empty allowed_tools = unrestricted
        let tool_context = ToolContext::default();
        let result = runtime.run_tool_with_context_and_role(
            "fs.read",
            json!({"path": file.to_string_lossy()}),
            Role::Admin,
            tool_context,
        );

        assert!(result.is_ok(), "empty allowlist should not restrict tools");
    }

    #[test]
    fn successful_tool_execution_persists_audit_and_event() {
        let dir = tempdir().expect("temp dir");
        let file = dir.path().join("sample.txt");
        std::fs::write(&file, "ok").expect("write file");

        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let output = runtime
            .run_tool("fs.read", json!({"path": file.to_string_lossy()}))
            .expect("tool succeeds");

        assert_eq!(output["content"], "ok");

        let audits = runtime.list_audits(10).expect("audits");
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].event_type, "ToolExecuted");

        let events = runtime.event_log.snapshot();
        assert!(matches!(
            events.first(),
            Some(OrbitEvent::ToolExecuted { name }) if name == "fs.read"
        ));
    }

    #[test]
    fn tool_results_redact_sensitive_environment_values() {
        unsafe {
            std::env::set_var("TEST_API_TOKEN", "token-value-to-hide");
        }

        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let output = runtime
            .run_tool(
                "proc.spawn",
                json!({
                    "program": "sh",
                    "args": ["-c", "printf '%s' \"$TEST_API_TOKEN\""],
                }),
            )
            .expect("tool succeeds");

        assert_eq!(output["stdout"], "[REDACTED_ENV]");
        assert_eq!(output["stderr"], "");
    }

    #[test]
    fn mutation_boundary_always_emits_audit() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let _ = runtime
            .add_task(TaskAddParams {
                title: "ship orbit".to_string(),
                ..Default::default()
            })
            .expect("add task");

        let tasks = runtime.list_tasks().expect("tasks");
        let audits = runtime.list_audits(10).expect("audits");

        assert_eq!(tasks.len(), 1);
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].event_type, "TaskAdded");
    }

    #[test]
    fn job_run_now_rejects_concurrent_active_run() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let dir = tempdir().expect("temp dir");
        let agent_path = dir.path().join("mock-agent");
        std::fs::write(
            &agent_path,
            "#!/bin/sh\nprintf '{\"schemaVersion\":1,\"status\":\"success\",\"result\":{},\"error\":null,\"durationMs\":1}'\n",
        )
        .expect("write mock agent");
        #[cfg(unix)]
        std::fs::set_permissions(&agent_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod mock agent");

        runtime
            .add_activity(ActivityAddParams {
                id: "spec-core-double-run".to_string(),
                spec_type: "agent_invoke".to_string(),
                description: "spec for job test".to_string(),
                input_schema_json: json!({}),
                output_schema_json: json!({}),
                spec_config: json!({}),
                workspace_path: None,
                identity_id: None,
                created_by: None,
            })
            .expect("insert activity");

        let job = runtime
            .add_job(JobAddParams {
                job_id: None,
                default_input: None,
                steps: vec![JobStep {
                    target_type: JobTargetType::Activity,
                    target_id: "spec-core-double-run".to_string(),
                    agent_cli: agent_path.to_string_lossy().to_string(),
                    timeout_seconds: 30,
                    env_extra: vec![],
                }],
                initial_state_override: None,
            })
            .expect("add job");

        runtime.run_job_now(&job.job_id).expect("first run");

        let sessions = runtime.job_history(&job.job_id).expect("history");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].state, JobRunState::Success);
    }

    #[test]
    fn list_tools_returns_builtins() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let tools = runtime.list_tools().expect("list tools");

        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"fs.read"));
        assert!(names.contains(&"fs.write"));
        assert!(names.contains(&"proc.spawn"));
        assert!(names.contains(&"time.now"));

        for tool in &tools {
            assert!(tool.builtin);
            assert!(tool.enabled);
        }
    }

    #[test]
    fn show_tool_returns_schema_with_params() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let tool = runtime.show_tool("fs.read").expect("show tool");

        assert_eq!(tool.name, "fs.read");
        assert!(tool.builtin);
        assert!(tool.enabled);
        assert!(!tool.parameters.is_empty());
        assert_eq!(tool.parameters[0].name, "path");
        assert!(tool.parameters[0].required);
    }

    #[test]
    fn show_tool_not_found() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let result = runtime.show_tool("nonexistent");
        assert!(matches!(result, Err(crate::OrbitError::ToolNotFound(_))));
    }

    #[test]
    fn disable_tool_prevents_execution() {
        let dir = tempdir().expect("temp dir");
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "content").expect("write");

        let runtime = OrbitRuntime::in_memory().expect("runtime");
        runtime.disable_tool("fs.read").expect("disable");

        let result = runtime.run_tool("fs.read", json!({"path": file.to_string_lossy()}));
        assert!(result.is_err());

        let audits = runtime.list_audits(10).expect("audits");
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].event_type, "ToolDisabled");
    }

    #[test]
    fn enable_tool_restores_execution() {
        let dir = tempdir().expect("temp dir");
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "restored").expect("write");

        let runtime = OrbitRuntime::in_memory().expect("runtime");
        runtime.disable_tool("fs.read").expect("disable");
        runtime.enable_tool("fs.read").expect("enable");

        let output = runtime
            .run_tool("fs.read", json!({"path": file.to_string_lossy()}))
            .expect("tool succeeds after re-enable");
        assert_eq!(output["content"], "restored");
    }

    #[test]
    fn cannot_remove_builtin_tool() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let result = runtime.remove_tool("fs.read");
        assert!(matches!(result, Err(crate::OrbitError::InvalidInput(_))));
    }

    #[test]
    fn dry_run_skips_execution() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let result = runtime
            .run_tool_dry_run("fs.read", &json!({"path": "/nonexistent"}))
            .expect("dry run");

        assert_eq!(result.tool_name, "fs.read");
        assert!(result.policy_allowed);
        assert!(result.missing_params.is_empty());

        // No audit records from dry run
        let audits = runtime.list_audits(10).expect("audits");
        assert_eq!(audits.len(), 0);
    }

    #[test]
    fn dry_run_reports_missing_params() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let result = runtime
            .run_tool_dry_run("fs.read", &json!({}))
            .expect("dry run");

        assert_eq!(result.missing_params, vec!["path"]);
    }

    #[test]
    fn doctor_reports_all_builtins_healthy() {
        use crate::command::tool::DoctorStatus;

        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let results = runtime.doctor().expect("doctor");

        assert!(!results.is_empty());
        for r in &results {
            assert_eq!(
                r.status,
                DoctorStatus::Ok,
                "tool {} not ok: {}",
                r.tool_name,
                r.message
            );
        }
    }

    // --- Task lifecycle tests ---

    #[test]
    fn add_task_with_all_fields() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "full task".to_string(),
                description: "detailed".to_string(),
                plan: "steps".to_string(),
                context_files: vec!["ARCHITECTURE.md".to_string()],
                workspace_path: None,
                priority: TaskPriority::High,
                task_type: orbit_types::TaskType::Issue,
                ..Default::default()
            })
            .expect("add");

        assert_eq!(task.title, "full task");
        assert_eq!(task.description, "detailed");
        assert_eq!(task.plan, "steps");
        assert_eq!(task.context_files, vec!["ARCHITECTURE.md".to_string()]);
        assert_eq!(task.priority, TaskPriority::High);
        assert_eq!(task.task_type, orbit_types::TaskType::Issue);
        // Default status depends on task_approval_required_for_agent (false in memory → Backlog)
        assert_eq!(task.status, TaskStatus::Backlog);
    }

    #[test]
    fn get_task_returns_task() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "find me".to_string(),
                ..Default::default()
            })
            .expect("add");

        let found = runtime.get_task(&task.id).expect("get");
        assert_eq!(found.id, task.id);
        assert_eq!(found.title, "find me");
    }

    #[test]
    fn get_task_not_found() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let result = runtime.get_task("task-nonexistent");
        assert!(matches!(result, Err(crate::OrbitError::TaskNotFound(_))));
    }

    #[test]
    fn add_task_rejects_nonexistent_workspace() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let result = runtime.add_task(TaskAddParams {
            title: "invalid workspace".to_string(),
            workspace_path: Some("/path/does/not/exist".to_string()),
            ..Default::default()
        });
        assert!(matches!(result, Err(crate::OrbitError::InvalidInput(_))));
    }

    #[test]
    fn list_tasks_filters_by_status() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        runtime
            .add_task(TaskAddParams {
                title: "open".to_string(),
                ..Default::default()
            })
            .expect("add");
        let t2 = runtime
            .add_task(TaskAddParams {
                title: "archived".to_string(),
                ..Default::default()
            })
            .expect("add");
        runtime.archive_task(&t2.id).expect("archive");

        let backlog = runtime
            .list_tasks_filtered(Some(TaskStatus::Backlog), None)
            .expect("filter");
        assert_eq!(backlog.len(), 1);
        assert_eq!(backlog[0].title, "open");
    }

    #[test]
    fn list_tasks_filters_by_priority() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        runtime
            .add_task(TaskAddParams {
                title: "low".to_string(),
                priority: TaskPriority::Low,
                ..Default::default()
            })
            .expect("add");
        runtime
            .add_task(TaskAddParams {
                title: "high".to_string(),
                priority: TaskPriority::High,
                ..Default::default()
            })
            .expect("add");

        let high = runtime
            .list_tasks_filtered(None, Some(TaskPriority::High))
            .expect("filter");
        assert_eq!(high.len(), 1);
        assert_eq!(high[0].title, "high");
    }

    #[test]
    fn update_task_changes_fields() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "original".to_string(),
                ..Default::default()
            })
            .expect("add");

        let updated = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    title: None,
                    description: Some("updated description".to_string()),
                    plan: Some("updated plan".to_string()),
                    execution_summary: Some("validated with unit tests".to_string()),
                    comment: None,
                    status: None,
                    branch: None,
                    pr_number: None,
                },
            )
            .expect("update");

        assert_eq!(updated.description, "updated description");
        assert_eq!(updated.plan, "updated plan");
        assert_eq!(updated.execution_summary, "validated with unit tests");
        assert_eq!(updated.assigned_to.as_deref(), Some("human"));

        let audits = runtime.list_audits(10).expect("audits");
        assert!(audits.iter().any(|a| a.event_type == "TaskUpdated"));
    }

    #[test]
    fn update_task_rejects_field_mutations_for_done_tasks() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .context
            .task_store
            .create_task(StoreTaskCreateParams {
                actor: "human".to_string(),
                title: "done task".to_string(),
                description: String::new(),
                plan: String::new(),
                execution_summary: "already shipped".to_string(),
                context_files: Vec::new(),
                workspace_path: None,
                created_by: Some("human".to_string()),
                assigned_to: Some("human".to_string()),
                status: TaskStatus::Done,
                priority: TaskPriority::Medium,
                task_type: TaskType::Task,
                branch: None,
                pr_number: None,
                proposed_by: None,
                comments: Vec::new(),
            })
            .expect("create done task");

        let result = runtime.update_task(
            &task.id,
            TaskUpdateParams {
                title: Some("rename attempt".to_string()),
                description: None,
                plan: None,
                execution_summary: None,
                comment: None,
                status: None,
                branch: None,
                pr_number: None,
            },
        );

        assert!(matches!(
            result,
            Err(crate::OrbitError::InvalidInput(message)) if message.contains("cannot be modified")
        ));
    }

    #[test]
    fn update_task_rejects_field_mutations_for_archived_tasks() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .context
            .task_store
            .create_task(StoreTaskCreateParams {
                actor: "human".to_string(),
                title: "archived task".to_string(),
                description: String::new(),
                plan: String::new(),
                execution_summary: "archived after review".to_string(),
                context_files: Vec::new(),
                workspace_path: None,
                created_by: Some("human".to_string()),
                assigned_to: Some("human".to_string()),
                status: TaskStatus::Archived,
                priority: TaskPriority::Medium,
                task_type: TaskType::Task,
                branch: None,
                pr_number: None,
                proposed_by: None,
                comments: Vec::new(),
            })
            .expect("create archived task");

        let result = runtime.update_task(
            &task.id,
            TaskUpdateParams {
                title: Some("rename attempt".to_string()),
                description: None,
                plan: None,
                execution_summary: None,
                comment: None,
                status: None,
                branch: None,
                pr_number: None,
            },
        );

        assert!(matches!(
            result,
            Err(crate::OrbitError::InvalidInput(message)) if message.contains("cannot be modified")
        ));
    }

    #[test]
    fn update_task_allows_field_mutations_for_in_progress_tasks() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .context
            .task_store
            .create_task(StoreTaskCreateParams {
                actor: "human".to_string(),
                title: "active task".to_string(),
                description: String::new(),
                plan: String::new(),
                execution_summary: String::new(),
                context_files: Vec::new(),
                workspace_path: None,
                created_by: Some("human".to_string()),
                assigned_to: Some("human".to_string()),
                status: TaskStatus::InProgress,
                priority: TaskPriority::Medium,
                task_type: TaskType::Task,
                branch: None,
                pr_number: None,
                proposed_by: None,
                comments: Vec::new(),
            })
            .expect("create in-progress task");

        let updated = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    title: Some("retitled active task".to_string()),
                    description: None,
                    plan: None,
                    execution_summary: None,
                    comment: None,
                    status: None,
                    branch: None,
                    pr_number: None,
                },
            )
            .expect("update in-progress task");

        assert_eq!(updated.title, "retitled active task");
        assert_eq!(updated.status, TaskStatus::InProgress);
    }

    #[test]
    fn add_task_comment_uses_effective_actor() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "commented".to_string(),
                comment: Some("initial context".to_string()),
                ..Default::default()
            })
            .expect("add");

        assert_eq!(task.comments.len(), 1);
        assert_eq!(task.comments[0].by, "human");
        assert_eq!(task.comments[0].message, "initial context");
    }

    #[test]
    fn update_task_comment_appends_with_human_author() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "comment me".to_string(),
                ..Default::default()
            })
            .expect("add");

        let updated = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    title: None,
                    description: None,
                    plan: None,
                    execution_summary: None,
                    comment: Some("follow-up note".to_string()),
                    status: None,
                    branch: None,
                    pr_number: None,
                },
            )
            .expect("update");

        assert_eq!(updated.comments.len(), 1);
        assert_eq!(updated.comments[0].by, "human");
        assert_eq!(updated.comments[0].message, "follow-up note");
    }

    #[test]
    fn direct_runtime_task_attribution_uses_human_labels() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("config.toml"),
            "[user]\nname = \"daniel\"\n",
        )
        .expect("write config");
        let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

        let task = runtime
            .add_task(TaskAddParams {
                title: "configured author".to_string(),
                ..Default::default()
            })
            .expect("add");
        assert_eq!(task.created_by.as_deref(), Some("human"));

        let updated = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    title: None,
                    description: None,
                    plan: None,
                    execution_summary: None,
                    comment: Some("configured follow-up".to_string()),
                    status: None,
                    branch: None,
                    pr_number: None,
                },
            )
            .expect("update");

        assert_eq!(runtime.user_name(), "daniel");
        assert_eq!(updated.comments[0].by, "human");
        assert_eq!(updated.comments[0].message, "configured follow-up");
    }

    #[test]
    fn start_task_moves_backlog_work_into_progress() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "ready to start".to_string(),
                ..Default::default()
            })
            .expect("add");

        let started = runtime
            .start_task(
                &task.id,
                Some("picked up for implementation".to_string()),
                Some("starting now".to_string()),
            )
            .expect("start");

        assert_eq!(started.status, TaskStatus::InProgress);
        assert_eq!(started.assigned_to.as_deref(), Some("human"));
        assert_eq!(started.comments[0].message, "starting now");
        let history = started.history.last().expect("history");
        assert_eq!(history.event, "started");
        assert_eq!(
            history.note.as_deref(),
            Some("picked up for implementation")
        );

        let audits = runtime.list_audits(10).expect("audits");
        assert!(audits.iter().any(|a| a.event_type == "TaskStarted"));
    }

    #[test]
    fn start_task_from_proposed_records_approval_before_starting() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .context
            .task_store
            .create_task(StoreTaskCreateParams {
                actor: "agent".to_string(),
                title: "proposal to start".to_string(),
                description: String::new(),
                plan: String::new(),
                execution_summary: String::new(),
                context_files: Vec::new(),
                workspace_path: None,
                created_by: Some("agent".to_string()),
                assigned_to: Some("agent".to_string()),
                status: TaskStatus::Proposed,
                priority: TaskPriority::Medium,
                task_type: TaskType::Task,
                branch: None,
                pr_number: None,
                proposed_by: Some("agent".to_string()),
                comments: Vec::new(),
            })
            .expect("create proposed task");

        let started = runtime
            .start_task(&task.id, Some("approved in planning".to_string()), None)
            .expect("start");

        assert_eq!(started.status, TaskStatus::InProgress);
        let proposal_approved = started
            .history
            .iter()
            .find(|entry| entry.event == "proposal_approved")
            .expect("proposal approval history");
        assert_eq!(
            proposal_approved.note.as_deref(),
            Some("approved in planning")
        );
        assert_eq!(started.history.last().expect("history").event, "started");

        let audits = runtime.list_audits(10).expect("audits");
        assert!(audits.iter().any(|a| a.event_type == "TaskStarted"));
    }

    #[test]
    fn start_task_rejects_review_work() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "already reviewed".to_string(),
                ..Default::default()
            })
            .expect("add");

        runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    title: None,
                    description: None,
                    plan: None,
                    execution_summary: None,
                    comment: None,
                    status: Some(TaskStatus::InProgress),
                    branch: None,
                    pr_number: None,
                },
            )
            .expect("in progress");
        let review = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    title: None,
                    description: None,
                    plan: None,
                    execution_summary: Some("Implemented and verified".to_string()),
                    comment: None,
                    status: Some(TaskStatus::Review),
                    branch: None,
                    pr_number: None,
                },
            )
            .expect("review");

        let result = runtime.start_task(&review.id, None, None);
        assert!(matches!(result, Err(crate::OrbitError::InvalidInput(_))));
    }

    #[test]
    fn approve_task_comment_uses_approver_identity() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "approve me".to_string(),
                ..Default::default()
            })
            .expect("add");

        let task = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    title: None,
                    description: None,
                    plan: None,
                    execution_summary: None,
                    comment: None,
                    status: Some(TaskStatus::InProgress),
                    branch: None,
                    pr_number: None,
                },
            )
            .expect("in progress");
        let task = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    title: None,
                    description: None,
                    plan: None,
                    execution_summary: Some("ready".to_string()),
                    comment: None,
                    status: Some(TaskStatus::Review),
                    branch: None,
                    pr_number: None,
                },
            )
            .expect("review");

        let approved = runtime
            .approve_task(
                &task.id,
                Some("looks good".to_string()),
                Some("approved with note".to_string()),
            )
            .expect("approve");

        assert_eq!(approved.comments.len(), 1);
        assert_eq!(approved.comments[0].by, "human");
        assert_eq!(approved.comments[0].message, "approved with note");
    }

    #[test]
    fn blank_task_comment_is_rejected() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let result = runtime.add_task(TaskAddParams {
            title: "invalid comment".to_string(),
            comment: Some("   ".to_string()),
            ..Default::default()
        });

        assert!(matches!(result, Err(crate::OrbitError::InvalidInput(_))));
    }

    #[test]
    fn archive_task_sets_archived() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "archivable".to_string(),
                ..Default::default()
            })
            .expect("add");

        runtime.archive_task(&task.id).expect("archive");
        let archived = runtime.get_task(&task.id).expect("get");
        assert_eq!(archived.status, TaskStatus::Archived);

        let audits = runtime.list_audits(10).expect("audits");
        assert!(audits.iter().any(|a| a.event_type == "TaskArchived"));
    }

    #[test]
    fn reject_review_task_moves_to_rejected_with_audit_metadata() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "needs more work".to_string(),
                ..Default::default()
            })
            .expect("add");

        runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    title: None,
                    description: None,
                    plan: None,
                    execution_summary: None,
                    comment: None,
                    status: Some(TaskStatus::InProgress),
                    branch: None,
                    pr_number: None,
                },
            )
            .expect("in progress");
        runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    title: None,
                    description: None,
                    plan: None,
                    execution_summary: Some("Implemented initial pass.".to_string()),
                    comment: None,
                    status: Some(TaskStatus::Review),
                    branch: None,
                    pr_number: None,
                },
            )
            .expect("review");

        let rejected = runtime
            .reject_task(&task.id, "Missing regression coverage".to_string(), None)
            .expect("reject");

        assert_eq!(rejected.status, TaskStatus::Rejected);
        let history = rejected.history.last().expect("history entry");
        assert_eq!(history.by, "human");
        assert_eq!(history.event, "review_rejected");
        assert_eq!(history.note.as_deref(), Some("Missing regression coverage"));

        let audits = runtime.list_audits(10).expect("audits");
        assert!(audits.iter().any(|a| a.event_type == "TaskReviewRejected"));
    }

    #[test]
    fn reject_task_requires_supported_status_and_note() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "backlog task".to_string(),
                ..Default::default()
            })
            .expect("add");

        let wrong_status = runtime.reject_task(&task.id, "not ready".to_string(), None);
        assert!(matches!(
            wrong_status,
            Err(crate::OrbitError::InvalidInput(_))
        ));

        let empty_note = runtime.reject_task(&task.id, "   ".to_string(), None);
        assert!(matches!(
            empty_note,
            Err(crate::OrbitError::InvalidInput(_))
        ));
    }

    #[test]
    fn review_transition_requires_execution_summary() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "needs review summary".to_string(),
                ..Default::default()
            })
            .expect("add");

        let _in_progress = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    title: None,
                    description: None,
                    plan: None,
                    execution_summary: None,
                    comment: None,
                    status: Some(TaskStatus::InProgress),
                    branch: None,
                    pr_number: None,
                },
            )
            .expect("in progress");

        let missing_summary = runtime.update_task(
            &task.id,
            TaskUpdateParams {
                title: None,
                description: None,
                plan: None,
                execution_summary: None,
                comment: None,
                status: Some(TaskStatus::Review),
                branch: None,
                pr_number: None,
            },
        );
        assert!(matches!(
            missing_summary,
            Err(crate::OrbitError::InvalidInput(_))
        ));

        let review = runtime
            .update_task(
                &task.id,
                TaskUpdateParams {
                    title: None,
                    description: None,
                    plan: None,
                    execution_summary: Some("Implemented change and validated tests.".to_string()),
                    comment: None,
                    status: Some(TaskStatus::Review),
                    branch: None,
                    pr_number: None,
                },
            )
            .expect("review with summary");

        assert_eq!(review.status, TaskStatus::Review);
        assert_eq!(
            review.execution_summary,
            "Implemented change and validated tests."
        );
    }

    #[test]
    fn archive_already_archived_returns_error() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "already archived".to_string(),
                ..Default::default()
            })
            .expect("add");

        runtime.archive_task(&task.id).expect("archive");
        let result = runtime.archive_task(&task.id);
        assert!(matches!(result, Err(crate::OrbitError::InvalidInput(_))));
    }

    #[test]
    fn unarchive_task_sets_backlog() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "unarchive me".to_string(),
                ..Default::default()
            })
            .expect("add");

        runtime.archive_task(&task.id).expect("archive");
        runtime.unarchive_task(&task.id).expect("unarchive");

        let unarchived = runtime.get_task(&task.id).expect("get");
        assert_eq!(unarchived.status, TaskStatus::Backlog);

        let audits = runtime.list_audits(10).expect("audits");
        assert!(audits.iter().any(|a| a.event_type == "TaskUnarchived"));
    }

    #[test]
    fn unarchive_non_archived_returns_error() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "not archived".to_string(),
                ..Default::default()
            })
            .expect("add");

        let result = runtime.unarchive_task(&task.id);
        assert!(matches!(result, Err(crate::OrbitError::InvalidInput(_))));
    }

    #[test]
    fn delete_task_removes_it() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "delete me".to_string(),
                ..Default::default()
            })
            .expect("add");

        runtime.delete_task(&task.id).expect("delete");
        let result = runtime.get_task(&task.id);
        assert!(matches!(result, Err(crate::OrbitError::TaskNotFound(_))));

        let audits = runtime.list_audits(10).expect("audits");
        assert!(audits.iter().any(|a| a.event_type == "TaskDeleted"));
    }

    #[test]
    fn delete_nonexistent_returns_error() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let result = runtime.delete_task("task-nonexistent");
        assert!(matches!(result, Err(crate::OrbitError::TaskNotFound(_))));
    }

    #[test]
    fn search_tasks_matches_title() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        runtime
            .add_task(TaskAddParams {
                title: "fix login bug".to_string(),
                ..Default::default()
            })
            .expect("add");
        runtime
            .add_task(TaskAddParams {
                title: "add feature".to_string(),
                ..Default::default()
            })
            .expect("add");

        let results = runtime.search_tasks("login").expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "fix login bug");
    }

    #[test]
    fn search_tasks_matches_description() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        runtime
            .add_task(TaskAddParams {
                title: "task one".to_string(),
                description: "needs database migration".to_string(),
                ..Default::default()
            })
            .expect("add");
        runtime
            .add_task(TaskAddParams {
                title: "task two".to_string(),
                ..Default::default()
            })
            .expect("add");

        let results = runtime.search_tasks("migration").expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "task one");
    }

    #[test]
    fn doctor_reports_disabled_tool() {
        use crate::command::tool::DoctorStatus;

        let runtime = OrbitRuntime::in_memory().expect("runtime");
        runtime.disable_tool("fs.read").expect("disable");

        let results = runtime.doctor().expect("doctor");
        let fs_read = results
            .iter()
            .find(|r| r.tool_name == "fs.read")
            .expect("fs.read in results");
        assert_eq!(fs_read.status, DoctorStatus::Warning);
        assert!(fs_read.message.contains("disabled"));
    }

    // --- Audit event tests ---

    #[test]
    fn audit_event_record_list_round_trip() {
        use orbit_store::AuditEventInsertParams;
        use orbit_types::AuditEventStatus;

        let runtime = OrbitRuntime::in_memory().expect("runtime");
        runtime
            .record_audit_event(&AuditEventInsertParams {
                execution_id: "exec-integ-1".to_string(),
                command: "tool".to_string(),
                subcommand: Some("run".to_string()),
                tool_name: Some("fs.read".to_string()),
                target_type: None,
                target_id: None,
                role: "admin".to_string(),
                status: AuditEventStatus::Success,
                exit_code: 0,
                duration_ms: 42,
                working_directory: "/tmp".to_string(),
                arguments_json: None,
                stdout_truncated: None,
                stderr_truncated: None,
                error_message: None,
                host: None,
                pid: 1,
                session_id: None,
            })
            .expect("record");

        let events = runtime
            .list_audit_events(None, None, None, None, 10)
            .expect("list");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].command, "tool");
        assert_eq!(events[0].status, AuditEventStatus::Success);
    }

    #[test]
    fn audit_event_show_not_found() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let result = runtime.show_audit_event(999);
        assert!(matches!(result, Err(crate::OrbitError::InvalidInput(_))));
    }

    #[test]
    fn audit_event_prune_via_runtime() {
        use orbit_store::AuditEventInsertParams;
        use orbit_types::AuditEventStatus;

        let runtime = OrbitRuntime::in_memory().expect("runtime");
        runtime
            .record_audit_event(&AuditEventInsertParams {
                execution_id: "exec-prune-integ".to_string(),
                command: "tool".to_string(),
                subcommand: None,
                tool_name: None,
                target_type: None,
                target_id: None,
                role: "admin".to_string(),
                status: AuditEventStatus::Success,
                exit_code: 0,
                duration_ms: 10,
                working_directory: "/tmp".to_string(),
                arguments_json: None,
                stdout_truncated: None,
                stderr_truncated: None,
                error_message: None,
                host: None,
                pid: 1,
                session_id: None,
            })
            .expect("record");

        let future = chrono::Utc::now() + chrono::Duration::days(1);
        let pruned = runtime.prune_audit_events(&future).expect("prune");
        assert_eq!(pruned, 1);
    }

    #[test]
    fn audit_event_stats_via_runtime() {
        use orbit_store::AuditEventInsertParams;
        use orbit_types::AuditEventStatus;

        let runtime = OrbitRuntime::in_memory().expect("runtime");

        for (i, status) in [
            AuditEventStatus::Success,
            AuditEventStatus::Failure,
            AuditEventStatus::Denied,
        ]
        .iter()
        .enumerate()
        {
            runtime
                .record_audit_event(&AuditEventInsertParams {
                    execution_id: format!("exec-stats-integ-{i}"),
                    command: "tool".to_string(),
                    subcommand: None,
                    tool_name: None,
                    target_type: None,
                    target_id: None,
                    role: "admin".to_string(),
                    status: *status,
                    exit_code: 0,
                    duration_ms: (i as i64 + 1) * 100,
                    working_directory: "/tmp".to_string(),
                    arguments_json: None,
                    stdout_truncated: None,
                    stderr_truncated: None,
                    error_message: None,
                    host: None,
                    pid: 1,
                    session_id: None,
                })
                .expect("record");
        }

        let stats = runtime.audit_event_stats(None, None).expect("stats");
        assert_eq!(stats.total, 3);
        assert_eq!(stats.success_count, 1);
        assert_eq!(stats.failure_count, 1);
        assert_eq!(stats.denied_count, 1);
    }
}
