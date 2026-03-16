use std::sync::{Mutex, OnceLock};

use orbit_core::OrbitRuntime;
use orbit_core::command::task::TaskAddParams;
use orbit_types::AgentSessionStatus;
use tempfile::tempdir;

const ORBIT_TASK_ACTOR_KIND: &str = "ORBIT_TASK_ACTOR_KIND";
const ORBIT_TASK_ACTOR_IDENTITY_ID: &str = "ORBIT_TASK_ACTOR_IDENTITY_ID";

fn with_task_actor_env<T>(
    actor_kind: Option<&str>,
    identity_id: Option<&str>,
    f: impl FnOnce() -> T,
) -> T {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct EnvReset {
        previous_kind: Option<String>,
        previous_identity_id: Option<String>,
    }

    impl Drop for EnvReset {
        fn drop(&mut self) {
            unsafe {
                match &self.previous_kind {
                    Some(value) => std::env::set_var(ORBIT_TASK_ACTOR_KIND, value),
                    None => std::env::remove_var(ORBIT_TASK_ACTOR_KIND),
                }
                match &self.previous_identity_id {
                    Some(value) => std::env::set_var(ORBIT_TASK_ACTOR_IDENTITY_ID, value),
                    None => std::env::remove_var(ORBIT_TASK_ACTOR_IDENTITY_ID),
                }
            }
        }
    }

    let _guard = ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock");
    let previous_kind = std::env::var(ORBIT_TASK_ACTOR_KIND).ok();
    let previous_identity_id = std::env::var(ORBIT_TASK_ACTOR_IDENTITY_ID).ok();
    unsafe {
        match actor_kind {
            Some(value) => std::env::set_var(ORBIT_TASK_ACTOR_KIND, value),
            None => std::env::remove_var(ORBIT_TASK_ACTOR_KIND),
        }
        match identity_id {
            Some(value) => std::env::set_var(ORBIT_TASK_ACTOR_IDENTITY_ID, value),
            None => std::env::remove_var(ORBIT_TASK_ACTOR_IDENTITY_ID),
        }
    }
    let _reset = EnvReset {
        previous_kind,
        previous_identity_id,
    };
    f()
}

fn with_agent_task_actor<T>(f: impl FnOnce() -> T) -> T {
    with_task_actor_env(Some("agent"), None, f)
}

fn with_human_task_actor<T>(f: impl FnOnce() -> T) -> T {
    with_task_actor_env(None, None, f)
}

fn session_id_from_audits(audits: &[orbit_types::Audit]) -> Option<String> {
    audits.iter().find_map(|audit| {
        if audit.event_type != "AgentSessionStarted" {
            return None;
        }
        audit.payload["data"]["session_id"]
            .as_str()
            .map(str::to_string)
    })
}

#[test]
fn agent_run_executes_sequentially_and_stops_on_first_failure() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let ok_file = dir.path().join("ok.txt");
    std::fs::write(&ok_file, "hello").expect("write fixture");

    let task = runtime
        .add_task(TaskAddParams {
            title: "agent".to_string(),
            plan: format!(
                r#"{{
                  "tool_calls": [
                    {{"name":"fs.read","input":{{"path":"{}"}}}},
                    {{"name":"fs.read","input":{{"path":"{}"}}}},
                    {{"name":"time.now","input":{{}}}}
                  ]
                }}"#,
                ok_file.to_string_lossy(),
                dir.path().join("missing.txt").to_string_lossy()
            ),
            ..Default::default()
        })
        .expect("task");

    let result = runtime.run_agent_task(&task.id);
    assert!(result.is_err(), "second call should fail and stop session");

    let audits = runtime.list_audits(50).expect("audits");
    let session_id = session_id_from_audits(&audits).expect("session id from audits");
    let session = runtime
        .get_agent_session(&session_id)
        .expect("get session")
        .expect("session exists");

    assert_eq!(session.status, AgentSessionStatus::Failed);
    assert_eq!(session.tool_calls.len(), 2, "third call should not execute");
    assert!(session.tool_calls[0].success);
    assert!(!session.tool_calls[1].success);

    let audits = runtime.list_audits(50).expect("audits");
    assert!(
        audits
            .iter()
            .any(|a| a.event_type == "AgentSessionCompleted"),
        "failed sessions should record completion event"
    );
}

#[test]
fn successful_agent_run_records_session_and_audits() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let task = runtime
        .add_task(TaskAddParams {
            title: "agent success".to_string(),
            plan: r#"{"tool_calls":[{"name":"time.now","input":{}}]}"#.to_string(),
            ..Default::default()
        })
        .expect("task");

    let result = runtime.run_agent_task(&task.id).expect("run");
    assert_eq!(result.status, AgentSessionStatus::Completed);
    assert_eq!(result.tool_calls_executed, 1);

    let session = runtime
        .get_agent_session(&result.session_id)
        .expect("get session")
        .expect("session exists");
    assert_eq!(session.status, AgentSessionStatus::Completed);
    assert_eq!(session.tool_calls.len(), 1);
    assert!(session.tool_calls[0].success);

    let audits = runtime.list_audits(20).expect("audits");
    assert!(
        audits.iter().any(|a| a.event_type == "AgentSessionStarted"),
        "session start should be audited"
    );
    assert!(
        audits.iter().any(|a| a.event_type == "AgentToolCall"),
        "tool calls should be audited with session metadata"
    );
    assert!(
        audits
            .iter()
            .any(|a| a.event_type == "AgentSessionCompleted"),
        "session completion should be audited"
    );
}

#[test]
fn agent_run_requires_approval_when_config_enabled() {
    let dir = tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");

    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let task = with_agent_task_actor(|| {
        runtime
            .add_task(TaskAddParams {
                title: "agent gated".to_string(),
                plan: r#"{"tool_calls":[{"name":"time.now","input":{}}]}"#.to_string(),
                ..Default::default()
            })
            .expect("task")
    });

    let result = runtime.run_agent_task(&task.id);
    assert!(matches!(
        result,
        Err(orbit_types::OrbitError::TaskApprovalRequired(_))
    ));
}

#[test]
fn agent_run_succeeds_after_explicit_approval() {
    let dir = tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("config.toml"),
        "[task.approval]\nrequired_for_agent = true\n",
    )
    .expect("write config");

    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let task = with_agent_task_actor(|| {
        runtime
            .add_task(TaskAddParams {
                title: "agent after approval".to_string(),
                plan: r#"{"tool_calls":[{"name":"time.now","input":{}}]}"#.to_string(),
                ..Default::default()
            })
            .expect("task")
    });

    // Task starts as proposed, so agent should fail
    let result = runtime.run_agent_task(&task.id);
    assert!(matches!(
        result,
        Err(orbit_types::OrbitError::TaskApprovalRequired(_))
    ));

    // Approve the task (proposed → backlog)
    with_human_task_actor(|| {
        runtime
            .approve_task(&task.id, Some("looks good".to_string()), None)
            .expect("approve");
    });

    // Now agent should succeed
    let result = runtime.run_agent_task(&task.id).expect("run");
    assert_eq!(result.status, AgentSessionStatus::Completed);

    let approved_task = runtime.get_task(&task.id).expect("task");
    let history = approved_task.history.last().expect("history entry");
    assert_eq!(history.by, "human");
    assert_eq!(history.event, "proposal_approved");
    assert_eq!(history.note.as_deref(), Some("looks good"));
}
