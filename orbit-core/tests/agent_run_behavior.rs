use orbit_core::OrbitRuntime;
use orbit_core::command::task::TaskAddParams;
use orbit_types::AgentSessionStatus;
use tempfile::tempdir;

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
    let task = runtime
        .add_task(TaskAddParams {
            title: "agent gated".to_string(),
            plan: r#"{"tool_calls":[{"name":"time.now","input":{}}]}"#.to_string(),
            ..Default::default()
        })
        .expect("task");

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
    let task = runtime
        .add_task(TaskAddParams {
            title: "agent after approval".to_string(),
            plan: r#"{"tool_calls":[{"name":"time.now","input":{}}]}"#.to_string(),
            ..Default::default()
        })
        .expect("task");

    // Task starts as proposed, so agent should fail
    let result = runtime.run_agent_task(&task.id);
    assert!(matches!(
        result,
        Err(orbit_types::OrbitError::TaskApprovalRequired(_))
    ));

    // Approve the task (proposed → backlog)
    runtime
        .approve_task(&task.id, "human-reviewer", Some("looks good".to_string()))
        .expect("approve");

    // Now agent should succeed
    let result = runtime.run_agent_task(&task.id).expect("run");
    assert_eq!(result.status, AgentSessionStatus::Completed);

    let approved_task = runtime.get_task(&task.id).expect("task");
    assert_eq!(
        approved_task.proposal_approved_by.as_deref(),
        Some("human-reviewer")
    );
    assert_eq!(
        approved_task.proposal_decision_note.as_deref(),
        Some("looks good")
    );
}
