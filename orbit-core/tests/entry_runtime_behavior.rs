use orbit_core::OrbitRuntime;
use orbit_core::command::entry::EntryAddParams;
use orbit_core::command::task::TaskAddParams;
use orbit_types::{AuthorType, EntityType, EntryType, OrbitError, OrbitEvent};
use tempfile::tempdir;

fn add_task(runtime: &OrbitRuntime, title: &str) -> String {
    runtime
        .add_task(TaskAddParams {
            title: title.to_string(),
            ..Default::default()
        })
        .expect("add task")
        .id
}

#[test]
fn reject_empty_body() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let task_id = add_task(&runtime, "entry");

    let result = runtime.add_entry(EntryAddParams {
        entity_type: EntityType::Task,
        entity_id: task_id,
        session_id: None,
        entry_type: EntryType::Comment,
        author_type: AuthorType::Human,
        author_id: "daniel".to_string(),
        author_model: None,
        body: "   ".to_string(),
    });

    assert!(matches!(result, Err(OrbitError::EntryValidation(_))));
}

#[test]
fn require_author_model_when_author_type_agent() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let task_id = add_task(&runtime, "entry");

    let result = runtime.add_entry(EntryAddParams {
        entity_type: EntityType::Task,
        entity_id: task_id,
        session_id: None,
        entry_type: EntryType::Reasoning,
        author_type: AuthorType::Agent,
        author_id: "orbit-agent".to_string(),
        author_model: None,
        body: "summary".to_string(),
    });

    assert!(matches!(result, Err(OrbitError::EntryValidation(_))));
}

#[test]
fn session_entity_requires_matching_session_id() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let task = runtime
        .add_task(TaskAddParams {
            title: "agent".to_string(),
            instructions: r#"{"tool_calls":[{"name":"time.now","input":{}}]}"#.to_string(),
            ..Default::default()
        })
        .expect("task");
    let run = runtime.run_agent_task(&task.id).expect("run");

    let missing_session = runtime.add_entry(EntryAddParams {
        entity_type: EntityType::Session,
        entity_id: run.session_id.clone(),
        session_id: None,
        entry_type: EntryType::Comment,
        author_type: AuthorType::Human,
        author_id: "daniel".to_string(),
        author_model: None,
        body: "note".to_string(),
    });
    assert!(matches!(
        missing_session,
        Err(OrbitError::EntryValidation(_))
    ));

    let mismatched_session = runtime.add_entry(EntryAddParams {
        entity_type: EntityType::Session,
        entity_id: run.session_id.clone(),
        session_id: Some("session-other".to_string()),
        entry_type: EntryType::Comment,
        author_type: AuthorType::Human,
        author_id: "daniel".to_string(),
        author_model: None,
        body: "note".to_string(),
    });
    assert!(matches!(
        mismatched_session,
        Err(OrbitError::EntryValidation(_))
    ));
}

#[test]
fn reject_nonexistent_entities_and_unsupported_workflow() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let missing_task = runtime.add_entry(EntryAddParams {
        entity_type: EntityType::Task,
        entity_id: "task-missing".to_string(),
        session_id: None,
        entry_type: EntryType::Comment,
        author_type: AuthorType::Human,
        author_id: "daniel".to_string(),
        author_model: None,
        body: "note".to_string(),
    });
    assert!(matches!(missing_task, Err(OrbitError::EntryValidation(_))));

    let workflow = runtime.add_entry(EntryAddParams {
        entity_type: EntityType::Workflow,
        entity_id: "workflow-1".to_string(),
        session_id: None,
        entry_type: EntryType::Comment,
        author_type: AuthorType::Human,
        author_id: "daniel".to_string(),
        author_model: None,
        body: "note".to_string(),
    });
    assert!(matches!(workflow, Err(OrbitError::EntryValidation(_))));
}

#[test]
fn successful_append_emits_entry_created_audit_and_event() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");
    let task_id = add_task(&runtime, "entry");

    let first = runtime
        .add_entry(EntryAddParams {
            entity_type: EntityType::Task,
            entity_id: task_id.clone(),
            session_id: None,
            entry_type: EntryType::Comment,
            author_type: AuthorType::Human,
            author_id: "daniel".to_string(),
            author_model: None,
            body: "first".to_string(),
        })
        .expect("entry");
    let second = runtime
        .add_entry(EntryAddParams {
            entity_type: EntityType::Task,
            entity_id: task_id.clone(),
            session_id: None,
            entry_type: EntryType::Decision,
            author_type: AuthorType::Human,
            author_id: "daniel".to_string(),
            author_model: None,
            body: "second".to_string(),
        })
        .expect("entry");

    assert_eq!(first.sequence_number, 1);
    assert_eq!(second.sequence_number, 2);

    let listed = runtime
        .list_entries(EntityType::Task, &task_id)
        .expect("list entries");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].body, "first");
    assert_eq!(listed[1].body, "second");

    let events = runtime.event_bus.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            event,
            OrbitEvent::EntryCreated {
                entity_type,
                entity_id,
                ..
            } if entity_type == "task" && entity_id == &task_id
        )
    }));

    let audits = runtime.list_audits(50).expect("audits");
    assert!(
        audits
            .iter()
            .any(|audit| audit.event_type == "EntryCreated"),
        "EntryCreated should be audited"
    );
}
