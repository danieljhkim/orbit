pub mod agent;
pub mod command;
pub mod context;
pub mod job;
pub mod runtime;
pub mod watch;

pub use context::OrbitContext;
pub use orbit_types::OrbitError;
pub use orbit_types::{
    AgentSessionStatus, AuthorType, EntityType, Entry, EntryType, Role, Skill, Task, TaskPriority,
    TaskStatus, TaskType,
};
pub use runtime::OrbitRuntime;

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use orbit_policy::PolicyEngine;
    use orbit_types::{JobStatus, OrbitEvent, TaskPriority, TaskStatus, TaskType};
    use serde_json::json;
    use tempfile::tempdir;

    use crate::OrbitRuntime;
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

        let events = runtime.event_bus.snapshot();
        assert!(matches!(
            events.first(),
            Some(OrbitEvent::ToolExecuted { name }) if name == "fs.read"
        ));
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
    fn job_run_does_not_double_execute_due_job() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let now = Utc::now();
        let job = runtime
            .schedule_job("demo", "true", now)
            .expect("schedule job");

        let first = runtime.run_due_jobs(now).expect("first run");
        let second = runtime.run_due_jobs(now).expect("second run");

        assert_eq!(first, 1);
        assert_eq!(second, 0);

        let status = runtime
            .job_status(&job.id)
            .expect("status")
            .expect("present");
        assert_eq!(status, JobStatus::Complete);
    }

    #[test]
    fn job_run_skips_when_global_lock_held() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        assert!(
            runtime
                .context
                .store
                .try_lock(orbit_store::Store::global_job_lock_name())
                .expect("lock")
        );

        let ran = runtime.run_jobs().expect("run jobs");
        assert_eq!(ran, 0);

        let _ = runtime
            .context
            .store
            .unlock(orbit_store::Store::global_job_lock_name());
    }

    #[test]
    fn watch_debounce_coalesces_burst_events() {
        let mut d = crate::watch::DebounceQueueOne::new(100);
        let first = d.on_event("a.txt".to_string(), 0);
        let second = d.on_event("b.txt".to_string(), 10);
        let third = d.on_event("c.txt".to_string(), 20);

        assert_eq!(first.as_deref(), Some("a.txt"));
        assert!(second.is_none());
        assert!(third.is_none());

        assert!(d.on_tick(50).is_none());
        assert_eq!(d.on_tick(100).as_deref(), Some("c.txt"));
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
                instructions: "steps".to_string(),
                context_files: vec!["ARCHITECTURE.md".to_string()],
                priority: TaskPriority::High,
                task_type: TaskType::Bug,
                owner: "alice".to_string(),
                parent_id: None,
            })
            .expect("add");

        assert_eq!(task.title, "full task");
        assert_eq!(task.description, "detailed");
        assert_eq!(task.instructions, "steps");
        assert_eq!(task.context_files, vec!["ARCHITECTURE.md".to_string()]);
        assert_eq!(task.priority, TaskPriority::High);
        assert_eq!(task.task_type, TaskType::Bug);
        assert_eq!(task.owner, "alice");
        assert_eq!(task.status, TaskStatus::Todo);
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
                title: "closed".to_string(),
                ..Default::default()
            })
            .expect("add");
        runtime.close_task(&t2.id).expect("close");

        let todos = runtime
            .list_tasks_filtered(Some(TaskStatus::Todo), None)
            .expect("filter");
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].title, "open");
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
                    title: Some("changed".to_string()),
                    description: Some("new desc".to_string()),
                    instructions: None,
                    context_files: None,
                    status: None,
                    priority: Some(TaskPriority::High),
                    task_type: None,
                    owner: Some("bob".to_string()),
                    parent_id: None,
                },
            )
            .expect("update");

        assert_eq!(updated.title, "changed");
        assert_eq!(updated.description, "new desc");
        assert_eq!(updated.priority, TaskPriority::High);
        assert_eq!(updated.owner, "bob");

        let audits = runtime.list_audits(10).expect("audits");
        assert!(audits.iter().any(|a| a.event_type == "TaskUpdated"));
    }

    #[test]
    fn close_task_sets_done() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "closable".to_string(),
                ..Default::default()
            })
            .expect("add");

        runtime.close_task(&task.id).expect("close");
        let closed = runtime.get_task(&task.id).expect("get");
        assert_eq!(closed.status, TaskStatus::Done);

        let audits = runtime.list_audits(10).expect("audits");
        assert!(audits.iter().any(|a| a.event_type == "TaskClosed"));
    }

    #[test]
    fn close_already_done_returns_error() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "already done".to_string(),
                ..Default::default()
            })
            .expect("add");

        runtime.close_task(&task.id).expect("close");
        let result = runtime.close_task(&task.id);
        assert!(matches!(result, Err(crate::OrbitError::InvalidInput(_))));
    }

    #[test]
    fn reopen_task_sets_todo() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "reopen me".to_string(),
                ..Default::default()
            })
            .expect("add");

        runtime.close_task(&task.id).expect("close");
        runtime.reopen_task(&task.id).expect("reopen");

        let reopened = runtime.get_task(&task.id).expect("get");
        assert_eq!(reopened.status, TaskStatus::Todo);

        let audits = runtime.list_audits(10).expect("audits");
        assert!(audits.iter().any(|a| a.event_type == "TaskReopened"));
    }

    #[test]
    fn reopen_non_closed_returns_error() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let task = runtime
            .add_task(TaskAddParams {
                title: "not closed".to_string(),
                ..Default::default()
            })
            .expect("add");

        let result = runtime.reopen_task(&task.id);
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
    fn add_task_with_parent_validates_parent_exists() {
        let runtime = OrbitRuntime::in_memory().expect("runtime");
        let result = runtime.add_task(TaskAddParams {
            title: "child".to_string(),
            parent_id: Some("task-nonexistent".to_string()),
            ..Default::default()
        });
        assert!(matches!(result, Err(crate::OrbitError::TaskNotFound(_))));
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
}
