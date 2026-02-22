use orbit_core::OrbitRuntime;
use orbit_core::command::skill::SkillAddParams;
use orbit_core::command::task::TaskAddParams;
use orbit_types::{AgentSessionStatus, AuthorType, EntityType, EntryType, Role};
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
            instructions: format!(
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

    runtime
        .add_skill(SkillAddParams {
            name: "read-only".to_string(),
            description: None,
            instructions: "read only".to_string(),
            context_files: vec![],
            allowed_tools: vec!["fs.read".to_string()],
            role: Role::Agent,
        })
        .expect("skill");
    runtime
        .attach_skill_to_task(&task.id, "read-only")
        .expect("attach");

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

    let entries = runtime
        .list_entries(EntityType::Session, &session_id)
        .expect("session entries");
    assert!(
        entries
            .iter()
            .any(|entry| entry.entry_type == EntryType::Reasoning),
        "failed sessions should include reasoning entries"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.entry_type == EntryType::System),
        "failed sessions should include system entries"
    );
}

#[test]
fn successful_agent_run_records_session_and_audits() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let task = runtime
        .add_task(TaskAddParams {
            title: "agent success".to_string(),
            instructions: r#"{"tool_calls":[{"name":"time.now","input":{}}]}"#.to_string(),
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

    let entries = runtime
        .list_entries(EntityType::Session, &result.session_id)
        .expect("session entries");
    assert!(
        entries
            .iter()
            .any(|entry| entry.entry_type == EntryType::Reasoning),
        "successful sessions should include reasoning entries"
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.entry_type == EntryType::System),
        "successful sessions should include system entries"
    );
    assert!(
        entries
            .iter()
            .all(|entry| { entry.session_id.as_deref() == Some(result.session_id.as_str()) })
    );
    assert!(entries.iter().any(|entry| {
        entry.entry_type == EntryType::Reasoning
            && entry.author_type == AuthorType::Agent
            && entry.author_model.is_some()
    }));

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
