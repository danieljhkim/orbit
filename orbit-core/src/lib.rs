pub mod command;
pub mod context;
pub mod job;
pub mod runtime;
pub mod watch;

pub use context::OrbitContext;
pub use orbit_types::OrbitError;
pub use runtime::OrbitRuntime;

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use orbit_policy::PolicyEngine;
    use orbit_types::{JobStatus, OrbitEvent};
    use serde_json::json;
    use tempfile::tempdir;

    use crate::OrbitRuntime;

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
        let _ = runtime.add_task("ship orbit").expect("add task");

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
}
