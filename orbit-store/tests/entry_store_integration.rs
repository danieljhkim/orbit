use orbit_store::Store;
use orbit_store::task_store::TaskInsertParams;
use orbit_types::{AuthorType, EntityType, EntryType};

fn create_task(store: &Store, title: &str) -> String {
    store
        .with_transaction(|tx| {
            tx.insert_task(&TaskInsertParams {
                title: title.to_string(),
                ..Default::default()
            })
        })
        .expect("insert task")
        .id
}

#[test]
fn per_entity_sequence_is_monotonic_and_independent() {
    let store = Store::open_in_memory().expect("store");
    let task_a = create_task(&store, "a");
    let task_b = create_task(&store, "b");

    let (entry_a1, entry_a2, entry_b1) = store
        .with_transaction(|tx| {
            let a1 = tx.append_entry(
                EntityType::Task,
                &task_a,
                None,
                EntryType::Comment,
                AuthorType::Human,
                "daniel",
                None,
                "first",
            )?;
            let a2 = tx.append_entry(
                EntityType::Task,
                &task_a,
                None,
                EntryType::Comment,
                AuthorType::Human,
                "daniel",
                None,
                "second",
            )?;
            let b1 = tx.append_entry(
                EntityType::Task,
                &task_b,
                None,
                EntryType::Comment,
                AuthorType::Human,
                "daniel",
                None,
                "other-task",
            )?;
            Ok((a1, a2, b1))
        })
        .expect("append");

    assert_eq!(entry_a1.sequence_number, 1);
    assert_eq!(entry_a2.sequence_number, 2);
    assert_eq!(entry_b1.sequence_number, 1);
}

#[test]
fn list_entries_is_deterministic_by_sequence_number() {
    let store = Store::open_in_memory().expect("store");
    let task_id = create_task(&store, "ordered");

    store
        .with_transaction(|tx| {
            tx.append_entry(
                EntityType::Task,
                &task_id,
                None,
                EntryType::Comment,
                AuthorType::Human,
                "daniel",
                None,
                "alpha",
            )?;
            tx.append_entry(
                EntityType::Task,
                &task_id,
                None,
                EntryType::Decision,
                AuthorType::Human,
                "daniel",
                None,
                "beta",
            )?;
            tx.append_entry(
                EntityType::Task,
                &task_id,
                None,
                EntryType::Artifact,
                AuthorType::Human,
                "daniel",
                None,
                "gamma",
            )?;
            Ok(())
        })
        .expect("append");

    let entries = store
        .list_entries(EntityType::Task, &task_id)
        .expect("list entries");
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].sequence_number, 1);
    assert_eq!(entries[1].sequence_number, 2);
    assert_eq!(entries[2].sequence_number, 3);
    assert_eq!(entries[0].body, "alpha");
    assert_eq!(entries[1].body, "beta");
    assert_eq!(entries[2].body, "gamma");
}

#[test]
fn list_entries_by_session_returns_cross_entity_records() {
    let store = Store::open_in_memory().expect("store");
    let task_id = create_task(&store, "session");
    let session_id = "session-123";

    store
        .with_transaction(|tx| {
            tx.append_entry(
                EntityType::Task,
                &task_id,
                Some(session_id),
                EntryType::Comment,
                AuthorType::Agent,
                "orbit-agent",
                Some("planner-v1"),
                "task-note",
            )?;
            tx.append_entry(
                EntityType::Session,
                session_id,
                Some(session_id),
                EntryType::Reasoning,
                AuthorType::Agent,
                "orbit-agent",
                Some("planner-v1"),
                "session-note",
            )?;
            Ok(())
        })
        .expect("append");

    let entries = store
        .list_entries_by_session(session_id)
        .expect("list session entries");
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|entry| entry.body == "task-note"));
    assert!(entries.iter().any(|entry| entry.body == "session-note"));
}
